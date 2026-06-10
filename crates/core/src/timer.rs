use std::collections::VecDeque;
use std::time::{Duration, Instant};

// Wheel thresholds — chosen so each wheel covers roughly 4× the previous one.
const MS_THRESHOLD_MS: u128 = 64; // < 64 ms  → millisecond wheel
const S_THRESHOLD_S: u64 = 4; // < 4 s    → second wheel
const M_THRESHOLD_S: u64 = 240; // < 4 min  → minute wheel
const H_THRESHOLD_S: u64 = 14_400; // < 4 h    → hour wheel
const D_THRESHOLD_S: u64 = 345_600; // < 4 days → day wheel
// ≥ 4 days → large wheel

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerId(pub u64);

impl TimerId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

pub struct Timer {
    pub id: TimerId,
    pub callback: Box<dyn Fn() + Send>,
    pub scheduled_at: Instant,
    pub repeating: bool,
    pub interval: Option<Duration>,
}

impl Timer {
    pub fn new(id: TimerId, callback: Box<dyn Fn() + Send>, delay: Duration) -> Self {
        let scheduled_at = Instant::now() + delay;
        Self {
            id,
            callback,
            scheduled_at,
            repeating: false,
            interval: None,
        }
    }

    pub fn interval(id: TimerId, callback: Box<dyn Fn() + Send>, interval: Duration) -> Self {
        let scheduled_at = Instant::now() + interval;
        Self {
            id,
            callback,
            scheduled_at,
            repeating: true,
            interval: Some(interval),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.scheduled_at <= Instant::now()
    }

    pub fn reschedule(&mut self) {
        if let Some(interval) = self.interval {
            self.scheduled_at = Instant::now() + interval;
        }
    }
}

pub struct TimerWheel {
    wheel_ms: VecDeque<Timer>,
    wheel_s: VecDeque<Timer>,
    wheel_m: VecDeque<Timer>,
    wheel_h: VecDeque<Timer>,
    wheel_d: VecDeque<Timer>,
    wheel_large: VecDeque<Timer>,
    next_timer_id: u64,
}

impl TimerWheel {
    pub fn new() -> Self {
        Self {
            wheel_ms: VecDeque::new(),
            wheel_s: VecDeque::new(),
            wheel_m: VecDeque::new(),
            wheel_h: VecDeque::new(),
            wheel_d: VecDeque::new(),
            wheel_large: VecDeque::new(),
            next_timer_id: 0,
        }
    }

    pub fn next_timer_id(&mut self) -> TimerId {
        let id = self.next_timer_id;
        self.next_timer_id += 1;
        TimerId(id)
    }

    pub fn schedule(&mut self, delay: Duration) -> TimerId {
        let id = self.next_timer_id();
        let timer = Timer::new(id, Box::new(|| {}), delay);
        self.add_timer(timer);
        id
    }

    pub fn schedule_with_callback<F>(&mut self, delay: Duration, callback: F) -> TimerId
    where
        F: Fn() + Send + 'static,
    {
        let id = self.next_timer_id();
        let timer = Timer::new(id, Box::new(callback), delay);
        self.add_timer(timer);
        id
    }

    pub fn schedule_interval_with_callback<F>(&mut self, interval: Duration, callback: F) -> TimerId
    where
        F: Fn() + Send + 'static,
    {
        let id = self.next_timer_id();
        let timer = Timer::interval(id, Box::new(callback), interval);
        self.add_timer(timer);
        id
    }

    fn add_timer(&mut self, timer: Timer) {
        let delay = timer.scheduled_at.duration_since(Instant::now());
        let wheel = if delay.as_millis() < MS_THRESHOLD_MS {
            &mut self.wheel_ms
        } else if delay.as_secs() < S_THRESHOLD_S {
            &mut self.wheel_s
        } else if delay.as_secs() < M_THRESHOLD_S {
            &mut self.wheel_m
        } else if delay.as_secs() < H_THRESHOLD_S {
            &mut self.wheel_h
        } else if delay.as_secs() < D_THRESHOLD_S {
            &mut self.wheel_d
        } else {
            &mut self.wheel_large
        };
        wheel.push_back(timer);
    }

    /// Drain one ready timer from `wheel`, re-enqueue repeating ones, append to `ready`.
    fn poll_wheel(wheel: &mut VecDeque<Timer>, ready: &mut Vec<Timer>, repeating_ok: bool) {
        while let Some(timer) = wheel.pop_front() {
            if timer.is_ready() {
                if repeating_ok && timer.repeating {
                    let mut t = timer;
                    t.reschedule();
                    ready.push(t);
                } else {
                    ready.push(timer);
                }
            } else {
                wheel.push_front(timer);
                break;
            }
        }
    }

    pub fn poll(&mut self) -> Vec<Timer> {
        let mut ready = Vec::new();
        Self::poll_wheel(&mut self.wheel_ms, &mut ready, true);
        Self::poll_wheel(&mut self.wheel_s, &mut ready, true);
        Self::poll_wheel(&mut self.wheel_m, &mut ready, true);
        Self::poll_wheel(&mut self.wheel_h, &mut ready, true);
        Self::poll_wheel(&mut self.wheel_d, &mut ready, true);
        Self::poll_wheel(&mut self.wheel_large, &mut ready, false);
        ready
    }

    pub fn cancel(&mut self, id: TimerId) -> bool {
        let raw = id.0;
        for wheel in [
            &mut self.wheel_ms,
            &mut self.wheel_s,
            &mut self.wheel_m,
            &mut self.wheel_h,
            &mut self.wheel_d,
            &mut self.wheel_large,
        ] {
            if let Some(pos) = wheel.iter().position(|t| t.id.0 == raw) {
                wheel.remove(pos);
                return true;
            }
        }
        false
    }

    pub fn pending_count(&self) -> usize {
        self.wheel_ms.len()
            + self.wheel_s.len()
            + self.wheel_m.len()
            + self.wheel_h.len()
            + self.wheel_d.len()
            + self.wheel_large.len()
    }

    pub fn next_duration(&self) -> Option<Duration> {
        let wheels: [&VecDeque<Timer>; 6] = [
            &self.wheel_ms,
            &self.wheel_s,
            &self.wheel_m,
            &self.wheel_h,
            &self.wheel_d,
            &self.wheel_large,
        ];
        let now = Instant::now();
        wheels
            .iter()
            .flat_map(|w| w.iter())
            .map(|t| t.scheduled_at.duration_since(now))
            .reduce(|a, b| a.min(b))
    }
}

impl Default for TimerWheel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timer_creation() {
        let mut wheel = TimerWheel::new();
        let id = wheel.schedule(Duration::from_millis(10));
        assert_eq!(wheel.pending_count(), 1);
        assert!(wheel.cancel(id));
        assert_eq!(wheel.pending_count(), 0);
    }

    #[test]
    fn test_timer_wheel_scheduling() {
        let mut wheel = TimerWheel::new();

        wheel.schedule_with_callback(Duration::from_millis(1), || {});
        wheel.schedule_with_callback(Duration::from_millis(50), || {});
        wheel.schedule_with_callback(Duration::from_secs(2), || {});

        assert_eq!(wheel.pending_count(), 3);
    }
}
