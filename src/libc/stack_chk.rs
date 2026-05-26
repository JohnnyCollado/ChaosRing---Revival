/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Stack-protector (`-fstack-protector`) ABI.
//!
//! Compilers emit code at function entry that reads `__stack_chk_guard`
//! (a process-global 32-bit canary on 32-bit ARM), stores it on the stack
//! next to saved registers, and re-reads/compares it at function exit.
//! On mismatch the prologue calls `__stack_chk_fail`, which on a real
//! system aborts the process.
//!
//! Apps targeting iOS 4.x routinely require these symbols even when the
//! source code doesn't enable the stack protector explicitly, because
//! Apple's clang turned it on by default for some optimization levels.

use crate::dyld::{export_c_func, ConstantExports, FunctionExports, HostConstant};
use crate::mem::ConstVoidPtr;
use crate::Environment;

/// A fixed (non-random) canary. Apps don't see the value — only the address
/// of the variable matters — but the value itself is what gets copied onto
/// the stack and compared against. Any non-zero value works for correctness;
/// the hardcoded magic just makes it easy to spot in dumps.
const STACK_CHK_GUARD_VALUE: u32 = 0xC0DEFACE;

pub const CONSTANTS: ConstantExports = &[(
    "___stack_chk_guard",
    HostConstant::Custom(|env| -> ConstVoidPtr {
        env.mem
            .alloc_and_write(STACK_CHK_GUARD_VALUE)
            .cast()
            .cast_const()
    }),
)];

fn __stack_chk_fail(_env: &mut Environment) {
    panic!("__stack_chk_fail was called: stack canary check failed (guest binary detected stack corruption)");
}

pub const FUNCTIONS: FunctionExports = &[export_c_func!(__stack_chk_fail())];
