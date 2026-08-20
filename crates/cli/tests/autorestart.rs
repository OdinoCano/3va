// End-to-end tests for the supervised process manager's automatic restart on
// crash. These drive the real `3va` binary and kill real processes:
//
//   1. autorestart_on_exit_1           — the app calls process.exit(1)
//   2. autorestart_on_sigkill          — the app is killed with SIGKILL (kill -9)
//   3. autorestart_on_real_panic       — the app aborts via native code (SIGABRT,
//                                        the same signal a Rust panic-abort raises)
//   4. no_autorestart_marks_error      — --no-autorestart never respawns
//   5. restart_counter_accumulates     — manual `3va restart` keeps counting
//   6. permissions_preserved_on_restart — package.json grants survive a restart
//   7. deny_by_default_still_applies   — ungranted capabilities stay denied
//
// Every test isolates `~/.3va` via its own HOME so parallel runs never share
// state, and cleans up its managed process on drop.

use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_3va")
}

#[derive(Debug, Deserialize)]
struct Status {
    pid: u32,
    status: String,
    restarts: u32,
}

/// Per-test sandbox: an isolated project dir and HOME (`~/.3va` lives there).
struct Harness {
    root: tempfile::TempDir,
    home: PathBuf,
    name: String,
}

impl Harness {
    fn new(name: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().join("home");
        fs::create_dir_all(&home).unwrap();
        Harness {
            root,
            home,
            name: name.to_string(),
        }
    }

    fn write_script(&self, file: &str, body: &str) {
        fs::write(self.root.path().join(file), body).unwrap();
    }

    /// Write a package.json carrying the given `3va.permissions` root scope
    /// grants (empty string → no grants, deny-by-default).
    fn write_pkg(&self, root_grants: &str) {
        let grants = if root_grants.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str::<serde_json::Value>(root_grants).unwrap()
        };
        let pkg = serde_json::json!({
            "name": self.name,
            "version": "1.0.0",
            "main": "index.js",
            "3va": { "permissions": { ".": grants } }
        });
        fs::write(
            self.root.path().join("package.json"),
            serde_json::to_string_pretty(&pkg).unwrap(),
        )
        .unwrap();
    }

    fn run(&self, cwd: &Path, args: &[&str]) -> std::process::Output {
        Command::new(bin())
            .args(args)
            .current_dir(cwd)
            .env("HOME", &self.home)
            .output()
            .unwrap_or_else(|e| panic!("failed to run {args:?}: {e}"))
    }

    fn start(&self, script: &str, extra: &[&str]) {
        let mut args: Vec<&str> = vec!["start", script, "--name", &self.name];
        args.extend_from_slice(extra);
        let out = self.run(self.root.path(), &args);
        assert!(
            out.status.success(),
            "`3va start` failed: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn restart(&self) {
        let out = self.run(self.root.path(), &["restart", &self.name]);
        assert!(
            out.status.success(),
            "`3va restart` failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn status_json(&self) -> Option<Status> {
        let path = self
            .home
            .join(".3va/processes")
            .join(format!("{}.json", self.name));
        let raw = fs::read_to_string(path).ok()?;
        serde_json::from_str(&raw).ok()
    }

    fn log(&self) -> String {
        let path = self
            .home
            .join(".3va/processes")
            .join(format!("{}.log", self.name));
        fs::read_to_string(&path).unwrap_or_default()
    }

    fn worker_pids(&self) -> Vec<u32> {
        self.log()
            .lines()
            .filter_map(|l| {
                l.split("worker pid=")
                    .nth(1)?
                    .split_whitespace()
                    .next()?
                    .parse()
                    .ok()
            })
            .collect()
    }

    /// Poll `cond` every 200 ms until it returns true or `timeout` elapses.
    fn wait_for(&self, timeout: Duration, cond: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        let mut cond = cond;
        loop {
            if cond() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Wait for `cond` to hold; panic with context on timeout.
    fn wait_for_or(&self, timeout: Duration, ctx: &str, cond: impl FnMut() -> bool) {
        assert!(
            self.wait_for(timeout, cond),
            "timed out waiting for: {ctx}\n--- log ---\n{}",
            self.log()
        );
    }

    fn cleanup(&self) {
        let _ = self.run(self.root.path(), &["delete", &self.name]);
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.cleanup();
    }
}

#[cfg(unix)]
fn kill9(pid: u32) {
    let st = Command::new("kill")
        .args(["-9", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    assert!(st.is_ok() && st.unwrap().success(), "kill -9 {pid} failed");
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Locate the C library so the panic test can call a real `abort()` through
/// FFI. Returns None on images without a known path → test skips.
#[cfg(unix)]
fn find_libc() -> Option<&'static str> {
    [
        "/lib/x86_64-linux-gnu/libc.so.6",
        "/usr/lib/x86_64-linux-gnu/libc.so.6",
        "/lib64/libc.so.6",
        "/lib/aarch64-linux-gnu/libc.so.6",
    ]
    .into_iter()
    .find(|p| Path::new(p).exists())
}

// ── 1. process.exit(1) ────────────────────────────────────────────────────

#[test]
fn autorestart_on_exit_1() {
    let h = Harness::new("exit1");
    h.write_script(
        "crash.js",
        "setTimeout(()=>{ console.log('exiting'); process.exit(1); }, 1000);\n",
    );
    h.start("crash.js", &["--max-restarts", "10"]);

    h.wait_for_or(
        Duration::from_secs(20),
        "restart after process.exit(1)",
        || h.status_json().map(|s| s.restarts >= 1).unwrap_or(false),
    );
    let st = h.status_json().unwrap();
    assert_eq!(
        st.status, "running",
        "must still be supervised after a restart"
    );
    assert!(st.restarts >= 1, "must have restarted at least once");
}

// ── 2. SIGKILL (kill -9) ──────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn autorestart_on_sigkill() {
    let h = Harness::new("sigkill");
    h.write_script(
        "long.js",
        "console.log('worker pid=' + process.pid); setInterval(()=>{}, 1000);\n",
    );
    h.start("long.js", &["--max-restarts", "10"]);

    h.wait_for_or(Duration::from_secs(15), "first worker to come up", || {
        !h.worker_pids().is_empty()
    });
    let first = *h.worker_pids().last().unwrap();
    assert!(
        pid_alive(first),
        "worker {first} should be alive before the kill"
    );

    kill9(first);
    h.wait_for_or(
        Duration::from_secs(15),
        "supervisor to restart after SIGKILL",
        || h.status_json().map(|s| s.restarts >= 1).unwrap_or(false),
    );
    h.wait_for_or(
        Duration::from_secs(15),
        "a NEW worker pid after SIGKILL",
        || h.worker_pids().iter().any(|&p| p != first),
    );
    let st = h.status_json().unwrap();
    assert_eq!(st.status, "running");
}

// ── 2b. `3va stop` never triggers a respawn ───────────────────────────────

#[cfg(unix)]
#[test]
fn stop_never_triggers_autorestart() {
    let h = Harness::new("stop");
    h.write_script(
        "long.js",
        "console.log('worker pid=' + process.pid); setInterval(()=>{}, 1000);\n",
    );
    h.start("long.js", &["--max-restarts", "10"]);

    h.wait_for_or(Duration::from_secs(15), "first worker to come up", || {
        !h.worker_pids().is_empty()
    });
    let worker = *h.worker_pids().last().unwrap();

    let out = h.run(h.root.path(), &["stop", &h.name]);
    assert!(out.status.success(), "`3va stop` failed");

    h.wait_for_or(
        Duration::from_secs(15),
        "status `stopped` after stop",
        || {
            h.status_json()
                .map(|s| s.status == "stopped")
                .unwrap_or(false)
        },
    );
    let st = h.status_json().unwrap();
    assert_eq!(st.restarts, 0, "a clean stop must never count as a restart");
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        !pid_alive(worker),
        "the worker must stay dead after `3va stop` — no respawn"
    );
}

// ── 3. real panic (SIGABRT via native abort()) ────────────────────────────

#[cfg(unix)]
#[test]
fn autorestart_on_real_panic() {
    let Some(libc) = find_libc() else {
        eprintln!("skipping: no known libc path on this image");
        return;
    };
    let h = Harness::new("panic");
    // The abort() call is granted via package.json (root scope), so a restart
    // must re-apply the exact same grants for the test to keep crashing.
    h.write_pkg(&format!(r#"{{"allow-ffi":[{libc:?}]}}"#));
    h.write_script(
        "panic.js",
        &format!(
            "setTimeout(()=>{{ console.log('aborting'); const f=require('ffi').dlopen({libc:?},{{abort:{{returns:'void',args:[]}}}}); f.symbols.abort(); }}, 1000);\n"
        ),
    );
    h.start("panic.js", &["--max-restarts", "10"]);

    h.wait_for_or(
        Duration::from_secs(20),
        "restart after a real panic/SIGABRT",
        || h.status_json().map(|s| s.restarts >= 1).unwrap_or(false),
    );
    let st = h.status_json().unwrap();
    assert_eq!(st.status, "running");
}

// ── 4. --no-autorestart never respawns ────────────────────────────────────

#[test]
fn no_autorestart_marks_error_and_does_not_restart() {
    let h = Harness::new("nore");
    h.write_script(
        "crash.js",
        "setTimeout(()=>{ console.log('exiting'); process.exit(1); }, 1000);\n",
    );
    h.start("crash.js", &["--no-autorestart"]);

    h.wait_for_or(Duration::from_secs(20), "status to become `error`", || {
        h.status_json()
            .map(|s| s.status == "error")
            .unwrap_or(false)
    });
    let st = h.status_json().unwrap();
    assert_eq!(st.restarts, 0, "--no-autorestart must never respawn");
    // The status file is written just before the supervisor process exits, so
    // there's a brief window after `status` flips to `error` where the PID
    // can still be alive (process teardown, not yet reaped) — poll instead of
    // a single immediate check to avoid flaking on slower/noisier CI runners.
    h.wait_for_or(
        Duration::from_secs(5),
        "supervisor to exit after a no-autorestart crash",
        || !pid_alive(st.pid),
    );
}

// ── 5. restart counter survives manual `3va restart` ──────────────────────

#[test]
fn restart_counter_accumulates_across_manual_restart() {
    let h = Harness::new("cnt");
    h.write_script(
        "crash.js",
        "setTimeout(()=>{ console.log('exiting'); process.exit(1); }, 1000);\n",
    );
    h.start("crash.js", &["--max-restarts", "100"]);

    h.wait_for_or(Duration::from_secs(20), "2 automatic restarts", || {
        h.status_json().map(|s| s.restarts >= 2).unwrap_or(false)
    });
    let before = h.status_json().unwrap().restarts;

    h.restart();

    // The new supervisor must continue from `before + 1`, never reset to 0.
    h.wait_for_or(
        Duration::from_secs(15),
        "counter > before after manual restart",
        || {
            h.status_json()
                .map(|s| s.restarts > before)
                .unwrap_or(false)
        },
    );
    let after = h.status_json().unwrap().restarts;
    assert!(
        after > before,
        "manual restart must keep counting (before={before}, after={after})"
    );
}

// ── 6. permissions survive an automatic restart ───────────────────────────

#[cfg(unix)]
#[test]
fn permissions_preserved_on_autorestart() {
    let h = Harness::new("perm");
    let root = h.root.path().to_string_lossy().to_string();
    let tick_file = h
        .root
        .path()
        .join("ticks.log")
        .to_string_lossy()
        .to_string();
    // Only a scoped write grant for the project dir — everything else stays
    // deny-by-default, including on the restarted instance.
    h.write_pkg(&format!(r#"{{"allow-write":[{root:?}]}}"#));
    h.write_script(
        "writer.js",
        &format!(
            "const fs=require('fs'); fs.appendFileSync({tick_file:?}, 'tick\\n'); console.log('worker pid=' + process.pid); setInterval(()=>{{}}, 1000);\n"
        ),
    );
    h.start("writer.js", &["--max-restarts", "10"]);

    h.wait_for_or(Duration::from_secs(15), "first granted write", || {
        fs::read_to_string(&tick_file)
            .map(|s| s.lines().count() >= 1)
            .unwrap_or(false)
    });
    let first = *h.worker_pids().last().unwrap();
    kill9(first);

    h.wait_for_or(
        Duration::from_secs(20),
        "restarted worker's granted write (permission preserved)",
        || {
            fs::read_to_string(&tick_file)
                .map(|s| s.lines().count() >= 2)
                .unwrap_or(false)
        },
    );
    let st = h.status_json().unwrap();
    assert!(st.restarts >= 1, "worker should have been restarted");
    assert_eq!(st.status, "running");
}

// ── 7. deny-by-default still applies (negative control for #6) ────────────

#[test]
fn deny_by_default_still_applies() {
    let h = Harness::new("deny");
    h.write_pkg(""); // no grants at all
    let out_dir = h.root.path().join("out").to_string_lossy().to_string();
    h.write_script(
        "writer.js",
        &format!("const fs=require('fs'); fs.writeFileSync({out_dir:?}, 'x');\n"),
    );
    let out = h.run(h.root.path(), &["run", "writer.js"]);
    assert!(
        !out.status.success(),
        "an ungranted write must be denied: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("denied") || combined.contains("EACCES"),
        "denial must mention EACCES/denied, got: {combined}"
    );
}
