/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSOperation`, `NSBlockOperation`, `NSInvocationOperation`, and
//! `NSOperationQueue`.
//!
//! **Semantics differ from the real classes**: the queue is synchronous.
//! Operations added via `addOperation:` are `start`ed immediately on the
//! calling thread and run to completion before `addOperation:` returns. This
//! technically violates the contract (which promises concurrency), but is
//! sufficient for games that use the queue as an "eventually do this"
//! mechanism for asset loading or post-init work.

use crate::dyld::HostFunction;
use crate::frameworks::foundation::NSUInteger;
use crate::libc::pthread::thread::{
    pthread_attr_init, pthread_attr_setdetachstate, pthread_attr_t, pthread_create, pthread_t,
    PTHREAD_CREATE_DETACHED,
};
use crate::mem::{guest_size_of, MutPtr, MutVoidPtr};
use crate::objc::{
    autorelease, id, msg, msg_class, nil, objc_classes, release, retain, ClassExports, HostObject,
    NSZonePtr, SEL,
};
use crate::Environment;

#[derive(Default)]
pub struct State {
    main_queue: Option<id>,
}

// ---- Host objects -----------------------------------------------------------

struct NSOperationHostObject {
    target: id,
    selector: Option<SEL>,
    argument: id,
    completion_block: id,
    cancelled: bool,
    executing: bool,
    finished: bool,
}
impl HostObject for NSOperationHostObject {}

struct NSBlockOperationHostObject {
    base: NSOperationHostObject,
    blocks: Vec<id>,
}
impl HostObject for NSBlockOperationHostObject {}

struct NSInvocationOperationHostObject {
    base: NSOperationHostObject,
    invocation: id,
}
impl HostObject for NSInvocationOperationHostObject {}

struct NSOperationQueueHostObject {
    name: id,
    max_concurrent: i32,
    suspended: bool,
}
impl HostObject for NSOperationQueueHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

// =====================================================================
// NSOperation
// =====================================================================
@implementation NSOperation: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host = Box::new(NSOperationHostObject {
        target: nil,
        selector: None,
        argument: nil,
        completion_block: nil,
        cancelled: false,
        executing: false,
        finished: false,
    });
    env.objc.alloc_object(this, host, &mut env.mem)
}

- (id)init {
    this
}

- (())start {
    {
        let host = env.objc.borrow::<NSOperationHostObject>(this);
        if host.cancelled || host.finished {
            return;
        }
    }
    env.objc.borrow_mut::<NSOperationHostObject>(this).executing = true;
    () = msg![env; this main];
    {
        let host = env.objc.borrow_mut::<NSOperationHostObject>(this);
        host.executing = false;
        host.finished = true;
    }
    let completion = env.objc.borrow::<NSOperationHostObject>(this).completion_block;
    if completion != nil {
        log!("Warning: NSOperation completionBlock cannot be invoked (no block runtime)");
    }
}

- (())main {
    // Default: invoke target/selector if set.
    let (target, selector, argument) = {
        let host = env.objc.borrow::<NSOperationHostObject>(this);
        (host.target, host.selector, host.argument)
    };
    if let Some(sel) = selector {
        if argument != nil {
            let _: id = crate::objc::msg_send(env, (target, sel, argument));
        } else {
            let _: id = crate::objc::msg_send(env, (target, sel));
        }
    }
}

- (())cancel {
    env.objc.borrow_mut::<NSOperationHostObject>(this).cancelled = true;
}

- (bool)isCancelled {
    env.objc.borrow::<NSOperationHostObject>(this).cancelled
}

- (bool)isExecuting {
    env.objc.borrow::<NSOperationHostObject>(this).executing
}

- (bool)isFinished {
    env.objc.borrow::<NSOperationHostObject>(this).finished
}

- (bool)isReady {
    true
}

- (bool)isConcurrent {
    false
}

- (bool)isAsynchronous {
    false
}

- (())waitUntilFinished {
    // Poll isFinished, yielding via host sleep on each iteration.
    // Operations now run on real pthreads, so the main thread sleeping
    // here doesn't block the worker.
    loop {
        if env.objc.borrow::<NSOperationHostObject>(this).finished {
            return;
        }
        env.sleep(std::time::Duration::from_millis(1));
    }
}

- (())addDependency:(id)_other_op {
    // Dependencies are not honoured (operations may run out of submission
    // order). Most guest code that uses dependencies will also call
    // waitUntilFinished, which still works.
}

- (())removeDependency:(id)_other_op {
}

- (id)dependencies {
    msg_class![env; NSArray array]
}

- (())setCompletionBlock:(id)block {
    let new = retain(env, block);
    let old = std::mem::replace(
        &mut env.objc.borrow_mut::<NSOperationHostObject>(this).completion_block,
        new,
    );
    release(env, old);
}

- (id)completionBlock {
    env.objc.borrow::<NSOperationHostObject>(this).completion_block
}

- (())setQueuePriority:(NSUInteger)_p {
}

- (NSUInteger)queuePriority {
    0
}

- (())setThreadPriority:(f64)_p {
}

- (f64)threadPriority {
    0.5
}

- (())dealloc {
    let completion = env.objc.borrow::<NSOperationHostObject>(this).completion_block;
    release(env, completion);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

// =====================================================================
// NSBlockOperation
// =====================================================================
@implementation NSBlockOperation: NSOperation

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host = Box::new(NSBlockOperationHostObject {
        base: NSOperationHostObject {
            target: nil,
            selector: None,
            argument: nil,
            completion_block: nil,
            cancelled: false,
            executing: false,
            finished: false,
        },
        blocks: Vec::new(),
    });
    env.objc.alloc_object(this, host, &mut env.mem)
}

+ (id)blockOperationWithBlock:(id)block {
    let op: id = msg![env; this alloc];
    let op: id = msg![env; op init];
    () = msg![env; op addExecutionBlock:block];
    autorelease(env, op)
}

- (())addExecutionBlock:(id)block {
    let retained = retain(env, block);
    env.objc.borrow_mut::<NSBlockOperationHostObject>(this).blocks.push(retained);
}

- (id)executionBlocks {
    let blocks = env.objc.borrow::<NSBlockOperationHostObject>(this).blocks.clone();
    let arr: id = msg_class![env; NSMutableArray array];
    for b in blocks {
        () = msg![env; arr addObject:b];
    }
    arr
}

- (())main {
    // Snapshot the list so we don't hold a borrow on env.objc across calls.
    let blocks: Vec<id> = env.objc.borrow::<NSBlockOperationHostObject>(this).blocks.clone();
    for b in blocks {
        crate::objc::invoke_block_no_args(env, b);
    }
}

- (())dealloc {
    let blocks = std::mem::take(
        &mut env.objc.borrow_mut::<NSBlockOperationHostObject>(this).blocks,
    );
    for b in blocks {
        release(env, b);
    }
    let completion = env.objc.borrow::<NSBlockOperationHostObject>(this).base.completion_block;
    release(env, completion);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

// =====================================================================
// NSInvocationOperation
// =====================================================================
@implementation NSInvocationOperation: NSOperation

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host = Box::new(NSInvocationOperationHostObject {
        base: NSOperationHostObject {
            target: nil,
            selector: None,
            argument: nil,
            completion_block: nil,
            cancelled: false,
            executing: false,
            finished: false,
        },
        invocation: nil,
    });
    env.objc.alloc_object(this, host, &mut env.mem)
}

- (id)initWithTarget:(id)target selector:(SEL)sel object:(id)arg {
    let target_r = retain(env, target);
    let arg_r = retain(env, arg);
    let host = env.objc.borrow_mut::<NSInvocationOperationHostObject>(this);
    host.base.target = target_r;
    host.base.selector = Some(sel);
    host.base.argument = arg_r;
    this
}

- (id)initWithInvocation:(id)inv {
    let retained = retain(env, inv);
    env.objc.borrow_mut::<NSInvocationOperationHostObject>(this).invocation = retained;
    this
}

- (id)invocation {
    env.objc.borrow::<NSInvocationOperationHostObject>(this).invocation
}

- (())main {
    let inv = env.objc.borrow::<NSInvocationOperationHostObject>(this).invocation;
    if inv != nil {
        () = msg![env; inv invoke];
        return;
    }
    let (target, selector, argument) = {
        let host = env.objc.borrow::<NSInvocationOperationHostObject>(this);
        (host.base.target, host.base.selector, host.base.argument)
    };
    if let Some(sel) = selector {
        if argument != nil {
            let _: id = crate::objc::msg_send(env, (target, sel, argument));
        } else {
            let _: id = crate::objc::msg_send(env, (target, sel));
        }
    }
}

- (())dealloc {
    let (inv, target, argument, completion) = {
        let host = env.objc.borrow::<NSInvocationOperationHostObject>(this);
        (
            host.invocation,
            host.base.target,
            host.base.argument,
            host.base.completion_block,
        )
    };
    release(env, inv);
    release(env, target);
    release(env, argument);
    release(env, completion);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

// =====================================================================
// NSOperationQueue
// =====================================================================
@implementation NSOperationQueue: NSObject

+ (id)allocWithZone:(NSZonePtr)_zone {
    let host = Box::new(NSOperationQueueHostObject {
        name: nil,
        max_concurrent: -1,
        suspended: false,
    });
    env.objc.alloc_object(this, host, &mut env.mem)
}

+ (id)mainQueue {
    if let Some(q) = env.framework_state.foundation.ns_operation.main_queue {
        return q;
    }
    let q: id = msg![env; this alloc];
    let q: id = msg![env; q init];
    // Retain so the singleton stays alive forever.
    let _: id = msg![env; q retain];
    env.framework_state.foundation.ns_operation.main_queue = Some(q);
    q
}

+ (id)currentQueue {
    // We don't track which queue is "current"; return mainQueue as a default.
    msg![env; this mainQueue]
}

- (id)init {
    this
}

- (())addOperation:(id)op {
    if op == nil {
        return;
    }
    // Spawn a real worker pthread that runs [op start]. NSOperationQueue is
    // contractually asynchronous; running synchronously here deadlocks
    // guests whose operations expect to wait on flags set by the main
    // thread (e.g. Chaos Rings' CRoperation).
    retain(env, op);

    let hf: HostFunction = &(_touchHLE_NSOperationInvocationHelper as fn(&mut Environment, _) -> _);
    let gf = env
        .dyld
        .create_guest_function(&mut env.mem, "__touchHLE_NSOperationInvocationHelper", hf);

    let attr: MutPtr<pthread_attr_t> = env.mem.alloc(guest_size_of::<pthread_attr_t>()).cast();
    pthread_attr_init(env, attr);
    pthread_attr_setdetachstate(env, attr, PTHREAD_CREATE_DETACHED);
    let thread_ptr: MutPtr<pthread_t> = env.mem.alloc(guest_size_of::<pthread_t>()).cast();
    pthread_create(env, thread_ptr, attr.cast_const(), gf, op.cast());
}

- (())addOperations:(id)ops waitUntilFinished:(bool)_wait {
    if ops == nil {
        return;
    }
    let count: NSUInteger = msg![env; ops count];
    for i in 0..count {
        let op: id = msg![env; ops objectAtIndex:i];
        () = msg![env; this addOperation:op];
    }
}

- (())addOperationWithBlock:(id)block {
    if block == nil {
        return;
    }
    let op: id = msg_class![env; NSBlockOperation blockOperationWithBlock:block];
    () = msg![env; this addOperation:op];
}

- (NSUInteger)operationCount {
    0
}

- (id)operations {
    msg_class![env; NSArray array]
}

- (())cancelAllOperations {
}

- (())waitUntilAllOperationsAreFinished {
    // We don't track in-flight operations; the best we can do is yield once.
    // Guests that rely on this should be calling waitUntilFinished on
    // individual operations instead.
    env.sleep(std::time::Duration::from_millis(1));
}

- (())setMaxConcurrentOperationCount:(i32)n {
    env.objc.borrow_mut::<NSOperationQueueHostObject>(this).max_concurrent = n;
}

- (i32)maxConcurrentOperationCount {
    env.objc.borrow::<NSOperationQueueHostObject>(this).max_concurrent
}

- (())setSuspended:(bool)s {
    env.objc.borrow_mut::<NSOperationQueueHostObject>(this).suspended = s;
}

- (bool)isSuspended {
    env.objc.borrow::<NSOperationQueueHostObject>(this).suspended
}

- (())setName:(id)name {
    let copy: id = msg![env; name copy];
    let old = std::mem::replace(
        &mut env.objc.borrow_mut::<NSOperationQueueHostObject>(this).name,
        copy,
    );
    release(env, old);
}

- (id)name {
    env.objc.borrow::<NSOperationQueueHostObject>(this).name
}

- (MutVoidPtr)underlyingQueue {
    crate::mem::Ptr::null()
}

- (())setUnderlyingQueue:(MutVoidPtr)_q {
}

- (())dealloc {
    let name = env.objc.borrow::<NSOperationQueueHostObject>(this).name;
    release(env, name);
    env.objc.dealloc_object(this, &mut env.mem)
}

@end

};

/// Worker-thread entry point used by `[NSOperationQueue addOperation:]`.
/// Receives the operation object as its single argument, runs its `start`
/// method (which dispatches to `main`), and then releases the operation
/// (balancing the retain in `addOperation:`).
pub fn _touchHLE_NSOperationInvocationHelper(env: &mut Environment, op: id) {
    let _: () = msg![env; op start];
    release(env, op);
}
