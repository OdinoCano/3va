pub mod task_queue;
pub mod timer;

use vvva_permissions::PermissionState;

pub use task_queue::{Task, TaskQueue, TaskType};
pub use timer::{TimerId, TimerWheel};

pub struct Runtime {
    pub permissions: PermissionState,
    task_queue: TaskQueue,
    timer_wheel: TimerWheel,
}

impl Runtime {
    pub fn new(permissions: PermissionState) -> Self {
        Self {
            permissions,
            task_queue: TaskQueue::new(),
            timer_wheel: TimerWheel::new(),
        }
    }

    /// Drive the event loop: fire expired timers and drain completed tasks.
    /// Returns when all pending work is exhausted or the safety limit is hit.
    pub fn run(&mut self) -> anyhow::Result<()> {
        const MAX_ITERS: usize = 100_000;
        let mut iters = 0;

        while self.pending_task_count() > 0 && iters < MAX_ITERS {
            iters += 1;

            // Fire all timers that have expired
            let expired = self.timer_wheel.poll();
            for timer in expired {
                (timer.callback)();
                // Re-add repeating timers
                if timer.repeating {
                    if let Some(interval) = timer.interval {
                        self.timer_wheel.schedule_with_callback(interval, || {});
                    }
                }
            }

            // Short yield to let tokio tasks make progress
            if self.task_queue.pending_count() > 0 {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }

            // Next timer hasn't fired yet — sleep until it's due
            if let Some(wait) = self.next_timer_duration() {
                if wait > std::time::Duration::ZERO {
                    std::thread::sleep(wait.min(std::time::Duration::from_millis(50)));
                }
            }
        }

        Ok(())
    }

    pub fn schedule_task<F>(&mut self, task_type: TaskType, future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let task_id = self.task_queue.next_id();
        let task = Task::with_future(task_id, task_type, tokio::spawn(future));
        self.task_queue.push(task);
    }

    pub fn set_timeout<F>(&mut self, delay: std::time::Duration, callback: F) -> TimerId
    where
        F: FnOnce() + Send + 'static,
    {
        self.timer_wheel.schedule_with_callback(delay, callback)
    }

    pub fn set_interval<F>(&mut self, interval: std::time::Duration, callback: F) -> TimerId
    where
        F: FnOnce() + Send + 'static,
    {
        self.timer_wheel
            .schedule_interval_with_callback(interval, callback)
    }

    pub fn clear_timeout(&mut self, id: TimerId) -> bool {
        self.timer_wheel.cancel(id)
    }

    pub fn poll_timers(&mut self) -> Vec<timer::Timer> {
        self.timer_wheel.poll()
    }

    pub fn next_timer_duration(&self) -> Option<std::time::Duration> {
        self.timer_wheel.next_duration()
    }

    pub fn pending_task_count(&self) -> usize {
        self.task_queue.pending_count() + self.timer_wheel.pending_count()
    }
}
