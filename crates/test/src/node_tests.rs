// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Runner for nodejs/node's `test/parallel` suite (see scripts/setup-node-tests.sh).
//!
//! Unlike test262 (which runs in-process), each Node test is executed with the
//! `3va run` binary in its own process: they spawn children, open sockets, read
//! fixtures and set `process.exitCode`, so the observable contract is the
//! process exit code, exactly as `tools/test.py` uses.
//!
//! Tests that need a Node/V8 flag we do not implement (`// Flags: …`,
//! `--expose-internals`, …) or that reach into internals (`require('internal/…')`,
//! `internalBinding`) are **skipped with a reason**, listed separately, and count
//! neither as pass nor as failure. Everything else is a pass or a failure; the
//! failure list is compared against a versioned baseline (a ratchet) so a new
//! failure fails the run and a fixed one must be removed from the baseline.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One test that failed.
#[derive(Debug, Clone)]
pub struct Failed {
    pub name: String,
    pub reason: String,
}

/// One test skipped before running.
#[derive(Debug, Clone)]
pub struct Skipped {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ModuleCounts {
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
}

#[derive(Debug)]
pub struct Summary {
    pub passed: usize,
    pub failed: Vec<Failed>,
    pub skipped: Vec<Skipped>,
    pub by_module: BTreeMap<String, ModuleCounts>,
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Module prefixes to include, e.g. `["test-buffer-", "test-stream-"]`.
    pub prefixes: Vec<String>,
    /// 3va binary to run tests with.
    pub threeva_bin: PathBuf,
    /// Per-test wall-clock timeout.
    pub timeout: Duration,
    /// Permission flags passed to `3va run`.
    pub perms: Vec<String>,
    /// Run only files whose name contains this substring.
    pub filter: Option<String>,
    /// Tests to skip with a reason (name → reason), e.g. ones that allocate
    /// more memory than the run budget.
    pub skips: BTreeMap<String, String>,
    /// How many test files to run at the same time.
    pub jobs: usize,
}

/// The module key for a test file, from the longest matching prefix
/// (`test-buffer-alloc.js` with prefix `test-buffer-` → `buffer`).
fn module_of(name: &str, prefixes: &[String]) -> String {
    let mut best: Option<(&String, usize)> = None;
    for p in prefixes {
        if let Some(stripped) = name.strip_prefix(p.as_str()) {
            let _ = stripped;
            if best.map(|(_, l)| p.len() > l).unwrap_or(true) {
                best = Some((p, p.len()));
            }
        }
    }
    match best {
        Some((p, _)) => p
            .trim_start_matches("test-")
            .trim_end_matches('-')
            .to_string(),
        None => "other".to_string(),
    }
}

/// Node tests declare runner flags in a `// Flags:` line in the leading comment
/// block. The flags 3va understands are passed to `3va run`; any other flag
/// means we cannot reproduce the test faithfully, so it is skipped rather
/// than counted.
fn declared_flags(src: &str) -> Vec<String> {
    let mut flags = Vec::new();
    for line in src.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if !t.starts_with("//") {
            break;
        }
        if let Some(rest) = t.trim_start_matches('/').trim().strip_prefix("Flags:") {
            flags.extend(rest.split_whitespace().map(str::to_string));
        }
    }
    flags
}

/// The Node flags 3va honors, and can therefore hand to `3va run`.
///
/// * `--no-warnings` — silences the warning output (the `warning` event still
///   fires, like in Node).
/// * `--expose-gc` / `--expose_gc` — exposes `globalThis.gc()`.
/// * `--zero-fill-buffers` — `Buffer.allocUnsafe()` hands out zeroed memory.
/// * `--insecure-http-parser` — the lenient HTTP header parser.
///
/// The flags left out are the ones 3va cannot honor yet, and a test that
/// needs them is skipped with its reason instead of being run and failed:
/// `--expose-internals` and `--allow-natives-syntax` reach into Node's
/// internals, `--experimental-stream-iter` is an experimental Node API,
/// `--pending-deprecation` has nothing to activate (3va emits no pending
/// deprecations) and `--tls-min-v1.0` is below rustls' TLS 1.2 floor.
const SUPPORTED_FLAGS: &[&str] = &[
    "--no-warnings",
    "--expose-gc",
    "--expose_gc",
    "--zero-fill-buffers",
    "--insecure-http-parser",
];

/// The declared flags 3va can pass to `3va run`.
fn supported_flags(src: &str) -> Vec<String> {
    declared_flags(src)
        .into_iter()
        .filter(|f| SUPPORTED_FLAGS.contains(&f.as_str()))
        .collect()
}

/// Reason to skip a test before running it, if any.
fn skip_reason(name: &str, src: &str) -> Option<String> {
    let flags = declared_flags(src);
    let unsupported: Vec<String> = flags
        .iter()
        .filter(|f| !SUPPORTED_FLAGS.contains(&f.as_str()))
        .cloned()
        .collect();
    if !unsupported.is_empty() {
        return Some(format!("unsupported flags: {}", unsupported.join(" ")));
    }
    for needle in [
        "require('internal",
        "require(\"internal",
        "require('node:internal",
        "require(\"node:internal",
        "internalBinding(",
        "process.binding(",
    ] {
        if src.contains(needle) {
            return Some(format!("uses Node internals ({needle})"));
        }
    }
    if name.contains("worker") {
        // Worker tests need a module/worker surface that is out of scope here.
        return Some("worker_threads surface out of scope".to_string());
    }
    None
}

/// The port `test/common` uses for its own server (`common.PORT`). Each
/// parallel worker gets its own, so two tests never fight for it.
const COMMON_PORT_BASE: i64 = 12346;

/// Tests that run alone, after the parallel ones.
///
/// They spawn processes of their own (a `3va` inside a `3va`, a `python3`),
/// and with several of them at the same time the session runs out of
/// processes: the spawn fails with "Resource temporarily unavailable" and the
/// test fails for a reason that has nothing to do with what it checks. They
/// still count — they are only not run at the same time as another test.
const SEQUENTIAL_TESTS: &[&str] = &[
    "test-child-process-set-blocking.js",
    "test-child-process-spawnsync-non-string-args.js",
];

/// Spawn the command, waiting for the machine when it is momentarily out of
/// process slots.
///
/// The scope the suite runs in has a limit on how many processes it can have
/// at once (`TasksMax`). When many tests spawn children at the same time, the
/// limit is reached and `fork` is refused with `EAGAIN`. That is the machine
/// running out of room, not the test failing: wait a moment and try again
/// instead of counting it as a failure.
fn spawn_with_retry(cmd: &mut Command) -> std::io::Result<std::process::Child> {
    let mut last: Option<std::io::Error> = None;
    for attempt in 0..20u32 {
        match cmd.spawn() {
            Ok(child) => return Ok(child),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(50 * (attempt as u64 + 1)));
            }
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("spawn failed")))
}

/// Wait for the child, but not for ever.
///
/// A test that ignored the kill (or a process stuck in an uninterruptible
/// state) must not block the runner: after the deadline the child is left
/// behind and the run continues.
fn wait_bounded(
    child: &mut std::process::Child,
    limit: Duration,
) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Whether the process has exited and nobody has reaped it yet.
///
/// A zombie keeps its pid reserved, so the pid can be used to identify the
/// process group without the risk of hitting an unrelated process that
/// inherited the number after the child was reaped.
fn is_zombie(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    // "pid (comm) state ...": the command name can contain spaces and
    // parentheses, so the state is the first field after the last ')'.
    let Some((_, rest)) = stat.rsplit_once(')') else {
        return false;
    };
    rest.split_whitespace().next() == Some("Z")
}

/// Run one test file with a timeout, returning `(passed, reason)`.
///
/// `worker` is the index of the parallel worker running it: it decides the
/// port `test/common` uses and the temporary directory it works in, so two
/// tests never share either.
fn run_one(
    opts: &Options,
    root: &Path,
    file: &Path,
    flags: &[String],
    worker: usize,
) -> (bool, String) {
    use std::os::unix::process::CommandExt;

    let mut cmd = Command::new(&opts.threeva_bin);
    cmd.arg("run").arg("--no-prompt");
    for p in &opts.perms {
        cmd.arg(p);
    }
    for f in flags {
        cmd.arg(f);
    }
    cmd.arg(file)
        .current_dir(root)
        // The test's own helper (`test/common`) opens a server on this port;
        // one port per worker, so two tests never pick the same one.
        .env(
            "NODE_COMMON_PORT",
            (COMMON_PORT_BASE + 100 * worker as i64).to_string(),
        )
        // `test/common`'s temporary directory is named after this variable
        // (`.tmp.<TEST_THREAD_ID>`), so each worker gets its own — Node's own
        // runner does the same. Without it, two workers refreshing the
        // temporary directory delete each other's files.
        .env("TEST_THREAD_ID", worker.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Own process group, so a test that leaves a detached child (fork/spawn)
    // does not leak it into the run: the whole group is killed afterwards.
    cmd.process_group(0);

    let mut child = match spawn_with_retry(&mut cmd) {
        Ok(c) => c,
        Err(e) => return (false, format!("spawn failed: {e}")),
    };
    let pid = child.id();
    // The stderr of the test is drained while it runs: nobody read the pipe,
    // so a test that writes more than the pipe buffer (64 KB) blocked
    // forever and the runner waited for it until the end of the session.
    let stderr_tail = child.stderr.take().map(|mut pipe| {
        let tail = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let sink = tail.clone();
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 8192];
            loop {
                match pipe.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut sink = sink.lock().expect("stderr sink mutex");
                        sink.push_str(&String::from_utf8_lossy(&buf[..n]));
                        // Only the tail is kept: the whole stderr of a test
                        // would fill the memory of the runner.
                        if sink.len() > 8192 {
                            let cut = sink.len() - 8192;
                            sink.drain(..cut);
                        }
                    }
                }
            }
        });
        tail
    });

    // Wait for the test to finish *without* reaping it: while the child is a
    // zombie its pid is still reserved, so killing its process group below
    // cannot reach an unrelated process that reused the number (with several
    // tests running at once, that is not a rare accident: it killed other
    // tests' processes at random).
    let deadline = Instant::now() + opts.timeout;
    let mut timed_out = false;
    loop {
        if is_zombie(pid) {
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    // The whole group goes down: the test may have left children behind
    // (a detached spawn, a server that did not close). The child itself is
    // killed too, so `wait()` always returns: the group signal can fail
    // (the process is in a state that cannot be signalled) and a runner that
    // waits for ever is worse than a leaked process.
    let group_killed = kill_group(pid);
    let _ = child.kill();
    let status = wait_bounded(&mut child, Duration::from_secs(2));
    if timed_out {
        let mut reason = format!("timeout after {}s", opts.timeout.as_secs());
        if !group_killed {
            reason.push_str(" (the process group did not die)");
        }
        return (false, reason);
    }
    let Some(status) = status else {
        return (false, "wait failed".to_string());
    };
    if status.success() {
        return (true, String::new());
    }
    let mut reason = match status.code() {
        Some(c) => format!("exit {c}"),
        None => "killed by signal".to_string(),
    };
    if let Some(tail) = stderr_tail {
        let text = tail.lock().expect("stderr sink mutex").trim().to_string();
        if !text.is_empty() {
            let text: String = text.chars().take(400).collect();
            reason.push_str(": ");
            reason.push_str(&text);
        }
    }
    (false, reason)
}

/// Kill the process group `pgid` (the whole tree a test may have spawned).
///
/// The call goes straight to `kill(2)` with the negated pgid, as the rules of
/// the project ask: never `kill -KILL -<pgid>` without `--`, because procps-ng
/// parses that as `kill(-1, SIGKILL)` and kills every process of the user (it
/// took down the desktop session twice on 2026-10-16). The `pgid > 1` guard
/// keeps the target away from the caller's own group.
///
/// Returns whether the signal was delivered (a group that is already gone is
/// not an error: the test exited by itself).
#[allow(unsafe_code)]
fn kill_group(pgid: u32) -> bool {
    if pgid <= 1 {
        return false;
    }
    // Safety: `kill(2)` with a negative pid signals the process group `pgid`.
    // The guard above keeps it away from group 0 (the caller's group) and 1
    // (every process of the user), so the target is only the group the test
    // was given.
    let rc = unsafe { libc::kill(-(pgid as i32), libc::SIGKILL) };
    if rc == 0 {
        return true;
    }
    // ESRCH: the group is already gone (the test exited). Anything else: the
    // signal was not delivered.
    std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

/// Collect the test files matching the prefixes, sorted.
fn collect(root: &Path, opts: &Options) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let dir = root.join("test/parallel");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return files;
    };
    for e in entries.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if !name.ends_with(".js") {
            continue;
        }
        if !opts.prefixes.iter().any(|p| name.starts_with(p.as_str())) {
            continue;
        }
        if let Some(f) = &opts.filter {
            if !name.contains(f.as_str()) {
                continue;
            }
        }
        files.push(path);
    }
    files.sort();
    files
}

/// One test file, with everything decided before it runs.
struct Planned {
    name: String,
    file: PathBuf,
    module: String,
    /// The Node flags 3va honors, handed to `3va run`.
    flags: Vec<String>,
    /// Why the test is skipped, if it is.
    skip: Option<String>,
}

/// What a test ended up doing.
enum Outcome {
    Passed,
    Failed(String),
    Skipped(String),
}

/// Run the suite. `root` is the nodejs/node checkout (`tests/node`).
///
/// The files run in parallel (`opts.jobs` at a time), each in its own process
/// group with its own timeout, like Node's own runner does. The results are
/// collected per file and written out in the (sorted) file order, so a run
/// always produces the same lists, whatever the interleaving was.
pub fn run_suite(root: &Path, opts: &Options) -> Summary {
    let files = collect(root, opts);
    let planned: Vec<Planned> = files
        .iter()
        .map(|file| {
            let name = file
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let module = module_of(&name, &opts.prefixes);
            let src = std::fs::read_to_string(file).unwrap_or_default();
            let skip = opts
                .skips
                .get(&name)
                .cloned()
                .or_else(|| skip_reason(&name, &src));
            Planned {
                flags: supported_flags(&src),
                name,
                file: file.clone(),
                module,
                skip,
            }
        })
        .collect();

    let jobs = opts.jobs.max(1);
    let next = AtomicUsize::new(0);
    let outcomes: Mutex<Vec<Option<Outcome>>> =
        Mutex::new((0..planned.len()).map(|_| None).collect());

    let planned = &planned;
    let next = &next;
    let outcomes = &outcomes;
    std::thread::scope(|scope| {
        for worker in 0..jobs {
            scope.spawn(move || {
                // One port and one temporary directory per worker.
                loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(planned) = planned.get(index) else {
                        break;
                    };
                    if SEQUENTIAL_TESTS.contains(&planned.name.as_str()) {
                        // Run alone, after the parallel ones.
                        continue;
                    }
                    let outcome = match &planned.skip {
                        Some(reason) => Outcome::Skipped(reason.clone()),
                        None => {
                            // Printed BEFORE running: if a test kills the batch,
                            // the last START line names the culprit.
                            eprintln!("NODE_TEST_START\t{}", planned.name);
                            let (passed, reason) =
                                run_one(opts, root, &planned.file, &planned.flags, worker);
                            eprintln!(
                                "NODE_TEST_DONE\t{}\t{}",
                                planned.name,
                                if passed { "ok" } else { "fail" }
                            );
                            if passed {
                                Outcome::Passed
                            } else {
                                Outcome::Failed(reason)
                            }
                        }
                    };
                    outcomes.lock().expect("outcomes mutex")[index] = Some(outcome);
                }
            });
        }
    });

    // The tests that must run alone, one after the other, with the machine to
    // themselves.
    for (index, planned) in planned.iter().enumerate() {
        if !SEQUENTIAL_TESTS.contains(&planned.name.as_str()) {
            continue;
        }
        let outcome = match &planned.skip {
            Some(reason) => Outcome::Skipped(reason.clone()),
            None => {
                eprintln!("NODE_TEST_START\t{}", planned.name);
                let (passed, reason) = run_one(opts, root, &planned.file, &planned.flags, 0);
                eprintln!(
                    "NODE_TEST_DONE\t{}\t{}",
                    planned.name,
                    if passed { "ok" } else { "fail" }
                );
                if passed {
                    Outcome::Passed
                } else {
                    Outcome::Failed(reason)
                }
            }
        };
        outcomes.lock().expect("outcomes mutex")[index] = Some(outcome);
    }

    let mut summary = Summary {
        passed: 0,
        failed: Vec::new(),
        skipped: Vec::new(),
        by_module: BTreeMap::new(),
    };
    let outcomes = outcomes
        .lock()
        .expect("outcomes mutex")
        .drain(..)
        .collect::<Vec<_>>();
    for (planned, outcome) in planned.iter().zip(outcomes) {
        let entry = summary.by_module.entry(planned.module.clone()).or_default();
        match outcome {
            Some(Outcome::Passed) => {
                entry.passed += 1;
                summary.passed += 1;
            }
            Some(Outcome::Failed(reason)) => {
                entry.failed += 1;
                summary.failed.push(Failed {
                    name: planned.name.clone(),
                    reason,
                });
            }
            Some(Outcome::Skipped(reason)) => {
                entry.skipped += 1;
                summary.skipped.push(Skipped {
                    name: planned.name.clone(),
                    reason,
                });
            }
            None => {
                // The worker died before running this file; count it as a
                // failure instead of silently dropping it from the count.
                entry.failed += 1;
                summary.failed.push(Failed {
                    name: planned.name.clone(),
                    reason: "not run (the worker died)".to_string(),
                });
            }
        }
    }
    summary
}

/// Compare a run against the baseline: returns `(new_failures, fixed)`.
pub fn diff_baseline(baseline: &BTreeSet<String>, failed: &[Failed]) -> (Vec<String>, Vec<String>) {
    let now: BTreeSet<String> = failed.iter().map(|f| f.name.clone()).collect();
    let new_failures = now.difference(baseline).cloned().collect();
    let fixed = baseline.difference(&now).cloned().collect();
    (new_failures, fixed)
}

/// Read a baseline file (one test name per line, `#` comments and blanks ignored).
pub fn read_baseline(path: &Path) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    if let Ok(text) = std::fs::read_to_string(path) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            set.insert(line.to_string());
        }
    }
    set
}

/// Write a baseline file from a list of failing tests.
pub fn write_baseline(path: &Path, failed: &[Failed], header: &str) {
    let mut names: Vec<&str> = failed.iter().map(|f| f.name.as_str()).collect();
    names.sort();
    let mut out = String::new();
    out.push_str(header);
    out.push('\n');
    for n in names {
        out.push_str(n);
        out.push('\n');
    }
    std::fs::write(path, out).unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
}
