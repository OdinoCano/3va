//! Reports unhandled promise rejections to stderr instead of letting them
//! vanish silently.
//!
//! Without this, a rejected promise with no `.catch()` (e.g. an unawaited
//! `async function` call, a common pattern in CLI entry points like Vite's
//! `bin/vite.js`) just disappears: our event loop eventually notices there's
//! no more pending work and exits 0, with zero output explaining why nothing
//! happened. Node's default behavior is to print the rejection reason (and,
//! since Node 15, crash) — this wires up QuickJS's native
//! `JS_SetHostPromiseRejectionTracker` hook to do the same.
//!
//! ponytail: we print on the first "unhandled" callback and don't track
//! retraction (the tracker fires again with `is_handled=1` if a `.catch` is
//! attached on a later microtask turn) — so a rejection caught one tick late
//! can print a spurious warning. Good enough to turn "silent death" into
//! "some log line to grep for"; add retraction tracking if false positives
//! become a real problem.
use std::os::raw::{c_int, c_void};

unsafe extern "C" fn on_rejection(
    ctx: *mut rquickjs_sys::JSContext,
    _promise: rquickjs_sys::JSValue,
    reason: rquickjs_sys::JSValue,
    is_handled: c_int,
    _opaque: *mut c_void,
) {
    if is_handled != 0 {
        return;
    }
    let mut len: rquickjs_sys::size_t = 0;
    let c_str = unsafe { rquickjs_sys::JS_ToCStringLen2(ctx, &mut len, reason, 0) };
    if c_str.is_null() {
        eprintln!("Unhandled promise rejection (reason could not be stringified)");
        return;
    }
    let msg = unsafe { std::ffi::CStr::from_ptr(c_str) }
        .to_string_lossy()
        .into_owned();
    unsafe { rquickjs_sys::JS_FreeCString(ctx, c_str) };

    // If `reason` is an Error, its `.stack` says which JS call actually threw
    // — the toString() above only gives "Name: message", not where. Best
    // effort: skip silently if the property read/stringify fails.
    let stack = unsafe {
        let prop = c"stack".as_ptr();
        let stack_val = rquickjs_sys::JS_GetPropertyStr(ctx, reason, prop);
        if rquickjs_sys::JS_IsUndefined(stack_val) || rquickjs_sys::JS_IsNull(stack_val) {
            rquickjs_sys::JS_FreeValue(ctx, stack_val);
            None
        } else {
            let mut slen: rquickjs_sys::size_t = 0;
            let stack_c = rquickjs_sys::JS_ToCStringLen2(ctx, &mut slen, stack_val, 0);
            let out = if stack_c.is_null() {
                None
            } else {
                let s = std::ffi::CStr::from_ptr(stack_c)
                    .to_string_lossy()
                    .into_owned();
                rquickjs_sys::JS_FreeCString(ctx, stack_c);
                Some(s)
            };
            rquickjs_sys::JS_FreeValue(ctx, stack_val);
            out
        }
    };

    match stack {
        Some(stack) => eprintln!("Unhandled promise rejection: {msg}\n{stack}"),
        None => eprintln!("Unhandled promise rejection: {msg}"),
    }
}

/// # Safety
/// `rt_ptr` must be a valid `*mut JSRuntime` for the lifetime of the runtime.
pub unsafe fn install(rt_ptr: *mut c_void) {
    unsafe {
        rquickjs_sys::JS_SetHostPromiseRejectionTracker(
            rt_ptr as *mut rquickjs_sys::JSRuntime,
            Some(on_rejection),
            std::ptr::null_mut(),
        );
    }
}
