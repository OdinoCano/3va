// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// SIGTERM handling, end to end with the real binary.
//
// Node disables the default action of a signal when the script registers a
// listener: the application decides when to exit. 3va installs its own signal
// handlers, so it needs a watchdog for the case the script cannot answer (a
// busy loop that never gives the event loop a turn) — but the watchdog must
// not cut a shutdown the application is implementing.
//
//   1. a busy loop that ignores SIGTERM dies with the conventional status;
//   2. a SIGTERM handler that drains for 8 s exits when *it* ends, not at 5 s;
//   3. with no handler, SIGTERM takes the default action.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_3va")
}

fn run_script(dir: &Path, src: &str) -> Child {
    let script = dir.join("main.js");
    fs::write(&script, src).unwrap();
    let home = dir.join("home");
    fs::create_dir_all(&home).unwrap();
    Command::new(bin())
        .arg("run")
        .arg("--no-prompt")
        .arg(&script)
        .env("HOME", &home)
        .spawn()
        .expect("cannot start 3va")
}

/// Send SIGTERM to `pid` with `kill`, as a supervisor (systemd, Kubernetes)
/// does.
fn send_sigterm(pid: u32) {
    let status = Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .expect("cannot run kill");
    assert!(status.success(), "kill -TERM {pid} failed");
}

/// Send SIGINT to `pid`, the signal a second Ctrl+C produces.
fn send_sigint(pid: u32) {
    let status = Command::new("kill")
        .arg("-INT")
        .arg(pid.to_string())
        .status()
        .expect("cannot run kill");
    assert!(status.success(), "kill -INT {pid} failed");
}

/// Wait for the child to exit, killing it if it takes longer than `limit`.
fn wait_bounded(child: &mut Child, limit: Duration) -> (ExitStatus, Duration) {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait failed") {
            return (status, started.elapsed());
        }
        if started.elapsed() >= limit {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the process was still alive after {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn temp_dir(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    fs::create_dir_all(&path).unwrap();
    (dir, path)
}

/// A script that never yields cannot run its handler; the watchdog ends it
/// with 128 + 15 so a supervisor sees the same status as with Node.
#[test]
fn a_busy_loop_dies_with_sigterm() {
    let (_dir, path) = temp_dir("busy");
    let mut child = run_script(&path, "while (true) {}\n");
    std::thread::sleep(Duration::from_secs(1));
    send_sigterm(child.id());
    let (status, elapsed) = wait_bounded(&mut child, Duration::from_secs(15));
    assert_eq!(status.code(), Some(143), "status: {status:?}");
    assert!(
        elapsed >= Duration::from_secs(4) && elapsed < Duration::from_secs(12),
        "the watchdog fired at {elapsed:?}"
    );
}

/// With a listener, Node keeps the loop alive and the application decides
/// when to stop: an 8 s drain must not be cut at 5 s.
#[test]
fn a_sigterm_handler_drains_for_as_long_as_it_needs() {
    let (_dir, path) = temp_dir("drain");
    let mut child = run_script(
        &path,
        r#"
        process.on('SIGTERM', () => {
            setTimeout(() => { process.exit(0); }, 8000);
        });
        setInterval(() => {}, 1000);
        "#,
    );
    std::thread::sleep(Duration::from_secs(1));
    let started = Instant::now();
    send_sigterm(child.id());
    let (status, elapsed) = wait_bounded(&mut child, Duration::from_secs(20));
    assert_eq!(status.code(), Some(0), "status: {status:?}");
    assert!(
        started.elapsed() >= Duration::from_secs(7),
        "the drain was cut after {elapsed:?}"
    );
}

/// Without a listener the signal takes the default action, promptly.
#[test]
fn sigterm_without_a_listener_exits_with_128_plus_the_signal() {
    let (_dir, path) = temp_dir("default");
    let mut child = run_script(&path, "setInterval(() => {}, 1000);\n");
    std::thread::sleep(Duration::from_secs(1));
    send_sigterm(child.id());
    let (status, elapsed) = wait_bounded(&mut child, Duration::from_secs(5));
    assert_eq!(status.code(), Some(143), "status: {status:?}");
    assert!(
        elapsed < Duration::from_secs(4),
        "the exit took {elapsed:?}"
    );
}

/// Every signal reaches the handler, not just the first: a second Ctrl+C (the
/// one a person sends to cut a stuck shutdown) must emit `SIGINT` again. Node
/// re-emits the event for each signal.
#[test]
fn a_handler_receives_every_sigint() {
    let (_dir, path) = temp_dir("repeat");
    let mut child = run_script(
        &path,
        r#"
        let n = 0;
        process.on('SIGINT', () => {
            n += 1;
            if (n >= 2) process.exit(0);
        });
        setInterval(() => {}, 1000);
        "#,
    );
    std::thread::sleep(Duration::from_secs(1));
    send_sigint(child.id());
    std::thread::sleep(Duration::from_millis(300));
    send_sigint(child.id());
    // The handler counts two signals and exits; if the second is lost the
    // process keeps running and the watchdog would not help (the first signal
    // was already acknowledged).
    let (status, _elapsed) = wait_bounded(&mut child, Duration::from_secs(10));
    assert_eq!(status.code(), Some(0), "status: {status:?}");
}
