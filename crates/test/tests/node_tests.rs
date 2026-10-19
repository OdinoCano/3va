// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! nodejs/node `test/parallel` compatibility run with a baseline ratchet.
//!
//! Requires `scripts/setup-node-tests.sh` first (~4k tests; `#[ignore]`d — opt
//! in with `cargo test --ignored node_parallel_suite`). The failure baseline
//! lives in `crates/test/tests/node_tests_baseline.txt`: a new failure fails the
//! run, and a test that starts passing must be removed from the baseline (so
//! progress is recorded). Set `NODE_TESTS_BASELINE_OUT` to regenerate it.
//!
//! Run a slice while iterating:
//!   NODE_TESTS_FILTER=buffer cargo test --ignored node_parallel_suite
//!
//! See docs/09-testing/07-compatibilidad-node.md (§7.4 step 2).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use vvva_test::node_tests::{
    diff_baseline, read_baseline, run_suite, write_baseline, Options, Summary,
};

// ponytail: the initial scope from the standard (§7.4 step 2). The `test/http/
// https/net/stream` server tests hang under 3va and dominate the runtime
// (~44 min total on a debug build), so the weekly deep-check is the right
// place for them; the rest of the prefixes run in a few minutes.
const PREFIXES: &[&str] = &[
    "test-buffer-",
    "test-child-process-",
    "test-crypto-",
    "test-events-",
    "test-fs-",
    "test-http-",
    "test-https-",
    "test-net-",
    "test-os-",
    "test-path-",
    "test-querystring-",
    "test-stream-",
    "test-string-decoder-",
    "test-timers-",
    "test-url-",
    "test-util-",
    "test-zlib-",
];

const BASELINE: &str = "tests/node_tests_baseline.txt";
const SKIP_LIST: &str = "tests/node_tests_skip.txt";

/// Read `name<TAB>reason` skip lines (see node_tests_skip.txt).
fn read_skips(path: &Path) -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    if let Ok(text) = std::fs::read_to_string(path) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (name, reason) = line
                .split_once('\t')
                .unwrap_or((line, "listed in node_tests_skip.txt"));
            map.insert(name.to_string(), reason.to_string());
        }
    }
    map
}

fn threeva_bin() -> PathBuf {
    if let Ok(p) = std::env::var("THREEVA_BIN") {
        return PathBuf::from(p);
    }
    // The debug build first: the suite is the ratchet of the Node
    // compatibility and it is measured with the debug build (the same one CI
    // uses, see .github/workflows/deep-checks.yml). A `target/release/3va`
    // left over from an older commit would silently measure the wrong code.
    for candidate in ["../../target/debug/3va", "../../target/release/3va"] {
        let p = Path::new(candidate);
        if p.exists() {
            return p.to_path_buf();
        }
    }
    PathBuf::from("3va")
}

fn print_summary(summary: &Summary) {
    println!(
        "node test/parallel: {} passed, {} failed, {} skipped",
        summary.passed,
        summary.failed.len(),
        summary.skipped.len()
    );
    println!(
        "{:<22} {:>7} {:>7} {:>7}  compat",
        "module", "pass", "fail", "skip"
    );
    for (module, c) in &summary.by_module {
        let denom = c.passed + c.failed;
        let pct = if denom == 0 {
            0.0
        } else {
            c.passed as f64 * 100.0 / denom as f64
        };
        println!(
            "{:<22} {:>7} {:>7} {:>7}  {:.1}%",
            module, c.passed, c.failed, c.skipped, pct
        );
    }
}

#[test]
#[ignore]
fn node_parallel_suite() {
    let root = Path::new("../../tests/node");
    if !root.join("test/parallel").is_dir() {
        eprintln!("tests/node not found — run scripts/setup-node-tests.sh first, skipping");
        return;
    }

    let bin = threeva_bin();
    let opts = Options {
        prefixes: PREFIXES.iter().map(|s| s.to_string()).collect(),
        threeva_bin: bin.clone(),
        // 10 s by default: a Node test that works finishes in a second or two, and a hung one
        // used to cost the full 30 s (a third of the http chunk's run time). Override with
        // NODE_TESTS_TIMEOUT_SECS for a slow machine or a final baseline run.
        timeout: Duration::from_secs(
            std::env::var("NODE_TESTS_TIMEOUT_SECS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
        ),
        perms: [
            "--allow-read",
            "--allow-write",
            "--allow-net",
            "--allow-child-process",
            "--allow-env",
            "--allow-ffi",
            "--allow-insecure",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        filter: std::env::var("NODE_TESTS_FILTER").ok(),
        skips: read_skips(Path::new(SKIP_LIST)),
        // How many test files run at the same time. Half the cores by
        // default (the run is mostly waiting on timeouts), 16 at most.
        jobs: std::env::var("NODE_TESTS_JOBS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| {
                let cores = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(2);
                (cores / 2).clamp(1, 16)
            }),
    };

    println!("3va: {}", bin.display());
    let summary = run_suite(root, &opts);
    print_summary(&summary);

    // All failures, for diffing two runs.
    if let Ok(path) = std::env::var("NODE_TESTS_FAILURES_OUT") {
        let mut names: Vec<String> = summary.failed.iter().map(|f| f.name.clone()).collect();
        names.sort();
        std::fs::write(&path, names.join("\n") + "\n")
            .unwrap_or_else(|e| panic!("cannot write {path}: {e}"));
    }
    // Skipped tests, with their reasons.
    if let Ok(path) = std::env::var("NODE_TESTS_SKIPPED_OUT") {
        let mut lines: Vec<String> = summary
            .skipped
            .iter()
            .map(|s| format!("{}\t{}", s.name, s.reason))
            .collect();
        lines.sort();
        std::fs::write(&path, lines.join("\n") + "\n")
            .unwrap_or_else(|e| panic!("cannot write {path}: {e}"));
    }
    // Per-module counts, for the measured compatibility table
    // (scripts/compat-table.sh).
    if let Ok(path) = std::env::var("NODE_TESTS_COUNTS_OUT") {
        let mut lines = Vec::new();
        for (module, c) in &summary.by_module {
            lines.push(format!(
                "{module}\t{}\t{}\t{}",
                c.passed, c.failed, c.skipped
            ));
        }
        std::fs::write(&path, lines.join("\n") + "\n")
            .unwrap_or_else(|e| panic!("cannot write {path}: {e}"));
    }

    // Regenerating the baseline is explicit and never a silent pass.
    if let Ok(path) = std::env::var("NODE_TESTS_BASELINE_OUT") {
        let header = "# nodejs/node test/parallel known failures (ratchet).\n\
                      # Regenerate with NODE_TESTS_BASELINE_OUT; see docs/09-testing/07-compatibilidad-node.md.\n\
                      # nodejs/node commit: see tests/node/COMMIT. One test file per line.";
        write_baseline(Path::new(&path), &summary.failed, header);
        println!(
            "baseline written to {path} ({} failures)",
            summary.failed.len()
        );
        return;
    }

    let baseline: BTreeSet<String> = read_baseline(Path::new(BASELINE));
    let (new_failures, fixed) = diff_baseline(&baseline, &summary.failed);

    if !new_failures.is_empty() {
        println!("\nnew failures (regressions) — {}:", new_failures.len());
        for n in &new_failures {
            let reason = summary
                .failed
                .iter()
                .find(|f| &f.name == n)
                .map(|f| f.reason.clone())
                .unwrap_or_default();
            println!("  FAIL {n}  ({reason})");
        }
    }
    if !fixed.is_empty() {
        println!("\nfixed (remove from {BASELINE}) — {}:", fixed.len());
        for n in &fixed {
            println!("  PASS {n}");
        }
    }

    assert!(
        new_failures.is_empty(),
        "{} tests newly fail versus the baseline in {BASELINE}. A regression, or update the \
         baseline deliberately with NODE_TESTS_BASELINE_OUT.",
        new_failures.len()
    );
    assert!(
        fixed.is_empty(),
        "{} tests now pass but are still in {BASELINE}; remove them so the ratchet records \
         the progress.",
        fixed.len()
    );
}
