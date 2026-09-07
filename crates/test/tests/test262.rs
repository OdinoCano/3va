//! tc39/test262 conformance run. Requires `scripts/setup-test262.sh` first
//! (~70k tests, so this is `#[ignore]`d — opt in with `cargo test --ignored test262`).
//! Run a narrower slice while a feature is unsupported, e.g.:
//!   cargo test --ignored test262 -- language/expressions

use std::path::Path;
use vvva_test::test262::run_suite;

// ponytail: features 3va doesn't implement yet — trims the suite to what should pass.
// Extend as engine coverage grows; "*" would disable feature filtering entirely.
const SUPPORTED_FEATURES: &[&str] = &["*"];

#[test]
#[ignore]
fn test262_language_and_builtins() {
    let root = Path::new("../../tests/test262");
    if !root.join("harness").is_dir() {
        eprintln!("tests/test262 not found — run scripts/setup-test262.sh first, skipping");
        return;
    }

    let mut non_module_failed = 0;
    let mut module_failed = 0;
    for subdir in ["language", "built-ins"] {
        let summary = run_suite(root, subdir, SUPPORTED_FEATURES);
        println!(
            "test262/{subdir}: {} passed, {} failed, {} skipped",
            summary.passed, summary.failed, summary.skipped
        );

        // `flags: [module]` tests run through the CJS transpile pipeline
        // (`transpile_to_cjs` in `vvva_js`), so module-record-only semantics
        // (early errors, live/indirect bindings, namespace exotic objects,
        // top-level await, source-phase/attribute loading) legitimately fail.
        // They are reported, not hidden — but kept separate from the
        // non-module baseline so a module-record gap isn't mistaken for a
        // regression in the plain-script engine.
        for f in summary.failures.iter().take(50) {
            println!("  FAIL {} — {}", f.name, f.error.as_deref().unwrap_or(""));
        }
        for f in &summary.failures {
            if f.name.contains("/module-code/") {
                module_failed += 1;
            } else {
                non_module_failed += 1;
            }
        }
    }

    println!(
        "module tests failing (expected — CJS transpile cannot express module records): {module_failed}"
    );

    // ponytail: aspirational. 3va's plain-script engine still has documented
    // gaps (direct-eval `arguments` bindings, destructuring evaluation order,
    // strict-mode syntax checks) so this stays red until those are closed;
    // the gate exists to catch *new* non-module regressions, not to claim the
    // suite is green.
    assert_eq!(
        non_module_failed, 0,
        "non-module test262 regressions found (see output above)"
    );
}
