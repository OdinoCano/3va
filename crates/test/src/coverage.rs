//! Basic file-level coverage: which source files have corresponding test files,
//! and what percentage of test cases passed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::framework::{TestResult, TestStatus};

#[derive(Debug)]
pub struct FileCoverage {
    pub source_file: PathBuf,
    /// Corresponding test file, if found.
    pub test_file: Option<PathBuf>,
    pub tests_total: usize,
    pub tests_passed: usize,
    pub tests_failed: usize,
}

#[derive(Debug)]
pub struct CoverageReport {
    pub files: Vec<FileCoverage>,
    pub total_source_files: usize,
    pub covered_files: usize,
    pub total_tests: usize,
    pub passed_tests: usize,
}

/// Find all `.js` / `.ts` source files under `root`, excluding test/spec files
/// and common non-source directories.
fn collect_source_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_recursive(root, &mut out);
    out.sort();
    out
}

fn collect_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with('.') || name == "node_modules" || name == "dist" || name == "target" {
                continue;
            }
            collect_recursive(&path, out);
        } else {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let is_source = (name.ends_with(".js") || name.ends_with(".ts"))
                && !name.ends_with(".d.ts")
                && !name.ends_with(".test.js")
                && !name.ends_with(".test.ts")
                && !name.ends_with(".spec.js")
                && !name.ends_with(".spec.ts");
            if is_source {
                out.push(path);
            }
        }
    }
}

/// Guess the test file path for a source file.
/// `src/foo.ts` → tries `src/foo.test.ts`, `tests/foo.test.ts`, etc.
fn find_test_file(source: &Path) -> Option<PathBuf> {
    let stem = source.file_stem()?.to_string_lossy();
    let ext = source.extension()?.to_str()?;
    let dir = source.parent()?;

    // Same directory: foo.test.ts / foo.spec.ts
    for suffix in [".test", ".spec"] {
        let candidate = dir.join(format!("{}{}.{}", stem, suffix, ext));
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // Sibling `tests/` / `__tests__/` directory
    for test_dir in ["tests", "__tests__", "test"] {
        let candidate = dir.join(test_dir).join(format!("{}.test.{}", stem, ext));
        if candidate.exists() {
            return Some(candidate);
        }
        let candidate = dir.join(test_dir).join(format!("{}.spec.{}", stem, ext));
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // Up one level: foo.test.ts next to src/
    if let Some(parent) = dir.parent() {
        for test_dir in ["tests", "__tests__", "test"] {
            let candidate = parent.join(test_dir).join(format!("{}.test.{}", stem, ext));
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
}

/// Build a coverage report from test results and source files under `root`.
pub fn generate_coverage_report(
    test_results: &[TestResult],
    root: &Path,
) -> CoverageReport {
    let source_files = collect_source_files(root);

    // Map test file path -> results for that file
    let mut results_by_file: HashMap<String, Vec<&TestResult>> = HashMap::new();
    for result in test_results {
        // TestResult.name is either a test name or a file path (file-level errors)
        // Group by parent path heuristic: if the name looks like a path, use dirname
        let key = PathBuf::from(&result.name)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(&result.name))
            .to_string_lossy()
            .to_string();
        results_by_file.entry(key).or_default().push(result);
    }

    let mut files: Vec<FileCoverage> = Vec::new();
    let mut covered_files = 0usize;
    let mut total_tests = 0usize;
    let mut passed_tests = 0usize;

    for source in &source_files {
        let test_file = find_test_file(source);

        // Gather test results associated with this test file (if found)
        let (tests_total, tests_passed, tests_failed) = if let Some(ref tf) = test_file {
            let canon = tf.canonicalize().unwrap_or_else(|_| tf.clone());
            let key = canon.to_string_lossy().to_string();
            let results = results_by_file.get(&key).map(|v| v.as_slice()).unwrap_or(&[]);
            let total   = results.len();
            let passed  = results.iter().filter(|r| r.status == TestStatus::Passed).count();
            let failed  = results.iter().filter(|r| r.status == TestStatus::Failed).count();
            (total, passed, failed)
        } else {
            (0, 0, 0)
        };

        if test_file.is_some() {
            covered_files += 1;
        }
        total_tests  += tests_total;
        passed_tests += tests_passed;

        files.push(FileCoverage {
            source_file: source.clone(),
            test_file,
            tests_total,
            tests_passed,
            tests_failed,
        });
    }

    CoverageReport {
        total_source_files: source_files.len(),
        covered_files,
        total_tests,
        passed_tests,
        files,
    }
}

/// Print a human-readable coverage report to stdout.
pub fn print_coverage_report(report: &CoverageReport) {
    let file_coverage_pct = if report.total_source_files == 0 {
        100.0
    } else {
        report.covered_files as f64 / report.total_source_files as f64 * 100.0
    };
    let test_pass_pct = if report.total_tests == 0 {
        100.0
    } else {
        report.passed_tests as f64 / report.total_tests as f64 * 100.0
    };

    println!("\n=============================== Coverage ================================");
    println!();
    println!("  File coverage :  {}/{} files have tests ({:.1}%)",
        report.covered_files, report.total_source_files, file_coverage_pct);
    println!("  Test pass rate:  {}/{} tests passed ({:.1}%)",
        report.passed_tests, report.total_tests, test_pass_pct);
    println!();

    // Print per-file table
    println!("  {:<50} {:<20} {}", "Source file", "Test file", "Tests");
    println!("  {}", "-".repeat(80));

    for f in &report.files {
        let src = shorten_path(&f.source_file, 48);
        let test_indicator = match &f.test_file {
            Some(p) => shorten_path(p, 18),
            None => "(no test)".to_string(),
        };
        let stats = if f.tests_total > 0 {
            format!("{} pass / {} fail", f.tests_passed, f.tests_failed)
        } else if f.test_file.is_some() {
            "0 tests registered".to_string()
        } else {
            String::new()
        };
        let marker = if f.test_file.is_none() { "✗" } else if f.tests_failed > 0 { "!" } else { "✓" };
        println!("  {} {:<50} {:<20} {}", marker, src, test_indicator, stats);
    }

    println!();
    if report.covered_files < report.total_source_files {
        let uncovered = report.total_source_files - report.covered_files;
        eprintln!("  ! {} source file(s) have no test coverage.", uncovered);
    }
    println!("=========================================================================\n");
}

fn shorten_path(p: &Path, max_len: usize) -> String {
    let s = p.to_string_lossy();
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("...{}", &s[s.len().saturating_sub(max_len - 3)..])
    }
}
