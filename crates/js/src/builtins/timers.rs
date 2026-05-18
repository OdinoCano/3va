use rquickjs::{Ctx, Result, Function};
use std::time::Duration;
use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use vvva_core::timer::TimerWheel;

type TimerId = u64;

pub struct TimerManager {
    wheel: Mutex<TimerWheel>,
    next_id: Mutex<TimerId>,
    callbacks: Mutex<HashMap<TimerId, String>>,
}

impl TimerManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            wheel: Mutex::new(TimerWheel::new()),
            next_id: Mutex::new(1),
            callbacks: Mutex::new(HashMap::new()),
        })
    }

    pub fn set_timeout(&self, callback_code: String, ms: u64) -> TimerId {
        let id = {
            let mut next = self.next_id.lock().unwrap();
            let id = *next;
            *next += 1;
            id
        };
        
        {
            let mut callbacks = self.callbacks.lock().unwrap();
            callbacks.insert(id, callback_code);
        }
        
        let delay = Duration::from_millis(ms);
        self.wheel.lock().unwrap().schedule_with_callback(delay, move || {});
        
        id
    }

    pub fn set_interval(&self, callback_code: String, ms: u64) -> TimerId {
        let id = {
            let mut next = self.next_id.lock().unwrap();
            let id = *next;
            *next += 1;
            id
        };
        
        {
            let mut callbacks = self.callbacks.lock().unwrap();
            callbacks.insert(id, callback_code);
        }
        
        let delay = Duration::from_millis(ms);
        self.wheel.lock().unwrap().schedule_interval_with_callback(delay, move || {});
        
        id
    }

    pub fn clear(&self, id: TimerId) {
        self.wheel.lock().unwrap().cancel(vvva_core::timer::TimerId(id));
        self.callbacks.lock().unwrap().remove(&id);
    }

    pub fn poll(&self, ctx: &Ctx) {
        let ready = self.wheel.lock().unwrap().poll();
        let callbacks = self.callbacks.lock().unwrap();
        
        for timer in ready {
            let id = timer.id.0;
            if let Some(code) = callbacks.get(&id) {
                let _ = ctx.eval::<(), _>(code.as_str());
            }
        }
    }
}

impl Default for TimerManager {
    fn default() -> Self {
        Self {
            wheel: Mutex::new(TimerWheel::new()),
            next_id: Mutex::new(1),
            callbacks: Mutex::new(HashMap::new()),
        }
    }
}

pub fn inject_timers(ctx: &Ctx) -> rquickjs::Result<()> {
    ctx.eval::<(), _>(r#"
        global.__timerCallbacks = {};
        global.setTimeout = function(fn, ms) {
            var id = Math.floor(Math.random() * 1000000);
            global.__timerCallbacks[id] = fn;
            return id;
        };
        global.clearTimeout = function(id) {
            delete global.__timerCallbacks[id];
        };
        global.setInterval = function(fn, ms) {
            var id = Math.floor(Math.random() * 1000000);
            global.__timerCallbacks[id] = fn;
            return id;
        };
        global.clearInterval = function(id) {
            delete global.__timerCallbacks[id];
        };
    "#)?;
    Ok(())
}