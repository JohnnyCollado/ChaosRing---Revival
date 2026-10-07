/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! Objective-C runtime.
//!
//! Apple's [Programming with Objective-C](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/ProgrammingWithObjectiveC/Introduction/Introduction.html)
//! is a useful introduction to the language from a user's perspective.
//! There are further resources in the child modules of this module, but they
//! are more implementation-specific.
//!
//! The strategy for this emulator will be to provide our own implementations of
//! an Objective-C runtime and libraries for it (Foundation etc). These
//! implementations will be "host code": Rust code forming part of the emulator,
//! not emulated code. The runtime will need to be able to handle classes that
//! originate from the guest app, classes defined by the host, and sometimes
//! classes that are both (considering Objective-C's support for inheritance,
//! categories and dynamic class editing).

use crate::dyld::{export_c_func, ConstantExports, FunctionExports, HostConstant, HostDylib};
use crate::mem::MutPtr;
use crate::objc::messages::ThreadInitializer;
use crate::MutexId;
use std::collections::HashMap;

mod classes;
mod messages;
mod methods;
mod objects;
mod properties;
mod selectors;
mod synchronization;

pub use classes::{objc_classes, Class, ClassExports, ClassTemplate};
pub use messages::{
    autorelease, msg, msg_class, msg_send, msg_send_no_type_checking, msg_send_super2, msg_super,
    objc_super, release, retain,
};
pub use methods::{HostIMP, IMP};
pub use objects::{
    id, impl_HostObject_with_superclass, nil, AnyHostObject, HostObject, TrivialHostObject,
};
pub use properties::todo_objc_setter;
pub use selectors::{selector, SEL};

use crate::mem::ConstVoidPtr;
use crate::Environment;
use classes::{
    class_getInstanceSize, class_getSuperclass, objc_getClass, ClassHostObject, FakeClass,
    UnimplementedClass,
};
pub(crate) use messages::objc_msgSend;
use messages::{objc_msgSendSuper2, objc_msgSend_stret, MsgSendSignature, MsgSendSuperSignature};
use methods::method_list_t;
use objects::{objc_object, object_getClass, HostObjectEntry};
use properties::{ivar_list_t, objc_copyStruct, objc_getProperty, objc_setProperty};
use selectors::sel_registerName;
use synchronization::{objc_sync_enter, objc_sync_exit};

/// Typedef for `NSZone *`. This is a [fossil type] found in the signature of
/// `allocWithZone:` and similar methods. Its value is always ignored.
///
/// [fossil type]: https://en.wiktionary.org/wiki/fossil_word
pub type NSZonePtr = crate::mem::MutVoidPtr;

/// Main type holding Objective-C runtime state.
pub struct ObjC {
    /// Known selectors (interned method name strings).
    selectors: HashMap<String, SEL>,

    /// Mapping of known (guest) object pointers to their host objects.
    ///
    /// If an object isn't in this map, we will consider it not to exist.
    objects: HashMap<id, HostObjectEntry>,

    /// Known classes.
    ///
    /// Look at the `isa` to get the metaclass for a class.
    classes: HashMap<String, Class>,

    /// Mutexes used in @synchronized blocks (objc_sync_enter/exit).
    sync_mutexes: HashMap<id, MutexId>,

    /// Mutexes for running the +initialize function.
    initializer_threads: HashMap<id, ThreadInitializer>,

    /// Temporary storage for optional type information when sending a message.
    /// Type information isn't part of the `objc_msgSend` ABI, so an alternative
    /// channel is needed.
    message_type_info: Option<(std::any::TypeId, &'static str)>,
}

impl ObjC {
    pub fn new() -> ObjC {
        ObjC {
            selectors: HashMap::new(),
            objects: HashMap::new(),
            classes: HashMap::new(),
            sync_mutexes: HashMap::new(),
            initializer_threads: HashMap::new(),
            message_type_info: None,
        }
    }
}

pub const DYLIB: HostDylib = HostDylib {
    path: "/usr/lib/libobjc.A.dylib",
    aliases: &["/usr/lib/libobjc.dylib"],
    class_exports: &[],
    constant_exports: &[CONSTANTS],
    function_exports: &[FUNCTIONS],
};

const CONSTANTS: ConstantExports = &[
    // We don't use these in our Objective-C runtime, but exporting useless
    // symbols for these silences the warning about the unhandled relocation,
    // and avoids a linker error for the integration tests.
    ("__objc_empty_vtable", HostConstant::NullPtr),
    ("__objc_empty_cache", HostConstant::NullPtr),
    // Clang block "isa" sentinels. Real iOS supplies these from libSystem.
    // The block runtime uses Block_layout->flags to determine block type, so
    // their addresses just need to be distinct, non-null markers.
    (
        "__NSConcreteGlobalBlock",
        HostConstant::Custom(|env| {
            env.mem
                .alloc_and_write::<u32>(0xC0DE0001)
                .cast()
                .cast_const()
        }),
    ),
    (
        "__NSConcreteStackBlock",
        HostConstant::Custom(|env| {
            env.mem
                .alloc_and_write::<u32>(0xC0DE0002)
                .cast()
                .cast_const()
        }),
    ),
    (
        "__NSConcreteMallocBlock",
        HostConstant::Custom(|env| {
            env.mem
                .alloc_and_write::<u32>(0xC0DE0003)
                .cast()
                .cast_const()
        }),
    ),
];

/// Block support is iOS 4+, but it seems like Block Runtime Helpers
/// could still be called on even if minimal iOS version is set to 3.x?
///
/// ref. <https://clang.llvm.org/docs/Block-ABI-Apple.html#runtime-helper-functions>
fn _Block_object_dispose(_env: &mut Environment, object: ConstVoidPtr, flags: i32) {
    // `BLOCK_FIELD_IS_BYREF` flag defines an on stack structure holding
    // the __block variable. It is _probably_ safe to ignore.
    // TODO: properly implement for block support
    assert!(flags == 8); // BLOCK_FIELD_IS_BYREF
    log!(
        "Warning: Ignoring _Block_object_dispose({:?}, BLOCK_FIELD_IS_BYREF)",
        object
    );
}

// === ARC (Automatic Reference Counting) runtime ===
//
// ARC was introduced with iOS 4.3 / Xcode 4.2. ARC-compiled binaries emit
// direct calls to these runtime functions instead of `[obj retain]` etc.
// Apps targeting iOS 4.x routinely require these symbols even if the source
// code doesn't use ARC, because some system frameworks/static libraries do.
//
// We implement the strong-reference family by delegating to the existing
// `retain`/`release`/`autorelease` helpers (which send the corresponding
// Objective-C messages). The weak-reference family is implemented as a
// degraded no-op variant that simply stores/loads the pointer without
// zeroing-on-dealloc semantics — touchHLE doesn't track weak references,
// but most games don't rely on the auto-zeroing behaviour.

fn objc_retain(env: &mut Environment, obj: id) -> id {
    messages::retain(env, obj);
    obj
}

fn objc_release(env: &mut Environment, obj: id) {
    messages::release(env, obj);
}

fn objc_autorelease(env: &mut Environment, obj: id) -> id {
    messages::autorelease(env, obj);
    obj
}

/// Called at the call-site of a method that returns a retained, autoreleased
/// object. The real runtime uses a TLS hand-shake with
/// `objc_autoreleaseReturnValue` to skip the autorelease pool; doing the
/// straightforward thing (always autorelease) is semantically correct.
fn objc_autoreleaseReturnValue(env: &mut Environment, obj: id) -> id {
    messages::autorelease(env, obj);
    obj
}

/// Inverse fast-path peer of [objc_autoreleaseReturnValue]. The naive correct
/// behaviour is to retain (which balances the autorelease the callee did).
fn objc_retainAutoreleasedReturnValue(env: &mut Environment, obj: id) -> id {
    messages::retain(env, obj);
    obj
}

fn objc_retainAutorelease(env: &mut Environment, obj: id) -> id {
    messages::retain(env, obj);
    messages::autorelease(env, obj);
    obj
}

fn objc_retainAutoreleaseReturnValue(env: &mut Environment, obj: id) -> id {
    messages::retain(env, obj);
    messages::autorelease(env, obj);
    obj
}

/// ARC's release-on-return-from-frame fast path. For correctness it's enough
/// to do nothing (the value lives until the autorelease pool drains).
fn objc_unsafeClaimAutoreleasedReturnValue(_env: &mut Environment, obj: id) -> id {
    obj
}

/// `objc_storeStrong(loc, obj)` is equivalent to:
///     id old = *loc; [obj retain]; *loc = obj; [old release];
fn objc_storeStrong(env: &mut Environment, loc: MutPtr<id>, obj: id) {
    let old: id = if loc.is_null() {
        nil
    } else {
        env.mem.read(loc)
    };
    messages::retain(env, obj);
    if !loc.is_null() {
        env.mem.write(loc, obj);
    }
    messages::release(env, old);
}

/// `objc_retainBlock` is `Block_copy`. We delegate to `_Block_copy` which
/// promotes stack blocks to the heap (so the pointer remains valid after the
/// caller's stack frame goes away) and leaves global / malloc blocks alone.
fn objc_retainBlock(env: &mut Environment, block: id) -> id {
    if block == nil {
        return nil;
    }
    let copied = _Block_copy(env, block.cast_void().cast_const());
    copied.cast::<objc_object>().cast_mut()
}

// --- Weak reference family (degraded; no zeroing) ---------------------------

fn objc_storeWeak(env: &mut Environment, loc: MutPtr<id>, obj: id) -> id {
    if !loc.is_null() {
        env.mem.write(loc, obj);
    }
    obj
}

fn objc_initWeak(env: &mut Environment, loc: MutPtr<id>, obj: id) -> id {
    if !loc.is_null() {
        env.mem.write(loc, obj);
    }
    obj
}

fn objc_destroyWeak(env: &mut Environment, loc: MutPtr<id>) {
    if !loc.is_null() {
        env.mem.write(loc, nil);
    }
}

fn objc_loadWeak(env: &mut Environment, loc: MutPtr<id>) -> id {
    let obj: id = if loc.is_null() {
        nil
    } else {
        env.mem.read(loc)
    };
    messages::autorelease(env, obj);
    obj
}

fn objc_loadWeakRetained(env: &mut Environment, loc: MutPtr<id>) -> id {
    let obj: id = if loc.is_null() {
        nil
    } else {
        env.mem.read(loc)
    };
    messages::retain(env, obj);
    obj
}

fn objc_copyWeak(env: &mut Environment, dst: MutPtr<id>, src: MutPtr<id>) {
    let obj: id = if src.is_null() {
        nil
    } else {
        env.mem.read(src)
    };
    if !dst.is_null() {
        env.mem.write(dst, obj);
    }
}

fn objc_moveWeak(env: &mut Environment, dst: MutPtr<id>, src: MutPtr<id>) {
    let obj: id = if src.is_null() {
        nil
    } else {
        env.mem.read(src)
    };
    if !dst.is_null() {
        env.mem.write(dst, obj);
    }
    if !src.is_null() {
        env.mem.write(src, nil);
    }
}

/// Stub for `dyld_stub_binder`. touchHLE handles lazy binding via SVCs, so
/// this should never actually be invoked at runtime; we provide it because
/// the binary's non-lazy pointer table references it.
fn dyld_stub_binder(_env: &mut Environment) {
    panic!("dyld_stub_binder was invoked; touchHLE should be handling lazy binding via SVC");
}

// =============================================================================
// Clang Block ABI runtime
// =============================================================================
//
// Blocks have this in-memory layout (32-bit):
//
//   struct Block_layout {
//       void *isa;                 // 0: _NSConcrete{Global,Stack,Malloc}Block
//       int flags;                 // 4
//       int reserved;              // 8
//       void (*invoke)(void *, ..);// 12
//       Block_descriptor *desc;    // 16
//       /* captured vars follow */
//   };
//
//   struct Block_descriptor_1 {
//       unsigned long reserved;    // 0
//       unsigned long Block_size;  // 4
//       // if BLOCK_HAS_COPY_DISPOSE:
//       //   void (*copy_helper)(void *dst, void *src);   // 8
//       //   void (*dispose_helper)(void *src);            // 12
//       // if BLOCK_HAS_SIGNATURE:
//       //   const char *signature;
//   };
//
// References:
// - https://clang.llvm.org/docs/Block-ABI-Apple.html
// - Apple's libclosure source.

#[allow(dead_code)]
mod block_flags {
    pub const BLOCK_DEALLOCATING: u32 = 1;
    pub const BLOCK_REFCOUNT_MASK: u32 = 0xfffe;
    pub const BLOCK_NEEDS_FREE: u32 = 1 << 24;
    pub const BLOCK_HAS_COPY_DISPOSE: u32 = 1 << 25;
    pub const BLOCK_HAS_CTOR: u32 = 1 << 26;
    pub const BLOCK_IS_GLOBAL: u32 = 1 << 28;
    pub const BLOCK_HAS_SIGNATURE: u32 = 1 << 30;
}

const BLOCK_OFFSET_FLAGS: u32 = 4;
const BLOCK_OFFSET_INVOKE: u32 = 12;
const BLOCK_OFFSET_DESCRIPTOR: u32 = 16;
const DESCRIPTOR_OFFSET_BLOCK_SIZE: u32 = 4;
const DESCRIPTOR_OFFSET_COPY_HELPER: u32 = 8;

/// Read a u32 at `addr` from guest memory.
fn read_u32(env: &mut Environment, addr: u32) -> u32 {
    env.mem.read(crate::mem::ConstPtr::<u32>::from_bits(addr))
}

/// Write a u32 to `addr` in guest memory.
fn write_u32(env: &mut Environment, addr: u32, value: u32) {
    env.mem
        .write(crate::mem::MutPtr::<u32>::from_bits(addr), value)
}

/// `Block_copy`. For a stack block, copies it to the heap and runs the copy
/// helper. For a global block, returns it unchanged. For a malloc block we
/// just return the same pointer (no refcount tracking — this leaks if the
/// guest expects multiple copies to be independent, but works correctly).
fn _Block_copy(env: &mut Environment, block: ConstVoidPtr) -> ConstVoidPtr {
    if block.is_null() {
        return block;
    }
    let block_addr = block.to_bits();
    let flags = read_u32(env, block_addr + BLOCK_OFFSET_FLAGS);

    if flags & block_flags::BLOCK_IS_GLOBAL != 0 {
        return block;
    }
    if flags & block_flags::BLOCK_NEEDS_FREE != 0 {
        // Already on the heap. A real implementation would bump the refcount;
        // we just return the same pointer.
        return block;
    }

    // Stack block: promote to heap.
    let descriptor_addr = read_u32(env, block_addr + BLOCK_OFFSET_DESCRIPTOR);
    let block_size = read_u32(env, descriptor_addr + DESCRIPTOR_OFFSET_BLOCK_SIZE);

    let new_block_addr = env.mem.alloc(block_size).to_bits();
    {
        let src = env
            .mem
            .bytes_at(
                crate::mem::ConstPtr::<u8>::from_bits(block_addr),
                block_size,
            )
            .to_vec();
        env.mem
            .bytes_at_mut(
                crate::mem::MutPtr::<u8>::from_bits(new_block_addr),
                block_size,
            )
            .copy_from_slice(&src);
    }

    // Update flags: mark as heap-allocated with a refcount of 1, drop the
    // BLOCK_IS_GLOBAL bit (it shouldn't be set, but be defensive).
    let new_flags = (flags & !block_flags::BLOCK_IS_GLOBAL) | block_flags::BLOCK_NEEDS_FREE | 2;
    write_u32(env, new_block_addr + BLOCK_OFFSET_FLAGS, new_flags);

    if flags & block_flags::BLOCK_HAS_COPY_DISPOSE != 0 {
        let copy_helper_addr = read_u32(env, descriptor_addr + DESCRIPTOR_OFFSET_COPY_HELPER);
        if copy_helper_addr != 0 {
            let helper = crate::abi::GuestFunction::from_addr_with_thumb_bit(copy_helper_addr);
            // copy_helper(void *dst, const void *src)
            use crate::abi::CallFromHost;
            let _: () = helper.call_from_host(
                env,
                (
                    crate::mem::MutVoidPtr::from_bits(new_block_addr),
                    crate::mem::MutVoidPtr::from_bits(block_addr),
                ),
            );
        }
    }

    crate::mem::ConstVoidPtr::from_bits(new_block_addr)
}

/// `Block_release`. For now, we just leak — releases are no-ops. This is
/// safe but wastes memory.
fn _Block_release(_env: &mut Environment, _block: ConstVoidPtr) {
    // TODO: implement refcounting + dispose helper invocation.
}

/// Invoke a block that takes no arguments. Reads the invoke fn pointer
/// from the block's layout and calls it with the block itself as r0.
pub fn invoke_block_no_args(env: &mut Environment, block: id) {
    if block == nil {
        return;
    }
    let block_addr = block.to_bits();
    let invoke_addr = read_u32(env, block_addr + BLOCK_OFFSET_INVOKE);
    if invoke_addr == 0 {
        log!(
            "Warning: invoke_block_no_args: block {:#x} has null invoke fn; skipping",
            block_addr
        );
        return;
    }
    let invoke_fn = crate::abi::GuestFunction::from_addr_with_thumb_bit(invoke_addr);
    use crate::abi::CallFromHost;
    let _: () = invoke_fn.call_from_host(env, (block,));
}

const FUNCTIONS: FunctionExports = &[
    export_c_func!(class_getInstanceSize(_)),
    export_c_func!(class_getSuperclass(_)),
    export_c_func!(objc_msgSend(_, _)),
    export_c_func!(objc_msgSend_stret(_, _, _)),
    export_c_func!(objc_msgSendSuper2(_, _)),
    export_c_func!(objc_getClass(_)),
    export_c_func!(objc_getProperty(_, _, _, _)),
    export_c_func!(objc_setProperty(_, _, _, _, _, _)),
    export_c_func!(objc_copyStruct(_, _, _, _, _)),
    export_c_func!(objc_sync_enter(_)),
    export_c_func!(objc_sync_exit(_)),
    export_c_func!(object_getClass(_)),
    export_c_func!(sel_registerName(_)),
    export_c_func!(_Block_object_dispose(_, _)),
    // ARC runtime
    export_c_func!(objc_retain(_)),
    export_c_func!(objc_release(_)),
    export_c_func!(objc_autorelease(_)),
    export_c_func!(objc_autoreleaseReturnValue(_)),
    export_c_func!(objc_retainAutoreleasedReturnValue(_)),
    export_c_func!(objc_retainAutorelease(_)),
    export_c_func!(objc_retainAutoreleaseReturnValue(_)),
    export_c_func!(objc_unsafeClaimAutoreleasedReturnValue(_)),
    export_c_func!(objc_storeStrong(_, _)),
    export_c_func!(objc_retainBlock(_)),
    export_c_func!(objc_storeWeak(_, _)),
    export_c_func!(objc_initWeak(_, _)),
    export_c_func!(objc_destroyWeak(_)),
    export_c_func!(objc_loadWeak(_)),
    export_c_func!(objc_loadWeakRetained(_)),
    export_c_func!(objc_copyWeak(_, _)),
    export_c_func!(objc_moveWeak(_, _)),
    // dyld lazy-binder stub (should never be invoked; SVC linker handles it)
    export_c_func!(dyld_stub_binder()),
    // Block runtime
    export_c_func!(_Block_copy(_)),
    export_c_func!(_Block_release(_)),
];
