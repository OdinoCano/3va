use rquickjs::{Ctx, Function, Result, Value, function::Rest};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type TimerId = u64;

/// Entry in the timer manager: tracks when each timer fires.
struct TimerEntry {
    fires_at: Instant,
    repeating: bool,
    interval_ms: u64,
    cancelled: bool,
}

/// Manages JS timers — registration and expiry polling.
/// Stored in a thread-local so Rust-backed native functions can access it.
pub struct TimerManager {
    timers: Mutex<HashMap<TimerId, TimerEntry>>,
}

impl TimerManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            timers: Mutex::new(HashMap::new()),
        })
    }

    /// Register a one-shot timer.
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
            },
        );
    }

    /// Register a repeating interval timer.
    pub fn set_interval(&self, id: TimerId, ms: u64) {
        let fires_at = Instant::now() + Duration::from_millis(ms);
        let mut timers = self.timers.lock().unwrap();
        timers.insert(
            id,
            TimerEntry {
                fires_at,
                repeating: true,
                interval_ms: ms,
                cancelled: false,
            },
        );
    }

    /// Cancel a timer by ID.
    pub fn cancel(&self, id: TimerId) {
        let mut timers = self.timers.lock().unwrap();
        if let Some(entry) = timers.get_mut(&id) {
            entry.cancelled = true;
        }
    }

    /// Return IDs of all expired (and not cancelled) timers, and reschedule intervals.
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

        expired
    }

    /// Return whether there are any pending (non-cancelled) timers.
    pub fn has_pending(&self) -> bool {
        let timers = self.timers.lock().unwrap();
        timers.values().any(|e| !e.cancelled)
    }

    /// Next expiry duration (for sleep decisions).
    pub fn next_expiry(&self) -> Option<Duration> {
        let now = Instant::now();
        let timers = self.timers.lock().unwrap();
        timers
            .values()
            .filter(|e| !e.cancelled)
            .map(|e| e.fires_at.saturating_duration_since(now))
            .min()
    }

    /// Fire all expired timers by calling the JS `__fireTimer(id)` function.
    pub fn fire_pending(ctx: &Ctx, manager: Arc<Self>) -> Result<()> {
        let expired = manager.poll_expired_ids();
        for id in expired {
            let code = format!(
                "if (typeof __fireTimer === 'function') {{ __fireTimer({}); }}",
                id
            );
            let _ = ctx.eval::<Value, _>(code.as_str());
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

/// Inject timer globals into the QuickJS context.
pub fn inject_timers(ctx: &Ctx, manager: Arc<TimerManager>) -> Result<()> {
    // __nativeSetTimeout(id, ms) — returns nothing (JS side stores the id)
    let mgr = manager.clone();
    let native_set_timeout = Function::new(ctx.clone(), move |args: Rest<i32>| {
        let mut iter = args.0.iter();
        let id = iter.next().copied().unwrap_or(0) as u64;
        let ms = iter.next().copied().unwrap_or(0) as u64;
        mgr.set_timeout(id, ms);
    })?;

    // __nativeSetInterval(id, ms)
    let mgr = manager.clone();
    let native_set_interval = Function::new(ctx.clone(), move |args: Rest<i32>| {
        let mut iter = args.0.iter();
        let id = iter.next().copied().unwrap_or(0) as u64;
        let ms = iter.next().copied().unwrap_or(0) as u64;
        mgr.set_interval(id, ms);
    })?;

    // __nativeClearTimer(id)
    let mgr = manager.clone();
    let native_clear_timer = Function::new(ctx.clone(), move |args: Rest<i32>| {
        let id = args.0.first().copied().unwrap_or(0) as u64;
        mgr.cancel(id);
    })?;

    let globals = ctx.globals();
    globals.set("__nativeSetTimeout", native_set_timeout)?;
    globals.set("__nativeSetInterval", native_set_interval)?;
    globals.set("__nativeClearTimer", native_clear_timer)?;

    // Inject JS-level timer wrappers
    ctx.eval::<(), _>(
        r#"
        globalThis.__timerCallbacks = {};
        globalThis.__timerNextId = 0;

        globalThis.__fireTimer = function(id) {
            var fn = globalThis.__timerCallbacks[id];
            if (fn) {
                delete globalThis.__timerCallbacks[id];
                fn();
            }
        };

        globalThis.setTimeout = function(fn, ms) {
            globalThis.__timerNextId = (globalThis.__timerNextId || 0) + 1;
            var id = globalThis.__timerNextId;
            globalThis.__timerCallbacks[id] = fn;
            __nativeSetTimeout(id, ms || 0);
            return id;
        };

        globalThis.clearTimeout = function(id) {
            delete globalThis.__timerCallbacks[id];
            __nativeClearTimer(id);
        };

        globalThis.setInterval = function(fn, ms) {
            globalThis.__timerNextId = (globalThis.__timerNextId || 0) + 1;
            var id = globalThis.__timerNextId;
            var intervalMs = ms || 0;
            var wrapper = function() {
                fn();
                // Re-register callback for next interval tick
                globalThis.__timerCallbacks[id] = wrapper;
            };
            globalThis.__timerCallbacks[id] = wrapper;
            __nativeSetInterval(id, intervalMs);
            return id;
        };

        globalThis.clearInterval = function(id) {
            delete globalThis.__timerCallbacks[id];
            __nativeClearTimer(id);
        };

        // setImmediate/clearImmediate — schedules a callback as a 0ms timeout
        globalThis.setImmediate = function(fn) {
            return globalThis.setTimeout(fn, 0);
        };
        globalThis.clearImmediate = function(id) {
            globalThis.clearTimeout(id);
        };

        // queueMicrotask — fires after current sync execution, before timers
        globalThis.queueMicrotask = function(fn) {
            Promise.resolve().then(fn);
        };
    "#,
    )?;

    Ok(())
}
