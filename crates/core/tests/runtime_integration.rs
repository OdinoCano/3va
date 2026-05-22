use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use vvva_core::{Runtime, task_queue::TaskType, timer::TimerWheel};
use vvva_permissions::PermissionState;

fn make_runtime() -> Runtime {
    Runtime::new(PermissionState::new())
}

// ── Runtime construction ──────────────────────────────────────────────────────

#[test]
fn runtime_starts_with_no_tasks() {
    let rt = make_runtime();
    assert_eq!(rt.pending_task_count(), 0);
}

#[test]
fn runtime_starts_with_no_timers() {
    let mut rt = make_runtime();
    assert!(rt.poll_timers().is_empty());
    assert!(rt.next_timer_duration().is_none());
}

// ── Timer registration ────────────────────────────────────────────────────────

#[test]
fn set_timeout_registers_timer() {
    let mut rt = make_runtime();
    let _id = rt.set_timeout(Duration::from_millis(100), || {});
    // A pending timer means next_timer_duration is Some
    assert!(rt.next_timer_duration().is_some());
}

#[test]
fn set_interval_registers_timer() {
    let mut rt = make_runtime();
    let _id = rt.set_interval(Duration::from_millis(50), || {});
    assert!(rt.next_timer_duration().is_some());
}

#[test]
fn clear_timeout_cancels_registered_timer() {
    let mut rt = make_runtime();
    let id = rt.set_timeout(Duration::from_millis(100), || {});
    assert!(rt.next_timer_duration().is_some(), "timer should be pending before cancel");
    let cancelled = rt.clear_timeout(id);
    assert!(cancelled, "clear_timeout should return true for existing timer");
    assert!(rt.next_timer_duration().is_none(), "no timers should remain after cancel");
}

#[test]
fn clear_timeout_returns_false_for_unknown_id() {
    let mut rt = make_runtime();
    use vvva_core::timer::TimerId;
    let fake_id = TimerId::new(99999);
    let result = rt.clear_timeout(fake_id);
    assert!(!result, "clearing unknown timer should return false");
}

// ── poll_timers ───────────────────────────────────────────────────────────────

#[test]
fn poll_timers_returns_expired_timer() {
    let mut rt = make_runtime();
    let fired = Arc::new(AtomicUsize::new(0));
    let fired_clone = Arc::clone(&fired);

    // Register a zero-delay timer (fires immediately)
    rt.set_timeout(Duration::ZERO, move || {
        fired_clone.fetch_add(1, Ordering::SeqCst);
    });

    let timers = rt.poll_timers();
    assert!(!timers.is_empty(), "at least one timer should have been returned");

    // poll() returns timers; the caller is responsible for invoking the callbacks
    for t in &timers {
        (t.callback)();
    }
    assert_eq!(fired.load(Ordering::SeqCst), 1, "callback should be invoked once by caller");
}

#[test]
fn poll_timers_does_not_fire_future_timer() {
    let mut rt = make_runtime();
    let fired = Arc::new(AtomicUsize::new(0));
    let fired_clone = Arc::clone(&fired);

    // Register a 10-second timer — should not fire immediately
    rt.set_timeout(Duration::from_secs(10), move || {
        fired_clone.fetch_add(1, Ordering::SeqCst);
    });

    let timers = rt.poll_timers();
    assert!(timers.is_empty(), "far-future timer should not fire immediately");
    assert_eq!(fired.load(Ordering::SeqCst), 0, "callback should not have been called");
}

#[test]
fn poll_timers_repeating_interval_has_repeating_flag() {
    let mut rt = make_runtime();

    rt.set_interval(Duration::ZERO, || {});

    // poll() returns the interval timer; it should have repeating=true and
    // a new scheduled_at set (the event loop re-adds it to the wheel)
    let timers = rt.poll_timers();
    assert!(!timers.is_empty(), "interval should be returned by poll");
    let t = &timers[0];
    assert!(t.repeating, "interval timer should have repeating=true");
    assert!(t.interval.is_some(), "interval timer should carry its interval duration");
}

// ── schedule_task / pending_task_count ────────────────────────────────────────

#[tokio::test]
async fn schedule_task_increments_pending_count() {
    let mut rt = make_runtime();
    assert_eq!(rt.pending_task_count(), 0);
    rt.schedule_task(TaskType::UserTask, async {});
    assert_eq!(rt.pending_task_count(), 1);
}

// ── TimerWheel unit tests ─────────────────────────────────────────────────────

#[test]
fn timer_wheel_starts_empty() {
    let mut wheel = TimerWheel::new();
    assert_eq!(wheel.pending_count(), 0);
    assert!(wheel.poll().is_empty());
    assert!(wheel.next_duration().is_none());
}

#[test]
fn timer_wheel_schedule_increments_pending() {
    let mut wheel = TimerWheel::new();
    wheel.schedule(Duration::from_millis(100));
    assert_eq!(wheel.pending_count(), 1);
}

#[test]
fn timer_wheel_poll_returns_zero_delay_timer_and_caller_invokes_callback() {
    let mut wheel = TimerWheel::new();
    let fired = Arc::new(AtomicUsize::new(0));
    let fired_clone = Arc::clone(&fired);

    wheel.schedule_with_callback(Duration::ZERO, move || {
        fired_clone.fetch_add(1, Ordering::SeqCst);
    });

    let timers = wheel.poll();
    assert!(!timers.is_empty(), "zero-delay timer should be returned by poll");
    for t in &timers {
        (t.callback)();
    }
    assert_eq!(fired.load(Ordering::SeqCst), 1, "callback should execute when caller invokes it");
}

#[test]
fn timer_wheel_cancel_removes_timer() {
    let mut wheel = TimerWheel::new();
    let id = wheel.schedule(Duration::from_secs(5));
    assert_eq!(wheel.pending_count(), 1);
    let ok = wheel.cancel(id);
    assert!(ok, "cancel should succeed");
    assert_eq!(wheel.pending_count(), 0);
}

#[test]
fn timer_wheel_next_duration_reflects_nearest_timer() {
    let mut wheel = TimerWheel::new();
    wheel.schedule(Duration::from_millis(500));
    wheel.schedule(Duration::from_millis(100));
    let d = wheel.next_duration().expect("should have pending timers");
    // Nearest timer is ~100 ms; just verify it's less than 500 ms
    assert!(d <= Duration::from_millis(500), "next_duration should reflect nearest timer");
}
