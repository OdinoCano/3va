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
/// block. Any flag means we cannot reproduce the test faithfully, so it is
/// skipped rather than counted.
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

/// Reason to skip a test before running it, if any.
fn skip_reason(name: &str, src: &str) -> Option<String> {
    let flags = declared_flags(src);
    if !flags.is_empty() {
        return Some(format!("unsupported flags: {}", flags.join(" ")));
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

/// Run one test file with a timeout, returning `(passed, reason)`.
fn run_one(opts: &Options, root: &Path, file: &Path) -> (bool, String) {
    use std::os::unix::process::CommandExt;

    let mut cmd = Command::new(&opts.threeva_bin);
    cmd.arg("run").arg("--no-prompt");
    for p in &opts.perms {
        cmd.arg(p);
    }
    cmd.arg(file)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    // Own process group, so a test that leaves a detached child (fork/spawn)
    // does not leak it into the run: the whole group is killed afterwards.
    cmd.process_group(0);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return (false, format!("spawn failed: {e}")),
    };

    let deadline = Instant::now() + opts.timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                kill_group(child.id());
                if status.success() {
                    return (true, String::new());
                }
                let reason = match status.code() {
                    Some(c) => format!("exit {c}"),
                    None => "killed by signal".to_string(),
                };
                return (false, reason);
            }
            Ok(None) => {}
            Err(e) => {
                kill_group(child.id());
                return (false, format!("wait failed: {e}"));
            }
        }
        if Instant::now() >= deadline {
            kill_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
            return (false, format!("timeout after {}s", opts.timeout.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// Kill the process group `pgid` (the whole tree a test may have spawned).
///
/// Never `kill -KILL -<pgid>` without `--`: procps-ng `kill` parses that as
/// `kill(-1, SIGKILL)` and kills every process of the user (it took down the
/// desktop session twice on 2026-10-16). Refuse pgids that would widen the target.
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

/// Run the suite. `root` is the nodejs/node checkout (`tests/node`).
pub fn run_suite(root: &Path, opts: &Options) -> Summary {
    let files = collect(root, opts);
    let mut summary = Summary {
        passed: 0,
        failed: Vec::new(),
        skipped: Vec::new(),
        by_module: BTreeMap::new(),
    };

    for file in files {
        let name = file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let module = module_of(&name, &opts.prefixes);
        let entry = summary.by_module.entry(module).or_default();

        let src = std::fs::read_to_string(&file).unwrap_or_default();
        if let Some(reason) = opts.skips.get(&name) {
            entry.skipped += 1;
            summary.skipped.push(Skipped {
                name,
                reason: reason.clone(),
            });
            continue;
        }
        if let Some(reason) = skip_reason(&name, &src) {
            entry.skipped += 1;
            summary.skipped.push(Skipped { name, reason });
            continue;
        }
        // Printed BEFORE running: if a test kills the batch, the last START
        // line names the culprit.
        eprintln!("NODE_TEST_START\t{name}");
        let (passed, reason) = run_one(opts, root, &file);
        eprintln!(
            "NODE_TEST_DONE\t{name}\t{}",
            if passed { "ok" } else { "fail" }
        );
        if passed {
            entry.passed += 1;
            summary.passed += 1;
        } else {
            entry.failed += 1;
            summary.failed.push(Failed { name, reason });
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
