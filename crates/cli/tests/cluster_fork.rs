// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// Regression tests for cluster/child_process.fork. Both used to turn any test
// that forks into a process bomb:
//
//   - `cluster.isPrimary`/`isWorker` were computed once while the builtins
//     snapshot was recorded (empty environment) and froze `isPrimary = true`
//     for every process, so a worker thought it was the primary and forked
//     again; `cluster.fork(env)` also dropped the `env` argument;
//   - `child_process.fork(path, args)` dropped `args`, so a child that branches
//     on `process.argv[2]` re-ran as its own parent and forked again.
//
// These tests drive the real `3va` binary. To be safe they use a fixture whose
// depth is tracked in a FILE (so the guard works even when the bug is a
// missing env var) and a tight process-group kill: at most two processes can
// ever be created, whatever regresses.
//
// Run: cargo test -p vvva_cli --test cluster_fork

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_3va")
}

// ponytail: the fixture carries the hard depth guard. `LIMIT` reports the
// cluster flags at the depth cap so the assertions can read them.
const FIXTURE: &str = r#"
'use strict';
const cluster = require('cluster');
const { fork } = require('child_process');
const fs = require('fs');

const DEPTH_FILE = process.env.DEPTH_FILE;
const MODE = process.env.MODE || 'cluster';

let depth = 0;
try { depth = parseInt(fs.readFileSync(DEPTH_FILE, 'utf8'), 10) || 0; } catch (e) {}
fs.writeFileSync(DEPTH_FILE, String(depth + 1));

// A child_process.fork child receives its extra argv, so it identifies itself
// and exits before it could ever fork again. If argv were dropped (the
// regression), it falls through to the LIMIT guard below and exits without
// spawning.
if (process.argv[2] === 'child') {
  console.log('CP-CHILD argv2=' + JSON.stringify(process.argv[2]) + ' FLAG=' + process.env.FLAG);
  process.exit(0);
}

// Hard cap: never spawn past depth 1, whatever isPrimary/env say.
if (depth >= 1) {
  console.log('LIMIT isPrimary=' + cluster.isPrimary + ' isWorker=' + cluster.isWorker +
              ' argv2=' + JSON.stringify(process.argv[2]) + ' FLAG=' + process.env.FLAG);
  process.exit(0);
}

if (MODE === 'cluster') {
  if (cluster.isPrimary) {
    console.log('PRIMARY isPrimary=' + cluster.isPrimary);
    cluster.fork({ FLAG: 'passed' });
    setTimeout(() => process.exit(0), 1000);
  } else {
    console.log('WORKER isWorker=' + cluster.isWorker + ' FLAG=' + process.env.FLAG);
    process.exit(0);
  }
} else {
  if (process.argv[2] === 'child') {
    console.log('CP-CHILD argv2=' + JSON.stringify(process.argv[2]) + ' FLAG=' + process.env.FLAG);
    process.exit(0);
  }
  console.log('CP-PARENT');
  const cp = fork(process.argv[1], ['child'], { env: Object.assign({}, process.env, { FLAG: 'passed' }) });
  cp.on('exit', () => { console.log('CP-EXIT'); process.exit(0); });
  setTimeout(() => process.exit(3), 2000);
}
"#;

/// Run the fixture in `mode`, bounded by a timeout, and return its stdout. The
/// child runs in its own process group, which is killed whole afterwards.
fn run_fixture(mode: &str, timeout: Duration) -> String {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fork_fixture.js");
    std::fs::write(&script, FIXTURE).unwrap();
    let depth_file = dir.path().join("depth.txt");

    let mut cmd = Command::new(bin());
    cmd.arg("run")
        .arg("--no-prompt")
        .arg("--allow-read")
        .arg("--allow-write")
        .arg("--allow-child-process")
        .arg("--allow-env")
        .arg(&script)
        .current_dir(dir.path())
        .env("MODE", mode)
        .env("DEPTH_FILE", &depth_file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.process_group(0);

    let mut child = cmd.spawn().expect("spawn 3va");
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().unwrap() {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                kill_group(child.id());
                break;
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let mut out = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    // The forked child's stdout is the IPC channel: the parent echoes its
    // non-JSON lines to its own stderr prefixed with `[worker-log]`, so the
    // worker's reports are only visible there.
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = e.read_to_string(&mut err);
    }
    let combined = format!("{out}\n{err}");
    // Kill any lingering child in the group, whatever happened above.
    kill_group(child.id());

    // A depth guard means at most two processes; if a regression ever slipped
    // past it we would see LIMIT lines repeating. Fail loudly instead of
    // hammering the machine.
    let depth = std::fs::read_to_string(&depth_file).unwrap_or_default();
    assert!(
        depth.trim().parse::<u32>().unwrap_or(0) <= 2,
        "fork depth exceeded 2 — a process bomb started; output:\n{combined}"
    );
    combined
}

#[test]
fn cluster_worker_is_not_primary_and_receives_env() {
    let out = run_fixture("cluster", Duration::from_secs(25));
    assert!(
        out.contains("PRIMARY isPrimary=true"),
        "no primary line:\n{out}"
    );
    assert!(
        out.contains("isPrimary=false") && out.contains("isWorker=true"),
        "the forked cluster child must be a worker, not a second primary:\n{out}"
    );
    assert!(
        out.contains("FLAG=passed"),
        "cluster.fork({{ FLAG }}) must pass the env to the worker:\n{out}"
    );
}

#[test]
fn child_process_fork_passes_args() {
    let out = run_fixture("cp", Duration::from_secs(25));
    assert!(
        out.contains("CP-CHILD argv2=\"child\""),
        "child_process.fork(path, args) must pass args to the child:\n{out}"
    );
    assert!(
        out.contains("FLAG=passed"),
        "the env must reach the child:\n{out}"
    );
    // NOTE: the forked ChildProcess does not emit 'exit'/'close' yet, so the
    // parent stops via its own timeout instead of `cp.on('exit')`. That is a
    // separate gap, not the bomb this test guards against.
}

// Keep the unused-import warning away when the file is compiled alone.
#[allow(dead_code)]
fn _unused(_: &Path) {}

/// Kill the process group `pgid`. Never `kill -KILL -<pgid>` without `--`:
/// procps-ng `kill` parses that as `kill(-1, SIGKILL)` and kills every process
/// of the user (it took down the desktop session on 2026-10-16).
fn kill_group(pgid: u32) {
    if pgid <= 1 {
        return;
    }
    let _ = Command::new("kill")
        .arg("-KILL")
        .arg("--")
        .arg(format!("-{pgid}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}
