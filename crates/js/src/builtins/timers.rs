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
            },
        );
    }

    pub fn cancel(&self, id: TimerId) {
        let mut timers = self.timers.lock().unwrap();
        if let Some(entry) = timers.get_mut(&id) {
            entry.cancelled = true;
        }
    }

    pub fn poll_expired_ids(&self) -> Vec<TimerId> {
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
                expired.push(id);
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

        expired.sort_unstable();
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
        for id in expired {
            let code = format!(
                "if (typeof __fireTimer === 'function') {{ __fireTimer({}); }}",
                id
            );
            let script = v8::Script::compile(scope, v8::String::new(scope, &code).unwrap(), None);
            if let Some(s) = script {
                let _ = s.run(scope);
            }
        }
        Ok(())
    }
}

impl Default for TimerManager {
    fn default() -> Self {
        Self {
            timers: Mutex::new(HashMap::new()),
        }
    }
}

pub fn inject_timers(
    scope: &mut ContextScope<HandleScope>,
    manager: Arc<TimerManager>,
) -> anyhow::Result<()> {
    let mgr_ptr = Arc::into_raw(manager.clone()) as *mut std::ffi::c_void;
    let external = v8::External::new(scope, mgr_ptr);
    let native_set_timeout = v8::Function::builder(
        |scope: &mut PinScope, args: FunctionCallbackArguments, _rv: ReturnValue| {
            let mgr = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                Arc::from_raw(ptr as *const TimerManager)
            };
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as u64;
            let ms = args.get(1).uint32_value(scope).unwrap_or(0) as u64;
            mgr.set_timeout(id, ms);
            std::mem::forget(mgr);
        },
    )
    .data(external.into())
    .build(scope)
    .unwrap();

    let mgr_ptr = Arc::into_raw(manager.clone()) as *mut std::ffi::c_void;
    let external = v8::External::new(scope, mgr_ptr);
    let native_set_interval = v8::Function::builder(
        |scope: &mut PinScope, args: FunctionCallbackArguments, _rv: ReturnValue| {
            let mgr = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                Arc::from_raw(ptr as *const TimerManager)
            };
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as u64;
            let ms = args.get(1).uint32_value(scope).unwrap_or(0) as u64;
            mgr.set_interval(id, ms);
            std::mem::forget(mgr);
        },
    )
    .data(external.into())
    .build(scope)
    .unwrap();

    let mgr_ptr = Arc::into_raw(manager.clone()) as *mut std::ffi::c_void;
    let external = v8::External::new(scope, mgr_ptr);
    let native_set_interval_background = v8::Function::builder(
        |scope: &mut PinScope, args: FunctionCallbackArguments, _rv: ReturnValue| {
            let mgr = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                Arc::from_raw(ptr as *const TimerManager)
            };
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as u64;
            let ms = args.get(1).uint32_value(scope).unwrap_or(0) as u64;
            mgr.set_interval_background(id, ms);
            std::mem::forget(mgr);
        },
    )
    .data(external.into())
    .build(scope)
    .unwrap();

    let mgr_ptr = Arc::into_raw(manager.clone()) as *mut std::ffi::c_void;
    let external = v8::External::new(scope, mgr_ptr);
    let native_clear_timer = v8::Function::builder(
        |scope: &mut PinScope, args: FunctionCallbackArguments, _rv: ReturnValue| {
            let mgr = unsafe {
                let ptr = args.data().cast::<v8::External>().value();
                Arc::from_raw(ptr as *const TimerManager)
            };
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as u64;
            mgr.cancel(id);
            std::mem::forget(mgr);
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
                delete globalThis.__timerCallbacks[id];
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
            var wrapper = function() {
                fn();
                globalThis.__timerCallbacks[id] = wrapper;
            };
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
            var wrapper = function() {
                fn();
                globalThis.__timerCallbacks[id] = wrapper;
            };
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

    let script = v8::Script::compile(scope, v8::String::new(scope, js_polyfill).unwrap(), None)
        .ok_or_else(|| anyhow::anyhow!("compile error"))?;
    let _ = script.run(scope);

    Ok(())
}
