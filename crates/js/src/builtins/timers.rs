// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

use crate::builtins::NativeCtxRegistry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use v8::{ContextScope, FunctionCallbackArguments, HandleScope, PinScope, ReturnValue};

type TimerId = u64;

struct TimerEntry {
    fires_at: Instant,
    repeating: bool,
    interval_ms: u64,
    cancelled: bool,
    /// A "background" timer still fires normally but doesn't count toward
    /// `has_pending()` — used by the CPU profiler's own sampling interval so
    /// it can't keep `run_event_loop` alive on its own once the script's
    /// real work (its own timers/tasks/async/listeners) is done. Without
    /// this, a self-rescheduling interval like the profiler's has no way to
    /// signal "I'm just housekeeping, not real pending work", and the loop
    /// (and the whole process) never exits on its own.
    background: bool,
    /// Package deny scopes active when the timer was scheduled, re-applied
    /// when it fires: `setTimeout(fetch.bind(null, url))` otherwise runs with
    /// only builtin frames on the stack (VULN-03).
    deny_scopes: Vec<String>,
}

pub struct TimerManager {
    timers: Mutex<HashMap<TimerId, TimerEntry>>,
}

impl TimerManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            timers: Mutex::new(HashMap::new()),
        })
    }

    pub fn set_timeout(&self, id: TimerId, ms: u64) {
        let fires_at = Instant::now() + Duration::from_millis(ms);
        let mut timers = self.timers.lock().unwrap();
        timers.insert(
            id,
            TimerEntry {
                fires_at,
                repeating: false,
                interval_ms: 0,
                cancelled: false,
                background: false,
                deny_scopes: vvva_permissions::deny_scopes(),
            },
        );
    }

    pub fn set_interval(&self, id: TimerId, ms: u64) {
        self.set_interval_inner(id, ms, false);
    }

    /// Same as `set_interval`, but the timer is exempt from `has_pending()`
    /// — see `TimerEntry::background`.
    pub fn set_interval_background(&self, id: TimerId, ms: u64) {
        self.set_interval_inner(id, ms, true);
    }

    fn set_interval_inner(&self, id: TimerId, ms: u64, background: bool) {
        let fires_at = Instant::now() + Duration::from_millis(ms);
        let mut timers = self.timers.lock().unwrap();
        timers.insert(
            id,
            TimerEntry {
                fires_at,
                repeating: true,
                interval_ms: ms,
                cancelled: false,
                background,
                deny_scopes: vvva_permissions::deny_scopes(),
            },
        );
    }

    pub fn cancel(&self, id: TimerId) {
        let mut timers = self.timers.lock().unwrap();
        if let Some(entry) = timers.get_mut(&id) {
            entry.cancelled = true;
        }
    }

    pub fn poll_expired_ids(&self) -> Vec<(TimerId, Vec<String>)> {
        let now = Instant::now();
        let mut expired = Vec::new();
        let mut timers = self.timers.lock().unwrap();

        let mut to_remove = Vec::new();
        let mut to_reschedule: Vec<(TimerId, u64)> = Vec::new();

        for (&id, entry) in timers.iter() {
            if entry.cancelled {
                to_remove.push(id);
                continue;
            }
            if entry.fires_at <= now {
                expired.push((id, entry.deny_scopes.clone()));
                if entry.repeating {
                    to_reschedule.push((id, entry.interval_ms));
                } else {
                    to_remove.push(id);
                }
            }
        }

        for id in to_remove {
            timers.remove(&id);
        }
        for (id, ms) in to_reschedule {
            if let Some(entry) = timers.get_mut(&id) {
                entry.fires_at = now + Duration::from_millis(ms);
            }
        }

        expired.sort_unstable_by_key(|(id, _)| *id);
        expired
    }

    pub fn has_pending(&self) -> bool {
        let timers = self.timers.lock().unwrap();
        timers.values().any(|e| !e.cancelled && !e.background)
    }

    pub fn pending_count(&self) -> usize {
        let timers = self.timers.lock().unwrap();
        timers.values().filter(|e| !e.cancelled).count()
    }

    pub fn next_expiry(&self) -> Option<Duration> {
        let now = Instant::now();
        let timers = self.timers.lock().unwrap();
        timers
            .values()
            .filter(|e| !e.cancelled)
            .map(|e| e.fires_at.saturating_duration_since(now))
            .min()
    }

    pub fn fire_pending(
        scope: &mut ContextScope<HandleScope>,
        manager: Arc<Self>,
    ) -> anyhow::Result<()> {
        let expired = manager.poll_expired_ids();
        // poll_expired_ids() already dequeued every id, so keep firing the rest
        // after a throw (dropping them would silently kill unrelated timers,
        // e.g. a socket's poll loop) and report the first error at the end.
        let mut first_error: Option<String> = None;
        if expired.is_empty() {
            return Ok(());
        }
        let fire = timer_fire_fn(scope);
        let recv: v8::Local<v8::Value> = v8::undefined(scope).into();
        for (id, deny_scopes) in expired {
            v8::tc_scope!(let try_catch, scope);
            let Some(fire) = fire.as_ref().map(|f| v8::Local::new(try_catch, f)) else {
                continue;
            };
            let arg: v8::Local<v8::Value> = v8::Number::new(try_catch, id as f64).into();
            let prev_scopes = vvva_permissions::set_inherited_scopes(deny_scopes);
            let ran = fire.call(try_catch, recv, &[arg]).is_some();
            vvva_permissions::set_inherited_scopes(prev_scopes);
            if !ran {
                let text = try_catch
                    .stack_trace()
                    .or_else(|| try_catch.exception())
                    .map(|e| e.to_rust_string_lossy(try_catch))
                    .unwrap_or_else(|| "unknown error".to_string());
                first_error.get_or_insert(text);
            }
        }
        match first_error {
            Some(text) => anyhow::bail!("Uncaught exception: {text}"),
            None => Ok(()),
        }
    }
}

/// The wrapper every timer fires through, compiled once per isolate instead
/// of formatting and compiling a fresh script for each fire. `__fireTimer` is
/// still looked up on each call, as before.
struct TimerFireFn(v8::Global<v8::Function>);

fn timer_fire_fn(scope: &mut ContextScope<HandleScope>) -> Option<v8::Global<v8::Function>> {
    if let Some(f) = scope.get_slot::<TimerFireFn>() {
        return Some(f.0.clone());
    }
    // Like Node: a throw from a timer callback goes to
    // process.on('uncaughtException') if anyone listens, otherwise it is
    // fatal (it used to be printed by V8 and ignored, exiting 0).
    let src = "(function (id) { if (typeof __fireTimer === 'function') { try { __fireTimer(id); } catch (e) { \
               if (typeof process !== 'undefined' && typeof process.listenerCount === 'function' \
               && process.listenerCount('uncaughtException') > 0) { process.emit('uncaughtException', e, 'uncaughtException'); } \
               else { throw e; } } } })";
    let source = v8::String::new(scope, src)?;
    let value = v8::Script::compile(scope, source, None)?.run(scope)?;
    let f = v8::Local::<v8::Function>::try_from(value).ok()?;
    let global = v8::Global::new(scope, f);
    scope.set_slot(TimerFireFn(global.clone()));
    Some(global)
}

impl Default for TimerManager {
    fn default() -> Self {
        Self {
            timers: Mutex::new(HashMap::new()),
        }
    }
}

/// Keeps the event loop alive while an async WebAssembly compile is pending.
///
/// `WebAssembly.compile/instantiate` finish on a V8 background thread and
/// settle through a task the event loop pumps; without something pending the
/// process exits first and the promise never settles. A 1 ms interval holds
/// the loop until the promise settles.
///
/// This must run on every start, not be part of the snapshot: V8 reinstalls
/// the `WebAssembly` functions when it creates a context from a snapshot, so
/// a wrapper recorded in the snapshot is silently replaced by the native
/// functions (the promise then never settled in the default `3va run`).
const WASM_KEEPALIVE_JS: &str = r#"(function() {
    var W = globalThis.WebAssembly;
    if (!W) return;
    ['compile', 'instantiate', 'compileStreaming', 'instantiateStreaming'].forEach(function(k) {
        var orig = W[k];
        if (typeof orig !== 'function') return;
        W[k] = function() {
            var keepAlive = setInterval(function() {}, 1);
            var p;
            try { p = orig.apply(W, arguments); } catch (e) { clearInterval(keepAlive); throw e; }
            return Promise.resolve(p).finally(function() { clearInterval(keepAlive); });
        };
    });
})();"#;

pub fn inject_timers(
    scope: &mut ContextScope<HandleScope>,
    manager: Arc<TimerManager>,
    native_ctx: &mut NativeCtxRegistry,
) -> anyhow::Result<()> {
    let mgr_ptr = native_ctx.leak(manager.clone());
    let external = v8::External::new(scope, mgr_ptr);
    let native_set_timeout = v8::Function::builder(
        |scope: &mut PinScope, args: FunctionCallbackArguments, _rv: ReturnValue| {
            let mgr = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                &*(ptr as *const Arc<TimerManager>)
            };
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as u64;
            let ms = args.get(1).uint32_value(scope).unwrap_or(0) as u64;
            mgr.set_timeout(id, ms);
        },
    )
    .data(external.into())
    .build(scope)
    .unwrap();

    let mgr_ptr = native_ctx.leak(manager.clone());
    let external = v8::External::new(scope, mgr_ptr);
    let native_set_interval = v8::Function::builder(
        |scope: &mut PinScope, args: FunctionCallbackArguments, _rv: ReturnValue| {
            let mgr = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                &*(ptr as *const Arc<TimerManager>)
            };
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as u64;
            let ms = args.get(1).uint32_value(scope).unwrap_or(0) as u64;
            mgr.set_interval(id, ms);
        },
    )
    .data(external.into())
    .build(scope)
    .unwrap();

    let mgr_ptr = native_ctx.leak(manager.clone());
    let external = v8::External::new(scope, mgr_ptr);
    let native_set_interval_background = v8::Function::builder(
        |scope: &mut PinScope, args: FunctionCallbackArguments, _rv: ReturnValue| {
            let mgr = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                &*(ptr as *const Arc<TimerManager>)
            };
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as u64;
            let ms = args.get(1).uint32_value(scope).unwrap_or(0) as u64;
            mgr.set_interval_background(id, ms);
        },
    )
    .data(external.into())
    .build(scope)
    .unwrap();

    let mgr_ptr = native_ctx.leak(manager.clone());
    let external = v8::External::new(scope, mgr_ptr);
    let native_clear_timer = v8::Function::builder(
        |scope: &mut PinScope, args: FunctionCallbackArguments, _rv: ReturnValue| {
            let mgr = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                &*(ptr as *const Arc<TimerManager>)
            };
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as u64;
            mgr.cancel(id);
        },
    )
    .data(external.into())
    .build(scope)
    .unwrap();

    let context = scope.get_current_context();
    let global = context.global(scope);
    global.set(
        scope,
        v8::String::new(scope, "__nativeSetTimeout").unwrap().into(),
        native_set_timeout.into(),
    );
    global.set(
        scope,
        v8::String::new(scope, "__nativeSetInterval")
            .unwrap()
            .into(),
        native_set_interval.into(),
    );
    global.set(
        scope,
        v8::String::new(scope, "__nativeSetIntervalBackground")
            .unwrap()
            .into(),
        native_set_interval_background.into(),
    );
    global.set(
        scope,
        v8::String::new(scope, "__nativeClearTimer").unwrap().into(),
        native_clear_timer.into(),
    );

    let js_polyfill = r#"
        globalThis.__timerCallbacks = {};
        globalThis.__timerNextId = 0;

        globalThis.__fireTimer = function(id) {
            var fn = globalThis.__timerCallbacks[id];
            if (fn) {
                // An interval's entry stays for as long as it runs, so a
                // clearInterval() from inside its own callback removes it
                // for good (re-adding it after the call kept it forever).
                if (!fn._repeat) delete globalThis.__timerCallbacks[id];
                fn();
            }
        };

        function __makeTimerHandle(id, ms, repeat) {
            var handle = {
                _id: id,
                _ms: ms,
                _repeat: repeat,
                unref: function() { return handle; },
                ref: function() { return handle; },
                refresh: function() {
                    // Re-arm: cancel then reschedule with same callback.
                    var cb = globalThis.__timerCallbacks[handle._id];
                    __nativeClearTimer(handle._id);
                    globalThis.__timerNextId = (globalThis.__timerNextId || 0) + 1;
                    handle._id = globalThis.__timerNextId;
                    globalThis.__timerCallbacks[handle._id] = cb;
                    if (repeat) __nativeSetInterval(handle._id, ms);
                    else __nativeSetTimeout(handle._id, ms);
                    return handle;
                },
                hasRef: function() { return true; }
            };
            return handle;
        }
        function __numericId(id) {
            if (id == null) return null;
            return (typeof id === 'object' && id !== null && id._id != null) ? id._id : id;
        }

        globalThis.setTimeout = function(fn, ms) {
            globalThis.__timerNextId = (globalThis.__timerNextId || 0) + 1;
            var id = globalThis.__timerNextId;
            var delay = Math.floor(+ms) || 0;
            globalThis.__timerCallbacks[id] = fn;
            __nativeSetTimeout(id, delay);
            return __makeTimerHandle(id, delay, false);
        };

        globalThis.clearTimeout = function(id) {
            var numId = __numericId(id);
            if (numId == null) return;
            delete globalThis.__timerCallbacks[numId];
            __nativeClearTimer(numId);
        };

        globalThis.setInterval = function(fn, ms) {
            globalThis.__timerNextId = (globalThis.__timerNextId || 0) + 1;
            var id = globalThis.__timerNextId;
            var intervalMs = Math.floor(+ms) || 0;
            var wrapper = function() { fn(); };
            wrapper._repeat = true;
            globalThis.__timerCallbacks[id] = wrapper;
            __nativeSetInterval(id, intervalMs);
            return __makeTimerHandle(id, intervalMs, true);
        };

        // Not a public global — internal use only (e.g. the CPU profiler's
        // own sampling interval). Same shape as setInterval, but the timer
        // doesn't count as "pending work" that keeps run_event_loop (and
        // the process) alive; clearInterval/the returned handle work on it
        // exactly the same as a normal interval.
        globalThis.__setIntervalBackground = function(fn, ms) {
            globalThis.__timerNextId = (globalThis.__timerNextId || 0) + 1;
            var id = globalThis.__timerNextId;
            var intervalMs = Math.floor(+ms) || 0;
            var wrapper = function() { fn(); };
            wrapper._repeat = true;
            globalThis.__timerCallbacks[id] = wrapper;
            __nativeSetIntervalBackground(id, intervalMs);
            return __makeTimerHandle(id, intervalMs, true);
        };

        globalThis.clearInterval = function(id) {
            var numId = __numericId(id);
            if (numId == null) return;
            delete globalThis.__timerCallbacks[numId];
            __nativeClearTimer(numId);
        };

        if (typeof globalThis.setImmediate === 'undefined') {
            globalThis.setImmediate = function(fn) {
                return globalThis.setTimeout(fn, 0);
            };
            globalThis.clearImmediate = function(id) {
                globalThis.clearTimeout(id);
            };
        }

        globalThis.queueMicrotask = function(fn) {
            Promise.resolve().then(fn);
        };
    "#;

    crate::builtins::code_cache::bootstrap_js(scope, "timers", js_polyfill)?;
    crate::builtins::code_cache::bootstrap_js_per_run(scope, "wasm-keepalive", WASM_KEEPALIVE_JS)?;

    Ok(())
}
