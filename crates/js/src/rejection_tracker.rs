//! Reports unhandled promise rejections to stderr using V8's promise rejection
//! callback mechanism.

use v8::{Isolate, PromiseRejectEvent, PromiseRejectMessage};

extern "C" fn on_promise_reject(msg: PromiseRejectMessage) {
    if msg.get_event() != PromiseRejectEvent::PromiseRejectWithNoHandler {
        return;
    }
    v8::callback_scope!(unsafe scope, &msg);
    let text = msg
        .get_value()
        .map(|v| v.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "unknown".to_string());
    eprintln!("Unhandled promise rejection: {text}");
}

pub fn install(isolate: &mut Isolate) {
    isolate.set_promise_reject_callback(on_promise_reject);
}
