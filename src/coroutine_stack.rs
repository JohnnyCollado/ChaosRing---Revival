/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Pre-committed coroutine stack for corosensei.
//!
//! corosensei's stock `DefaultStack` reserves the requested size with
//! `MEM_RESERVE` but only `MEM_COMMIT`s the top page (4 KB). The rest is
//! grown on demand via guard pages and `_chkstk`. That mechanism works for
//! normal user code, but it fails catastrophically when **Windows SEH
//! unwinding** runs on the coroutine: the unwinder probes pages well past
//! the initially-committed region, the guard page fault dispatcher itself
//! consumes more stack, and we end up with an access violation in
//! `_C_specific_handler_noexcept` while trying to unwind a C++ exception
//! thrown from native code (dynarmic / SDL / Symphonia / etc.).
//!
//! This module provides a corosensei `Stack` implementation that commits
//! the entire requested size upfront (with a single guard page at the very
//! bottom for overflow detection). Per-page cost is paid at allocation
//! time, not lazily; this is fine for our 1-coroutine-per-guest-pthread
//! model since the number of coroutines is small.
//!
//! Non-Windows builds keep using corosensei's default stack.

#[cfg(windows)]
pub use windows_impl::PreCommittedStack;

#[cfg(not(windows))]
pub type PreCommittedStack = corosensei::stack::DefaultStack;

#[cfg(windows)]
mod windows_impl {
    use corosensei::stack::{Stack, StackPointer, StackTebFields};
    use std::io;
    use windows_sys::Win32::System::Memory::{
        VirtualAlloc, VirtualFree, MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_GUARD, PAGE_READWRITE,
    };
    use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};

    fn page_size() -> usize {
        unsafe {
            let mut info: SYSTEM_INFO = std::mem::zeroed();
            GetSystemInfo(&mut info);
            info.dwPageSize as usize
        }
    }

    /// A coroutine stack with the full reservation pre-committed.
    ///
    /// Layout (low → high addresses):
    /// ```text
    ///   alloc_base
    ///   [ 1 guard page ]                          ← catches overflow
    ///   [ usable stack region (size - guard)  ]
    ///   alloc_top (= base())
    /// ```
    pub struct PreCommittedStack {
        alloc_base: usize,
        alloc_top: usize,
        /// Lowest usable address. Below this is the guard page.
        limit: usize,
    }

    impl PreCommittedStack {
        pub fn new(size: usize) -> io::Result<Self> {
            let page = page_size();
            // Round size up to a page multiple. Always leave one page for
            // the guard at the bottom.
            let aligned = (size + page - 1) & !(page - 1);
            let total = aligned + page; // + 1 page for the guard

            unsafe {
                // Reserve AND commit the entire allocation in one call.
                let alloc_base = VirtualAlloc(
                    std::ptr::null(),
                    total,
                    MEM_RESERVE | MEM_COMMIT,
                    PAGE_READWRITE,
                );
                if alloc_base.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let alloc_base = alloc_base as usize;
                let alloc_top = alloc_base + total;
                let limit = alloc_base + page; // top of the guard page

                // Re-protect the bottom page as a guard page so a stack
                // overflow (writes below `limit`) raises STATUS_GUARD_PAGE.
                let mut old_prot: u32 = 0;
                let ok = windows_sys::Win32::System::Memory::VirtualProtect(
                    alloc_base as *mut _,
                    page,
                    PAGE_READWRITE | PAGE_GUARD,
                    &mut old_prot,
                );
                if ok == 0 {
                    let err = io::Error::last_os_error();
                    // Best-effort cleanup of the reservation we just took.
                    let _ = VirtualFree(alloc_base as *mut _, 0, MEM_RELEASE);
                    return Err(err);
                }

                Ok(Self {
                    alloc_base,
                    alloc_top,
                    limit,
                })
            }
        }
    }

    // SAFETY: `Stack` is unsafe to implement; the contract requires a guard
    // page below `limit()` to catch overflow, that `base()` is properly
    // aligned, and that the memory remains valid for as long as the Stack
    // exists. We set the guard page in `new()`, allocate page-aligned via
    // VirtualAlloc, and only free on Drop.
    unsafe impl Stack for PreCommittedStack {
        fn base(&self) -> StackPointer {
            StackPointer::new(self.alloc_top).unwrap()
        }

        fn limit(&self) -> StackPointer {
            // Stack trait docs: "This limit must include any guard pages
            // in the stack." So we report the absolute base (covering the
            // guard page) here.
            StackPointer::new(self.alloc_base).unwrap()
        }

        fn teb_fields(&self) -> StackTebFields {
            StackTebFields {
                StackBase: self.alloc_top,
                StackLimit: self.limit, // first usable byte
                DeallocationStack: self.alloc_base,
                GuaranteedStackBytes: 0,
            }
        }

        fn update_teb_fields(&mut self, _stack_limit: usize, _guaranteed_stack_bytes: usize) {
            // We don't grow the stack on demand (everything is pre-
            // committed), so updates from the TEB are not meaningful.
        }
    }

    impl Drop for PreCommittedStack {
        fn drop(&mut self) {
            unsafe {
                // VirtualFree with MEM_RELEASE: size must be 0.
                let _ = VirtualFree(self.alloc_base as *mut _, 0, MEM_RELEASE);
            }
        }
    }

    // SAFETY: PreCommittedStack only holds a raw allocation and primitive
    // sizes; nothing inherently thread-bound. A coroutine consuming the
    // stack moves into the Coroutine, which is itself Send.
    unsafe impl Send for PreCommittedStack {}
    unsafe impl Sync for PreCommittedStack {}
}
