//! Reports unhandled promise rejections to stderr.
//!
//! V8's PromiseRejectWithNoHandler fires immediately when reject() is called —
//! before any synchronous .catch() can be attached. Node.js defers reporting
//! until after microtasks drain. We do the same: PENDING += rejection,
//! HANDLED -= it if PromiseHandlerAddedAfterReject fires, then report survivors
//! in report_pending() after each microtask checkpoint.

use std::cell::RefCell;
use v8::{Isolate, PromiseRejectEvent, PromiseRejectMessage};

struct Pending {
    promise_ptr: usize,
    text: String,
    stack: String,
}

thread_local! {
    static PENDING: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
    static HANDLED: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

fn local_ptr<T>(local: v8::Local<T>) -> usize {
    // Local<T> is repr(transparent) over NonNull<T>; read it as a pointer-sized int.
    // Safe: Local<T> is guaranteed to be the same size as a pointer.
    unsafe { *(&local as *const _ as *const usize) }
}

extern "C" fn on_promise_reject(msg: PromiseRejectMessage) {
    v8::callback_scope!(unsafe scope, &msg);
    let ptr = local_ptr(msg.get_promise());
    match msg.get_event() {
        PromiseRejectEvent::PromiseRejectWithNoHandler => {
            let text = msg
                .get_value()
                .map(|v| v.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "unknown".to_string());
            let stack = msg
                .get_value()
                .and_then(|v| {
                    let obj = v8::Local::<v8::Object>::try_from(v).ok()?;
                    let key = v8::String::new(scope, "stack")?;
                    let sv = obj.get(scope, key.into())?;
                    if sv.is_undefined() {
                        None
                    } else {
                        Some(sv.to_rust_string_lossy(scope))
                    }
                })
                .unwrap_or_default();
            PENDING.with(|p| {
                p.borrow_mut().push(Pending {
                    promise_ptr: ptr,
                    text,
                    stack,
                })
            });
        }
        PromiseRejectEvent::PromiseHandlerAddedAfterReject => {
            HANDLED.with(|h| h.borrow_mut().push(ptr));
        }
        _ => {}
    }
}

/// Call after every microtask checkpoint.
/// Rejections cancelled by PromiseHandlerAddedAfterReject are suppressed.
pub fn report_pending() {
    HANDLED.with(|h| {
        let mut handled = h.borrow_mut();
        PENDING.with(|p| {
            let mut pending = p.borrow_mut();
            pending.retain(|r| !handled.contains(&r.promise_ptr));
            for r in pending.drain(..) {
                eprintln!("Unhandled promise rejection: {}", r.text);
                if !r.stack.is_empty() {
                    eprintln!("{}", r.stack);
                }
            }
        });
        handled.clear();
    });
}

pub fn install(isolate: &mut Isolate) {
    isolate.set_promise_reject_callback(on_promise_reject);
}
