use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerId(pub u64);

impl TimerId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }
}

pub struct Timer {
    pub id: TimerId,
    pub callback: Box<dyn FnOnce() + Send>,
    pub scheduled_at: Instant,
    pub repeating: bool,
    pub interval: Option<Duration>,
}

impl Timer {
    pub fn new(id: TimerId, callback: Box<dyn FnOnce() + Send>, delay: Duration) -> Self {
        let scheduled_at = Instant::now() + delay;
        Self {
            id,
            callback,
            scheduled_at,
            repeating: false,
            interval: None,
        }
    }

    pub fn interval(
        id: TimerId,
        callback: Box<dyn FnOnce() + Send + Send>,
        interval: Duration,
    ) -> Self {
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
        F: FnOnce() + Send + 'static,
    {
        let id = self.next_timer_id();
        let timer = Timer::new(id, Box::new(callback), delay);
        self.add_timer(timer);
        id
    }

    pub fn schedule_interval_with_callback<F>(&mut self, interval: Duration, callback: F) -> TimerId
    where
        F: FnOnce() + Send + 'static,
    {
        let id = self.next_timer_id();
        let timer = Timer::interval(id, Box::new(callback), interval);
        self.add_timer(timer);
        id
    }

    fn add_timer(&mut self, timer: Timer) {
        let delay = timer.scheduled_at.duration_since(Instant::now());

        if delay.as_millis() < 64 {
            self.wheel_ms.push_back(timer);
        } else if delay.as_secs() < 4 {
            self.wheel_s.push_back(timer);
        } else if delay.as_secs() < 240 {
            self.wheel_m.push_back(timer);
        } else if delay.as_secs() < 14400 {
            self.wheel_h.push_back(timer);
        } else if delay.as_secs() < 345600 {
            self.wheel_d.push_back(timer);
        } else {
            self.wheel_large.push_back(timer);
        }
    }

    pub fn poll(&mut self) -> Vec<Timer> {
        let mut ready = Vec::new();

        while let Some(timer) = self.wheel_ms.pop_front() {
            if timer.is_ready() {
                if timer.repeating {
                    let mut t = timer;
                    t.reschedule();
                    ready.push(t);
                } else {
                    ready.push(timer);
                }
            } else {
                self.wheel_ms.push_front(timer);
                break;
            }
        }

        while let Some(timer) = self.wheel_s.pop_front() {
            if timer.is_ready() {
                if timer.repeating {
                    let mut t = timer;
                    t.reschedule();
                    ready.push(t);
                } else {
                    ready.push(timer);
                }
            } else {
                self.wheel_s.push_front(timer);
                break;
            }
        }

        while let Some(timer) = self.wheel_m.pop_front() {
            if timer.is_ready() {
                if timer.repeating {
                    let mut t = timer;
                    t.reschedule();
                    ready.push(t);
                } else {
                    ready.push(timer);
                }
            } else {
                self.wheel_m.push_front(timer);
                break;
            }
        }

        while let Some(timer) = self.wheel_h.pop_front() {
            if timer.is_ready() {
                if timer.repeating {
                    let mut t = timer;
                    t.reschedule();
                    ready.push(t);
                } else {
                    ready.push(timer);
                }
            } else {
                self.wheel_h.push_front(timer);
                break;
            }
        }

        while let Some(timer) = self.wheel_d.pop_front() {
            if timer.is_ready() {
                if timer.repeating {
                    let mut t = timer;
                    t.reschedule();
                    ready.push(t);
                } else {
                    ready.push(timer);
                }
            } else {
                self.wheel_d.push_front(timer);
                break;
            }
        }

        while let Some(timer) = self.wheel_large.pop_front() {
            if timer.is_ready() {
                ready.push(timer);
            } else {
                self.wheel_large.push_front(timer);
                break;
            }
        }

        ready
    }

    pub fn cancel(&mut self, id: TimerId) -> bool {
        let id = id.0;

        if let Some(pos) = self.wheel_ms.iter().position(|t| t.id.0 == id) {
            self.wheel_ms.remove(pos);
            return true;
        }
        if let Some(pos) = self.wheel_s.iter().position(|t| t.id.0 == id) {
            self.wheel_s.remove(pos);
            return true;
        }
        if let Some(pos) = self.wheel_m.iter().position(|t| t.id.0 == id) {
            self.wheel_m.remove(pos);
            return true;
        }
        if let Some(pos) = self.wheel_h.iter().position(|t| t.id.0 == id) {
            self.wheel_h.remove(pos);
            return true;
        }
        if let Some(pos) = self.wheel_d.iter().position(|t| t.id.0 == id) {
            self.wheel_d.remove(pos);
            return true;
        }
        if let Some(pos) = self.wheel_large.iter().position(|t| t.id.0 == id) {
            self.wheel_large.remove(pos);
            return true;
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
        let mut min = None;

        for timer in &self.wheel_ms {
            let remaining = timer.scheduled_at.duration_since(Instant::now());
            min = Some(min.map_or(remaining, |m: Duration| m.min(remaining)));
        }
        for timer in &self.wheel_s {
            let remaining = timer.scheduled_at.duration_since(Instant::now());
            min = Some(min.map_or(remaining, |m: Duration| m.min(remaining)));
        }
        for timer in &self.wheel_m {
            let remaining = timer.scheduled_at.duration_since(Instant::now());
            min = Some(min.map_or(remaining, |m: Duration| m.min(remaining)));
        }
        for timer in &self.wheel_h {
            let remaining = timer.scheduled_at.duration_since(Instant::now());
            min = Some(min.map_or(remaining, |m: Duration| m.min(remaining)));
        }
        for timer in &self.wheel_d {
            let remaining = timer.scheduled_at.duration_since(Instant::now());
            min = Some(min.map_or(remaining, |m: Duration| m.min(remaining)));
        }
        for timer in &self.wheel_large {
            let remaining = timer.scheduled_at.duration_since(Instant::now());
            min = Some(min.map_or(remaining, |m: Duration| m.min(remaining)));
        }

        min
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
