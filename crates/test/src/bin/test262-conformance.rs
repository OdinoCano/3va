//! Aggregate test262 pass rate across `language/` + `built-ins/` + `intl402/`
//! — the same three trees `crates/test/tests/test262.rs` gates, aggregated
//! here into one number — printed as a `bench/run.sh`-style markdown section
//! so `bench/sync-readme.sh` can fill the `<!--BENCH:test262-3va-->` marker
//! in README.md's Comparison table with a CI-measured number, not a
//! hand-typed one. Deliberately excludes `annexB/` and `staging/`: neither
//! is part of that gate, and `staging/` holds draft-proposal tests that
//! aren't exercised anywhere else in this repo's test262 work.
//!
//! Usage: `test262-conformance [path/to/tests/test262]`

use std::path::Path;
use vvva_test::test262::run_suite;

fn main() {
    let root = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "tests/test262".to_string());
    let root = Path::new(&root);
    let root = if root.is_dir() {
        root.to_path_buf()
    } else {
        Path::new("../../tests/test262").to_path_buf()
    };
    if !root.join("harness").is_dir() {
        eprintln!(
            "{} not found — run scripts/setup-test262.sh first",
            root.display()
        );
        std::process::exit(1);
    }

    // Deliberately NOT subdir "" (which would also walk annexB/ and
    // staging/): those aren't part of the language/built-ins/intl402 gate
    // crates/test/tests/test262.rs verifies, and staging/ in particular
    // holds draft-proposal tests (e.g. staging/sm) never exercised against
    // this engine's $262.agent/createRealm — one hung this binary for over
    // an hour (a test blocked forever on Atomics.wait with no notify, most
    // likely) before this got scoped down. Same three trees, aggregated.
    //
    // Prints each subdir's own summary line to stderr as it finishes, not
    // just the aggregate at the very end: the full run takes long enough
    // (an hour-plus on a CI runner) that "is this actually still moving, or
    // stuck" needs a real answer from the log, not just an assumption.
    let mut summary = vvva_test::test262::Test262Summary::default();
    for subdir in ["language", "built-ins", "intl402"] {
        let start = std::time::Instant::now();
        let s = run_suite(&root, subdir, &["*"]);
        eprintln!(
            "[test262-conformance] {subdir}: {} passed, {} failed, {} skipped ({:.0}s)",
            s.passed,
            s.failed,
            s.skipped,
            start.elapsed().as_secs_f64()
        );
        summary.passed += s.passed;
        summary.failed += s.failed;
        summary.skipped += s.skipped;
    }
    let total = summary.passed + summary.failed;
    let pct = if total > 0 {
        summary.passed as f64 / total as f64 * 100.0
    } else {
        0.0
    };

    println!("## ECMAScript conformance (test262)");
    println!();
    println!("| Runtime | test262 pass rate |");
    println!("|---|---|");
    println!(
        "| 3va | {pct:.1}% ({}/{}, {} skipped) |",
        summary.passed, total, summary.skipped
    );
}
