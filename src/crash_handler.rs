/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Capture native (non-Rust-panic) crashes on Windows.
//!
//! Hard memory errors from FFI / native libraries (e.g. an `access violation
//! 0xC0000005` raised inside `VCRUNTIME140.dll!memcpy`) skip Rust's panic
//! machinery entirely. Without a special handler, the process is terminated
//! by Windows Error Reporting and we get nothing useful.
//!
//! We register *both* handlers:
//!
//! - a **vectored exception handler** (`AddVectoredExceptionHandler` with
//!   `first=1`), which fires *before* frame-based `__try/__except` handlers
//!   that some C++ code (SDL, dynarmic) might install, and
//! - the **top-level unhandled-exception filter**
//!   (`SetUnhandledExceptionFilter`) as a fallback.
//!
//! Each handler does two things, in order of decreasing fragility:
//!
//! 1. Emit a marker via `OutputDebugStringA` — visible in DebugView / a
//!    debugger's Output window, doesn't touch the file system or the
//!    allocator, so it works even if the crashing thread's heap state is
//!    corrupted.
//! 2. Best-effort write of a textual report to `touchHLE_crash.txt`
//!    (exception code, fault PC, access-violation kind/target, Rust
//!    backtrace).
//!
//! Both return `EXCEPTION_CONTINUE_SEARCH` so Windows Error Reporting still
//! runs after we're done.

pub fn install() {
    #[cfg(windows)]
    windows_impl::install();
}

#[cfg(windows)]
mod windows_impl {
    use std::fmt::Write as _;
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows_sys::Win32::Foundation::EXCEPTION_ACCESS_VIOLATION;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, OutputDebugStringA, SetUnhandledExceptionFilter,
        EXCEPTION_POINTERS,
    };

    /// Re-entrancy guard so the handler itself can't loop forever if writing
    /// the report somehow re-faults.
    static HANDLER_FIRED: AtomicBool = AtomicBool::new(false);

    /// Pass the exception to the next handler in the chain.
    const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

    /// Symbol-resolve `addr` using the `backtrace` crate, which handles
    /// DbgHelp's SymInitialize lifecycle correctly (and won't race with
    /// `std::backtrace::Backtrace`). Returns a one-line description like
    /// `0x7ff6...  module!func+0x1a (file:line)` or a fall-through formatted
    /// hex if nothing was resolved.
    fn resolve_symbol(addr: u64) -> String {
        let mut out = format!("0x{:016x}", addr);
        let mut found = false;
        // SAFETY: backtrace::resolve takes a *mut c_void and dispatches to
        // platform-specific symbol resolution (DbgHelp on Windows). Safe to
        // call from an exception handler context.
        unsafe {
            backtrace::resolve(addr as *mut std::ffi::c_void, |sym| {
                found = true;
                let name = sym
                    .name()
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "<no name>".into());
                out.push_str("  ");
                out.push_str(&name);
                if let Some(file) = sym.filename() {
                    if let Some(line) = sym.lineno() {
                        out.push_str(&format!(" ({}:{})", file.display(), line));
                    } else {
                        out.push_str(&format!(" ({})", file.display()));
                    }
                }
            });
        }
        if !found {
            out.push_str("  <unresolved>");
        }
        out
    }

    /// Heuristic: does this look like an executable address (not data /
    /// stack)? We classify anything in the typical x64 user-mode code
    /// window as a candidate worth symbol-resolving. ASLR places EXEs and
    /// DLLs anywhere in 0x00007FFxxxxxxxxx, so we accept the entire prefix.
    fn looks_like_code_address(v: u64) -> bool {
        let hi = v >> 32;
        // Anything from 0x7FF0_xxxxxxxx through 0x7FFF_xxxxxxxx is plausibly
        // a code address; touchHLE.exe alone has been observed at 0x7FF6...,
        // VCRUNTIME140 at 0x7FF98147..., NTDLL at 0x7FF9bf2.... Include the
        // full 0x7FF0..=0x7FFF range to cover any ASLR slot.
        (0x7FF0..=0x7FFF).contains(&hi)
    }

    /// Send a marker to the Windows debug stream. Visible in DebugView,
    /// Visual Studio's Output window, or any debugger attached. We use this
    /// as a "the handler fired" proof of life that doesn't depend on the
    /// file system or Rust's runtime.
    unsafe fn dbg_print(msg: &str) {
        // Stack-allocated NUL-terminated copy, bounded to 256 bytes.
        let mut buf = [0u8; 256];
        let n = msg.len().min(buf.len() - 1);
        buf[..n].copy_from_slice(&msg.as_bytes()[..n]);
        // buf[n] is already 0
        OutputDebugStringA(buf.as_ptr());
    }

    fn build_report(info: *const EXCEPTION_POINTERS, source: &str) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "=== touchHLE native crash ({}) ===", source);
        unsafe {
            if !info.is_null() {
                let rec_ptr = (*info).ExceptionRecord;
                if !rec_ptr.is_null() {
                    let rec = &*rec_ptr;
                    let _ = writeln!(
                        out,
                        "Exception code: 0x{:08x}",
                        rec.ExceptionCode as u32
                    );
                    let _ = writeln!(out, "Fault PC:       {:p}", rec.ExceptionAddress);
                    let _ = writeln!(
                        out,
                        "Fault symbol:   {}",
                        resolve_symbol(rec.ExceptionAddress as u64)
                    );
                    if rec.ExceptionCode == EXCEPTION_ACCESS_VIOLATION
                        && rec.NumberParameters >= 2
                    {
                        let kind = rec.ExceptionInformation[0];
                        let target = rec.ExceptionInformation[1] as *const u8;
                        let kind_str = match kind {
                            0 => "read",
                            1 => "write",
                            8 => "DEP / execute",
                            _ => "?",
                        };
                        let _ = writeln!(
                            out,
                            "Access violation: {} of {:p} (kind={})",
                            kind_str, target, kind
                        );
                    }
                } else {
                    out.push_str("(ExceptionRecord pointer was null)\n");
                }

                // Dump the CPU context at the moment of the crash. This is
                // what `Backtrace::force_capture` cannot show — by the time
                // it runs we're already on the exception-dispatcher stack,
                // not the crashing one. The saved CONTEXT here is the
                // register state at the faulting instruction.
                let ctx_ptr = (*info).ContextRecord;
                if !ctx_ptr.is_null() {
                    out.push_str("\n=== CPU context at crash ===\n");
                    dump_context(&mut out, ctx_ptr);
                }
            } else {
                out.push_str("(EXCEPTION_POINTERS pointer was null)\n");
            }
        }
        let _ = writeln!(
            out,
            "Crashing thread: {:?}",
            std::thread::current().id()
        );
        // Backtrace last — Backtrace::force_capture may hang or fault on a
        // corrupted stack. Everything above is gathered first so the report
        // is useful even if the backtrace step never returns.
        out.push_str("\n=== Backtrace (handler thread; not the crash site) ===\n");
        let bt = std::backtrace::Backtrace::force_capture();
        let _ = writeln!(out, "{}", bt);
        out.push_str("=== end of crash report ===\n");
        out
    }

    /// Dump x86_64 register state from the saved CONTEXT, plus a slice of
    /// stack memory near RSP. With this and the touchHLE.pdb, a developer
    /// can identify the crashing call site (e.g. by looking up RIP in the
    /// PDB, or feeding the stack slice + the return-address pattern to a
    /// disassembler).
    unsafe fn dump_context(
        out: &mut String,
        ctx_ptr: *const windows_sys::Win32::System::Diagnostics::Debug::CONTEXT,
    ) {
        let ctx = &*ctx_ptr;
        let _ = writeln!(out, "RIP = 0x{:016x}", ctx.Rip);
        let _ = writeln!(out, "RSP = 0x{:016x}", ctx.Rsp);
        let _ = writeln!(out, "RBP = 0x{:016x}", ctx.Rbp);
        let _ = writeln!(out, "RAX = 0x{:016x}  RBX = 0x{:016x}", ctx.Rax, ctx.Rbx);
        let _ = writeln!(out, "RCX = 0x{:016x}  RDX = 0x{:016x}", ctx.Rcx, ctx.Rdx);
        let _ = writeln!(out, "RSI = 0x{:016x}  RDI = 0x{:016x}", ctx.Rsi, ctx.Rdi);
        let _ = writeln!(out, "R8  = 0x{:016x}  R9  = 0x{:016x}", ctx.R8, ctx.R9);
        let _ = writeln!(out, "R10 = 0x{:016x}  R11 = 0x{:016x}", ctx.R10, ctx.R11);
        let _ = writeln!(out, "R12 = 0x{:016x}  R13 = 0x{:016x}", ctx.R12, ctx.R13);
        let _ = writeln!(out, "R14 = 0x{:016x}  R15 = 0x{:016x}", ctx.R14, ctx.R15);

        // Read up to 64 words (512 bytes) of stack above RSP. Each 64-bit
        // word that looks like a code-segment address gets symbol-resolved
        // — these are likely return addresses pointing back into the call
        // chain that reached the crash.
        out.push_str("\n=== Stack near RSP (up to 64 qwords, code addrs symbolized) ===\n");
        let rsp = ctx.Rsp as *const u64;
        for i in 0..64 {
            let addr = rsp.wrapping_add(i);
            let val = std::ptr::read_volatile(addr);
            if looks_like_code_address(val) {
                let _ = writeln!(
                    out,
                    "  [RSP+{:#04x}] = {}",
                    i * 8,
                    resolve_symbol(val)
                );
            } else {
                let _ = writeln!(out, "  [RSP+{:#04x}] = 0x{:016x}", i * 8, val);
            }
        }
    }

    fn write_crash_report(info: *const EXCEPTION_POINTERS, source: &str) {
        // 1. Unambiguous proof-of-life via OutputDebugStringA. This always
        //    works — doesn't touch heap or fs.
        unsafe {
            let mut marker = [0u8; 96];
            let prefix = b"[touchHLE crash_handler] fired: ";
            let n = prefix.len().min(marker.len() - 1);
            marker[..n].copy_from_slice(&prefix[..n]);
            let m = source.len().min(marker.len() - 1 - n);
            marker[n..n + m].copy_from_slice(&source.as_bytes()[..m]);
            OutputDebugStringA(marker.as_ptr());
            dbg_print("[touchHLE crash_handler] fired\0");
        }

        // 2. Best-effort report file. catch_unwind so a panic here can't
        //    re-enter the handler.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let report = build_report(info, source);
            let _ = std::fs::write("touchHLE_crash.txt", &report);
            eprintln!("{}", report);
        }));
    }

    /// Top-level filter (fallback).
    unsafe extern "system" fn unhandled_filter(info: *const EXCEPTION_POINTERS) -> i32 {
        if HANDLER_FIRED.swap(true, Ordering::SeqCst) {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        write_crash_report(info, "SetUnhandledExceptionFilter");
        EXCEPTION_CONTINUE_SEARCH
    }

    /// Vectored handler (first-chance). Filters to real hardware exceptions
    /// only so we don't interfere with language-level SEH used by C++ or
    /// Rust internals.
    unsafe extern "system" fn vectored_handler(info: *mut EXCEPTION_POINTERS) -> i32 {
        if !info.is_null() {
            let rec_ptr = (*info).ExceptionRecord;
            if !rec_ptr.is_null() {
                let code = (*rec_ptr).ExceptionCode;
                let is_hardware_fault = code == EXCEPTION_ACCESS_VIOLATION
                    || code == windows_sys::Win32::Foundation::EXCEPTION_ILLEGAL_INSTRUCTION
                    || code == windows_sys::Win32::Foundation::EXCEPTION_STACK_OVERFLOW;
                if is_hardware_fault && !HANDLER_FIRED.swap(true, Ordering::SeqCst) {
                    write_crash_report(info as *const _, "VectoredExceptionHandler");
                }
            }
        }
        EXCEPTION_CONTINUE_SEARCH
    }

    pub fn install() {
        unsafe {
            // first=1 inserts at front of chain — runs before any
            // __try/__except blocks in dependency C++ code (SDL, dynarmic).
            AddVectoredExceptionHandler(1, Some(vectored_handler));
            SetUnhandledExceptionFilter(Some(unhandled_filter));
        }
        echo!(
            "touchHLE::crash_handler: vectored + unhandled-exception handlers \
             installed; crashes will be reported to touchHLE_crash.txt"
        );
    }
}
