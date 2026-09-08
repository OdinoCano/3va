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

    let features: &[&str] = &["*"];
    let subdir = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "intl402".to_string());
    let summary = run_suite(&root, &subdir, features);
    println!(
        "test262/{subdir}: {} passed, {} failed, {} skipped",
        summary.passed, summary.failed, summary.skipped
    );
    for f in summary.failures.iter().take(80) {
        println!("FAIL {} — {}", f.name, f.error.as_deref().unwrap_or(""));
    }
    println!("total failures: {}", summary.failures.len());
}
