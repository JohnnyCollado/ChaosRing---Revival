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
//! This module installs a `SetUnhandledExceptionFilter` that, on the first
//! unhandled exception, writes:
//!
//! - the exception code and faulting instruction address,
//! - for access violations, the access kind (read/write/execute) and target
//!   address,
//! - the Rust backtrace of the faulting thread,
//!
//! to `touchHLE_crash.txt` and stderr. Then it lets the process terminate
//! normally so Windows Error Reporting still kicks in.

#[cfg(windows)]
pub fn install() {
    windows_impl::install();
}

#[cfg(not(windows))]
pub fn install() {}

#[cfg(windows)]
mod windows_impl {
    use std::fmt::Write as _;
    use std::sync::atomic::{AtomicBool, Ordering};
    use windows_sys::Win32::Foundation::EXCEPTION_ACCESS_VIOLATION;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        SetUnhandledExceptionFilter, EXCEPTION_POINTERS,
    };

    /// Re-entrancy guard so the handler itself can't loop forever if writing
    /// the report somehow faults too.
    static HANDLER_FIRED: AtomicBool = AtomicBool::new(false);

    /// Return value telling Windows to proceed with default handling
    /// (terminate the process / show WER dialog). Letting WER take over
    /// preserves the existing crash-reporting flow while still giving us a
    /// chance to write our own report first.
    const EXCEPTION_CONTINUE_SEARCH: i32 = 0;

    unsafe extern "system" fn handler(info: *const EXCEPTION_POINTERS) -> i32 {
        if HANDLER_FIRED.swap(true, Ordering::SeqCst) {
            return EXCEPTION_CONTINUE_SEARCH;
        }

        let mut out = String::new();
        let _ = writeln!(out, "=== touchHLE native crash ===");

        if info.is_null() || (*info).ExceptionRecord.is_null() {
            let _ = writeln!(out, "(no exception record available)");
        } else {
            let rec = &*(*info).ExceptionRecord;
            let _ = writeln!(out, "Exception code: 0x{:08x}", rec.ExceptionCode as u32);
            let _ = writeln!(out, "Fault PC:       {:p}", rec.ExceptionAddress);
            if rec.ExceptionCode == EXCEPTION_ACCESS_VIOLATION && rec.NumberParameters >= 2 {
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
        }

        let _ = writeln!(out, "Crashing thread: {:?}", std::thread::current().id());
        let _ = writeln!(out);

        // `force_capture` ignores RUST_BACKTRACE and unconditionally captures.
        // On Windows release builds, frame resolution depends on DbgHelp +
        // PDBs; Cargo's release profile emits PDBs by default for the bin
        // target, so symbols should be present.
        let bt = std::backtrace::Backtrace::force_capture();
        let _ = writeln!(out, "=== Backtrace (crashing thread) ===");
        let _ = writeln!(out, "{}", bt);

        // Write to file and stderr. Use no_panic variants since we're inside
        // an exception handler — a panicking write here would be very bad.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = std::fs::write("touchHLE_crash.txt", out.as_bytes());
            eprintln!("{}", out);
        }));

        // Let the OS continue searching for handlers (i.e. WER terminates
        // the process). This preserves the existing crash-reporting flow.
        EXCEPTION_CONTINUE_SEARCH
    }

    pub fn install() {
        // SAFETY: SetUnhandledExceptionFilter takes a nullable callback and
        // returns the previous filter (which we discard). Safe to call from
        // any thread at any time; the callback signature matches.
        unsafe {
            SetUnhandledExceptionFilter(Some(handler));
        }
    }
}
