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
    /// Per-test resident-set limit in megabytes, the test and its descendants
    /// together. A test that goes over it is killed and fails; 0 disables it.
    pub max_rss_mb: u64,
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
/// * `--pending-deprecation` — the pending deprecation warnings are emitted.
///
/// The flags left out are the ones 3va cannot honor yet, and a test that
/// needs them is skipped with its reason instead of being run and failed:
/// `--expose-internals` and `--allow-natives-syntax` reach into Node's
/// internals, `--experimental-stream-iter` is an experimental Node API and
/// `--tls-min-v1.0` is below rustls' TLS 1.2 floor.
const SUPPORTED_FLAGS: &[&str] = &[
    "--no-warnings",
    "--expose-gc",
    "--expose_gc",
    "--zero-fill-buffers",
    "--insecure-http-parser",
    "--pending-deprecation",
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

/// The pids of the processes that descend from `pid`.
///
/// Linux keeps the list of children of each thread in
/// `/proc/<pid>/task/<tid>/children`. A test that forks a child which does
/// not end (and that leaves the test's process group) would stay alive after
/// the group is killed: the run of 2026-10-19 left 44 `3va run` processes of
/// `test-child-process-dgram-reuseport.js` behind, hanging on to the machine.
///
/// The list is read *while the test is still alive*: when it exits, its
/// children are re-parented and the list is empty.
fn descendants(pid: u32) -> Vec<SeenProcess> {
    let mut found = Vec::new();
    let mut queue = vec![pid];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(current) = queue.pop() {
        if !seen.insert(current) {
            continue;
        }
        let Ok(tasks) = std::fs::read_dir(format!("/proc/{current}/task")) else {
            continue;
        };
        for task in tasks.flatten() {
            let Ok(children) = std::fs::read_to_string(task.path().join("children")) else {
                continue;
            };
            for child in children.split_whitespace() {
                if let Ok(child) = child.parse::<u32>() {
                    if child > 1 {
                        queue.push(child);
                        // The moment it started is captured now, next to the
                        // pid: it is the only thing that tells this process
                        // from an unrelated one that reuses the number later.
                        if let Some(started) = start_time(child) {
                            found.push(SeenProcess {
                                pid: child,
                                started,
                            });
                        }
                    }
                }
            }
        }
    }
    found
}

/// A process the runner saw descending from the test, with the moment it
/// started: the pair is what makes it safe to signal later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SeenProcess {
    pid: u32,
    started: u64,
}

/// The moment a process started, in clock ticks since the boot (field 22 of
/// `/proc/<pid>/stat`).
///
/// It tells a process we saw from one that reused its number: a pid is only
/// ours while its start time is still the one we recorded.
fn start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(')')?;
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// Kill the processes we saw, if they are still the same ones.
///
/// The pid alone is not enough: with several tests running at once, an
/// unrelated process (from another worker or the desktop) can reuse the number
/// between the scan and the signal. Only when the current start time is still
/// the recorded one is the process ours.
#[allow(unsafe_code)]
fn kill_seen(procs: &[SeenProcess]) {
    for proc_ in procs {
        if proc_.pid <= 1 || proc_.started == 0 {
            continue;
        }
        if start_time(proc_.pid) != Some(proc_.started) {
            continue;
        }
        // Safety: `kill(2)` with a positive pid signals that process only.
        // The `pid > 1` guard keeps it away from init.
        unsafe {
            libc::kill(proc_.pid as i32, libc::SIGKILL);
        }
    }
}

/// Kill the processes that descend from `pid` and are still alive.
fn kill_descendants(pid: u32) {
    kill_seen(&descendants(pid));
}

/// Resident memory of a process, in kilobytes: `VmRSS` in `/proc/<pid>/status`.
///
/// It counts the pages the process actually has in RAM, not the virtual size a
/// `Buffer` reservation would report, so it is what the 1 GB per-test budget
/// is about. A process that exited reads as `None`.
fn rss_kb(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return rest.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

/// Resident memory of a test and the descendants the runner has seen, in
/// kilobytes.
///
/// The test itself is one process; a test that spawns children (or that leaves
/// them behind, the ones in `seen`) spends memory in each of them, so the
/// budget is per test, not per process.
fn group_rss_kb(pid: u32, seen: &[SeenProcess]) -> u64 {
    rss_kb(pid).unwrap_or(0) + seen.iter().filter_map(|p| rss_kb(p.pid)).sum::<u64>()
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
    stderr_dir: Option<&Path>,
) -> (bool, String) {
    let name = file
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    use std::os::unix::process::CommandExt;

    let stderr_path = stderr_dir.map(|dir| dir.join(format!("{name}.log")));
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
        .stdout(Stdio::null());
    // Own process group, so a test that leaves a detached child (fork/spawn)
    // does not leak it into the run: the whole group is killed afterwards.
    cmd.process_group(0);

    // The stderr of the test goes to a file: a pipe that nobody reads blocks
    // the test as soon as it writes more than the pipe buffer (64 KB), and a
    // thread per test does not fit in the task limit of the scope.
    let stderr_file = match &stderr_path {
        Some(path) => std::fs::File::create(path).ok(),
        None => None,
    };
    match stderr_file {
        Some(file) => {
            cmd.stderr(Stdio::from(file));
        }
        None => {
            cmd.stderr(Stdio::null());
        }
    }

    let mut child = match spawn_with_retry(&mut cmd) {
        Ok(c) => c,
        Err(e) => return (false, format!("spawn failed: {e}")),
    };
    let pid = child.id();
    // The descendants of the test, noted while it runs together with the
    // moment each one started: the ones that leave its process group survive
    // the group kill, and when the test exits its children are re-parented and
    // the list is gone.
    let mut orphans: Vec<SeenProcess> = Vec::new();
    let mut next_scan = Instant::now();
    // The stderr of the test is drained while it runs: nobody read the pipe,
    // so a test that writes more than the pipe buffer (64 KB) blocked
    // forever and the runner waited for it until the end of the session.

    // Wait for the test to finish *without* reaping it: while the child is a
    // zombie its pid is still reserved, so killing its process group below
    // cannot reach an unrelated process that reused the number (with several
    // tests running at once, that is not a rare accident: it killed other
    // tests' processes at random).
    let deadline = Instant::now() + opts.timeout;
    let mut timed_out = false;
    let mut memory_exceeded = false;
    loop {
        if is_zombie(pid) {
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            break;
        }
        if Instant::now() >= next_scan {
            next_scan = Instant::now() + Duration::from_millis(200);
            for seen in descendants(pid) {
                if !orphans.contains(&seen) {
                    orphans.push(seen);
                }
            }
            // A test that allocates without bound is a failure, not a reason
            // to raise the budget: it is measured while it runs because the
            // machine has to survive the run (six to fourteen `3va` at a time
            // used 27 GB on 2026-10-20).
            if opts.max_rss_mb > 0 && group_rss_kb(pid, &orphans) > opts.max_rss_mb * 1024 {
                memory_exceeded = true;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }

    // The whole group goes down: the test may have left children behind
    // (a detached spawn, a server that did not close). The child itself is
    // killed too, so `wait()` always returns: the group signal can fail
    // (the process is in a state that cannot be signalled) and a runner that
    // waits for ever is worse than a leaked process.
    let group_killed = kill_group(pid);
    // The descendants that left the group are killed by pid: the group signal
    // does not reach them, and they would keep the machine busy. The ones
    // that appeared after the last scan are caught here.
    kill_descendants(pid);
    kill_seen(&orphans);
    let _ = child.kill();
    let status = wait_bounded(&mut child, Duration::from_secs(2));
    if memory_exceeded {
        return (false, format!("memory limit {} MB", opts.max_rss_mb));
    }
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
    if let Some(path) = &stderr_path {
        if let Ok(bytes) = std::fs::read(path) {
            // Only the tail: the whole stderr of a test would fill the memory
            // of the runner.
            let start = bytes.len().saturating_sub(4096);
            let text = String::from_utf8_lossy(&bytes[start..]).trim().to_string();
            if !text.is_empty() {
                let text: String = text.chars().take(400).collect();
                reason.push_str(": ");
                reason.push_str(&text);
            }
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
    // One directory for the stderr of the tests; it is removed at the end of
    // the run.
    let stderr_dir = std::env::temp_dir().join(format!("3va-node-tests-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&stderr_dir);
    let stderr_dir = stderr_dir.is_dir().then_some(stderr_dir);
    let next = AtomicUsize::new(0);
    let outcomes: Mutex<Vec<Option<Outcome>>> =
        Mutex::new((0..planned.len()).map(|_| None).collect());

    let planned = &planned;
    let next = &next;
    let outcomes = &outcomes;
    std::thread::scope(|scope| {
        for worker in 0..jobs {
            let stderr_dir = stderr_dir.clone();
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
                            let (passed, reason) = run_one(
                                opts,
                                root,
                                &planned.file,
                                &planned.flags,
                                worker,
                                stderr_dir.as_deref(),
                            );
                            eprintln!(
                                "NODE_TEST_DONE\t{}\t{}\t{}",
                                planned.name,
                                if passed { "ok" } else { "fail" },
                                reason
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

    // The tests that hit the timeout are run again, one at a time: with
    // several tests running at the same time a slow one can miss the deadline
    // that it would meet alone, and a test that only fails under that
    // contention does not belong in the baseline (it would make the ratchet
    // fail at random, in the CI too).
    let timed_out: Vec<usize> = {
        let outcomes = outcomes.lock().expect("outcomes mutex");
        planned
            .iter()
            .enumerate()
            .filter_map(|(index, _)| match &outcomes[index] {
                Some(Outcome::Failed(reason)) if reason.starts_with("timeout after") => Some(index),
                _ => None,
            })
            .collect()
    };
    for index in timed_out {
        let planned = &planned[index];
        eprintln!("NODE_TEST_RETRY\t{}", planned.name);
        let (passed, reason) = run_one(
            opts,
            root,
            &planned.file,
            &planned.flags,
            0,
            stderr_dir.as_deref(),
        );
        eprintln!(
            "NODE_TEST_DONE\t{}\t{}\t{}",
            planned.name,
            if passed { "ok" } else { "fail" },
            reason
        );
        outcomes.lock().expect("outcomes mutex")[index] = Some(if passed {
            Outcome::Passed
        } else {
            Outcome::Failed(reason)
        });
    }

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
                let (passed, reason) = run_one(
                    opts,
                    root,
                    &planned.file,
                    &planned.flags,
                    0,
                    stderr_dir.as_deref(),
                );
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

    if let Some(dir) = &stderr_dir {
        let _ = std::fs::remove_dir_all(dir);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The two tests below look at the descendants of *this* process, so they
    /// cannot run at the same time: one of them kills the children of the
    /// other.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// A test that forks a child that never ends must not leave it alive.
    ///
    /// The child leaves the process group of the test (that is what the
    /// regression was about: 44 processes of `3va run` were still running
    /// fifteen minutes after the suite ended), so only the pids of the
    /// descendants reach it.
    #[test]
    fn the_descendants_of_a_test_are_killed() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // A child that does not end on its own.
        let mut child = std::process::Command::new("sleep")
            .arg("300")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("cannot spawn the child");
        let child_pid = child.id();
        let parent = std::process::id();

        // The child is a descendant of this process, with its start time.
        let found = descendants(parent);
        assert!(
            found.iter().any(|p| p.pid == child_pid),
            "el hijo debe aparecer entre los descendientes: {found:?}"
        );

        kill_seen(&[SeenProcess {
            pid: child_pid,
            started: start_time(child_pid).unwrap_or(0),
        }]);

        // The child is gone: a process that nobody reaped is a zombie, and
        // that is already dead.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if is_zombie(child_pid) || start_time(child_pid).is_none() {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                panic!("el hijo {child_pid} sigue vivo");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = child.wait();
    }

    /// A pid that was reused by an unrelated process must not be killed: the
    /// start time tells one from the other. The guard used to compare against
    /// the moment the test started, which a reused pid also passes; now the
    /// recorded start time has to match exactly.
    #[test]
    fn a_pid_that_is_not_the_one_we_saw_is_not_killed() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut child = std::process::Command::new("sleep")
            .arg("300")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("cannot spawn the child");
        let child_pid = child.id();
        // The pid was recorded earlier, with an older start time; the process
        // that wears it now started later. That is exactly a reused pid, and
        // the old rule ("it started after the test began") would kill it.
        let child_started = start_time(child_pid).unwrap_or(0);
        kill_seen(&[SeenProcess {
            pid: child_pid,
            started: child_started.saturating_sub(1),
        }]);
        // Give the signal a moment to arrive.
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            !is_zombie(child_pid) && start_time(child_pid).is_some(),
            "un proceso que empezó antes no debe recibir la señal"
        );
        let _ = child.kill();
        let _ = child.wait();
    }

    /// A test that allocates past the budget is killed and fails with the
    /// reason the run reports; the runner keeps running on a machine that has
    /// to survive a dozen tests at once.
    #[test]
    fn a_test_that_goes_over_the_memory_limit_is_killed() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("_3va_rss_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("hog.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\n\
             # 200 MB of resident memory, held until the runner kills the group.\n\
             a=$(head -c 200000000 /dev/zero | tr '\\0' x)\n\
             sleep 30\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let file = dir.join("test-hog.js");
        std::fs::write(&file, "// the script ignores it\n").unwrap();

        let opts = Options {
            prefixes: vec![],
            threeva_bin: script,
            timeout: Duration::from_secs(10),
            max_rss_mb: 32,
            perms: vec![],
            filter: None,
            skips: std::collections::BTreeMap::new(),
            jobs: 1,
        };
        let (passed, reason) = run_one(&opts, &dir, &file, &[], 0, None);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!passed, "un test que pasa de 32 MB no puede pasar");
        assert_eq!(reason, "memory limit 32 MB");
    }
}
