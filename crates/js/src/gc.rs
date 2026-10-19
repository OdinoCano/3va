// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! `globalThis.gc()` — exposed with `--expose-gc`, like Node.
//!
//! Node exposes V8's own `gc()` builtin with that flag; 3va has no way to
//! ask V8 for that builtin after the isolate exists, so it installs an
//! equivalent: a full garbage collection requested on the isolate.

use std::cell::Cell;

thread_local! {
    // The live isolate of this thread's JsEngine; cleared when it drops.
    static ISOLATE: Cell<Option<v8::UnsafeRawIsolatePtr>> = const { Cell::new(None) };
}

/// Remembers the isolate whose `globalThis.gc()` must trigger the collection.
pub(crate) fn install(isolate: v8::UnsafeRawIsolatePtr) {
    ISOLATE.with(|i| i.set(Some(isolate)));
}

/// Forgets the isolate, if it is still the one installed.
pub(crate) fn uninstall(isolate: &v8::Isolate) {
    let this = format!("{:?}", unsafe { isolate.as_raw_isolate_ptr() });
    ISOLATE.with(|i| {
        if i.get().is_some_and(|p| format!("{p:?}") == this) {
            i.set(None);
        }
    });
}

fn gc_callback(
    _scope: &mut v8::PinScope,
    _args: v8::FunctionCallbackArguments,
    _rv: v8::ReturnValue,
) {
    let Some(ptr) = ISOLATE.with(|i| i.get()) else {
        return;
    };
    // SAFETY: `ptr` is this thread's live isolate (cleared in JsEngine's
    // Drop before the isolate is disposed), the same pattern
    // `caller_scope::stack_scopes` uses to read the JS stack from a native
    // callback. The collection itself is scheduled by V8 and runs at the next
    // safe point, i.e. after this callback returns.
    let mut isolate = unsafe { v8::Isolate::from_raw_isolate_ptr(ptr) };
    isolate.request_garbage_collection_for_testing(v8::GarbageCollectionType::Full);
}

/// Installs `globalThis.gc` in the current context.
pub(crate) fn install_gc(scope: &mut v8::PinScope) {
    let callback = v8::Function::new(scope, gc_callback).unwrap();
    let global = scope.get_current_context().global(scope);
    let key = v8::String::new(scope, "gc").unwrap();
    global.set(scope, key.into(), callback.into());
}
