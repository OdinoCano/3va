//! Statement-level JS/TS coverage via Oxc AST instrumentation.
//!
//! Design (Istanbul/nyc approach, implemented in Rust):
//!
//! 1. `instrument_source(source, file_id)` — parses with Oxc, collects every
//!    executable statement (including nested ones inside functions/if/loops),
//!    and inserts a hit counter `globalThis.__cov["file_id"][stmtIdx]++` before
//!    each one.  Counters are keyed by **statement index** (not line number) so
//!    single-line functions like `function f() { return 1; }` track the
//!    declaration and the return as separate, independently countable items.
//!
//! 2. The instrumented source is evaluated in the JS engine alongside the test.
//!    After all tests complete, the caller calls:
//!    `engine.eval_to_string("JSON.stringify(globalThis.__cov['file_id'] || {})")`
//!    and passes the resulting JSON to `parse_hit_counts`.
//!
//! 3. `CoverageResult::from_hits` cross-references the hit map with the
//!    `StmtInfo` list produced during instrumentation to report which statements
//!    (and their source lines) were executed vs. not.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use oxc_ast_visit::Visit;
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

use crate::framework::{TestResult, TestStatus};

// ── Public types ─────────────────────────────────────────────────────────────

/// Metadata for a single instrumented statement.
#[derive(Debug, Clone)]
pub struct StmtInfo {
    /// Sequential index used as the counter key inside `__cov["file_id"][idx]`.
    pub index: usize,
    /// 1-indexed source line where the statement begins.
    pub line: u32,
}

/// Coverage result for one file, produced after running the instrumented source.
#[derive(Debug, Clone)]
pub struct CoverageResult {
    pub total_statements: usize,
    pub covered_statements: usize,
    /// Every instrumented statement with its hit count.
    pub stmt_hits: Vec<(StmtInfo, u64)>,
    /// Statements never executed (hit_count == 0), sorted by line.
    pub uncovered_stmts: Vec<StmtInfo>,
}

impl CoverageResult {
    /// Build a result by pairing the instrumented-statement list with actual hit counts.
    pub fn from_hits(stmts: &[StmtInfo], hit_counts: &HashMap<usize, u64>) -> Self {
        let total = stmts.len();
        let mut stmt_hits: Vec<(StmtInfo, u64)> = Vec::with_capacity(total);
        let mut uncovered: Vec<StmtInfo> = Vec::new();

        for s in stmts {
            let hits = hit_counts.get(&s.index).copied().unwrap_or(0);
            if hits == 0 {
                uncovered.push(s.clone());
            }
            stmt_hits.push((s.clone(), hits));
        }

        let covered = total - uncovered.len();
        CoverageResult {
            total_statements: total,
            covered_statements: covered,
            stmt_hits,
            uncovered_stmts: uncovered,
        }
    }

    /// Percentage of statements covered (0.0–100.0).
    pub fn coverage_percent(&self) -> f64 {
        if self.total_statements == 0 {
            100.0
        } else {
            self.covered_statements as f64 / self.total_statements as f64 * 100.0
        }
    }

    /// Unique source lines that have at least one uncovered statement.
    pub fn uncovered_lines(&self) -> Vec<u32> {
        let mut lines: Vec<u32> = self.uncovered_stmts.iter().map(|s| s.line).collect();
        lines.sort_unstable();
        lines.dedup();
        lines
    }
}

// ── Instrumentation ──────────────────────────────────────────────────────────

/// Instrument `source` for coverage tracking.
///
/// Returns `(instrumented_source, stmt_infos)`.
/// `stmt_infos` describes every statement that received a counter; pass it
/// to `CoverageResult::from_hits` together with the collected hit counts.
///
/// Counter format injected before every executable statement:
/// ```js
/// globalThis.__cov["file_id"][idx]=(globalThis.__cov["file_id"][idx]||0)+1;
/// ```
pub fn instrument_source(source: &str, file_id: &str) -> (String, Vec<StmtInfo>) {
    let allocator = Allocator::default();
    let source_type = SourceType::mjs().with_typescript(true);
    let ret = Parser::new(&allocator, source, source_type).parse();

    // Collect every executable statement (recursive, preserving insertion order).
    let mut collector = StmtCollector::default();
    collector.visit_program(&ret.program);

    if collector.items.is_empty() {
        return (source.to_string(), Vec::new());
    }

    // Pre-compute newline positions for byte-offset → line-number mapping.
    let newlines: Vec<u32> = source
        .bytes()
        .enumerate()
        .filter(|(_, b)| *b == b'\n')
        .map(|(i, _)| i as u32)
        .collect();
    let offset_to_line = |off: u32| -> u32 { newlines.partition_point(|&nl| nl < off) as u32 + 1 };

    // Build StmtInfo list (index + line) ordered by byte offset.
    let safe_id = file_id.replace(['"', '\\'], "_");
    let mut items: Vec<(u32, usize)> = collector.items; // (byte_offset, index)
    items.sort_by_key(|&(off, _)| off);

    let stmt_infos: Vec<StmtInfo> = items
        .iter()
        .map(|&(off, idx)| StmtInfo {
            index: idx,
            line: offset_to_line(off),
        })
        .collect();

    // Preamble: initialise the counter objects once.
    let preamble = format!(
        r#"(globalThis.__cov=globalThis.__cov||{{}});(globalThis.__cov["{safe_id}"]=globalThis.__cov["{safe_id}"]||{{}});"#
    );

    // Insert counters in reverse byte-offset order so earlier offsets stay valid.
    let mut output = source.to_string();
    for &(offset, idx) in items.iter().rev() {
        let snippet = format!(
            r#"globalThis.__cov["{safe_id}"][{idx}]=(globalThis.__cov["{safe_id}"][{idx}]||0)+1;"#
        );
        output.insert_str(offset as usize, &snippet);
    }
    output.insert_str(0, &preamble);

    (output, stmt_infos)
}

/// Parse the JSON produced by
/// `JSON.stringify(globalThis.__cov["file_id"] || {})`.
///
/// Returns a map of `statement_index → hit_count`.
pub fn parse_hit_counts(json: &str) -> HashMap<usize, u64> {
    let mut map = HashMap::new();
    if let Ok(serde_json::Value::Object(obj)) = serde_json::from_str(json) {
        for (k, v) in obj {
            if let (Ok(idx), Some(hits)) = (k.parse::<usize>(), v.as_u64()) {
                map.insert(idx, hits);
            }
        }
    }
    map
}

// ── AST visitor ──────────────────────────────────────────────────────────────

#[derive(Default)]
struct StmtCollector {
    /// (byte_offset, sequential_index) for each executable statement.
    items: Vec<(u32, usize)>,
}

impl<'a> Visit<'a> for StmtCollector {
    fn visit_statements(&mut self, stmts: &oxc_allocator::Vec<'_, Statement<'_>>) {
        for stmt in stmts {
            let is_coverable = !matches!(
                stmt,
                Statement::ImportDeclaration(_)
                    | Statement::ExportAllDeclaration(_)
                    | Statement::TSTypeAliasDeclaration(_)
                    | Statement::TSInterfaceDeclaration(_)
                    | Statement::TSEnumDeclaration(_)
                    | Statement::TSModuleDeclaration(_)
                    | Statement::TSImportEqualsDeclaration(_)
            );
            if is_coverable {
                let idx = self.items.len();
                self.items.push((stmt.span().start, idx));
            }
            // Always recurse so nested statements (function bodies etc.) are collected.
            self.visit_statement(stmt);
        }
    }
}

// ── File-level summary (for the test runner's high-level report) ──────────────

#[derive(Debug)]
pub struct FileCoverage {
    pub source_file: PathBuf,
    pub test_file: Option<PathBuf>,
    pub tests_total: usize,
    pub tests_passed: usize,
    pub tests_failed: usize,
    /// Statement-level detail when instrumentation data is available.
    pub statement_coverage: Option<CoverageResult>,
}

#[derive(Debug)]
pub struct CoverageReport {
    pub files: Vec<FileCoverage>,
    pub total_source_files: usize,
    pub covered_files: usize,
    pub total_tests: usize,
    pub passed_tests: usize,
}

pub fn generate_coverage_report(test_results: &[TestResult], root: &Path) -> CoverageReport {
    let source_files = collect_source_files(root);

    let mut results_by_file: HashMap<String, Vec<&TestResult>> = HashMap::new();
    for result in test_results {
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

        let (tests_total, tests_passed, tests_failed) = if let Some(ref tf) = test_file {
            let canon = tf.canonicalize().unwrap_or_else(|_| tf.clone());
            let results = results_by_file
                .get(&canon.to_string_lossy().to_string())
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            let total = results.len();
            let passed = results
                .iter()
                .filter(|r| r.status == TestStatus::Passed)
                .count();
            let failed = results
                .iter()
                .filter(|r| r.status == TestStatus::Failed)
                .count();
            (total, passed, failed)
        } else {
            (0, 0, 0)
        };

        if test_file.is_some() {
            covered_files += 1;
        }
        total_tests += tests_total;
        passed_tests += tests_passed;

        files.push(FileCoverage {
            source_file: source.clone(),
            test_file,
            tests_total,
            tests_passed,
            tests_failed,
            statement_coverage: None,
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

pub fn print_coverage_report(report: &CoverageReport) {
    let file_pct = if report.total_source_files == 0 {
        100.0
    } else {
        report.covered_files as f64 / report.total_source_files as f64 * 100.0
    };
    let pass_pct = if report.total_tests == 0 {
        100.0
    } else {
        report.passed_tests as f64 / report.total_tests as f64 * 100.0
    };

    println!("\n=============================== Coverage ================================");
    println!(
        "  File coverage     :  {}/{} files have tests ({:.1}%)",
        report.covered_files, report.total_source_files, file_pct
    );
    println!(
        "  Test pass rate    :  {}/{} tests passed ({:.1}%)",
        report.passed_tests, report.total_tests, pass_pct
    );

    let stmt_files: Vec<&FileCoverage> = report
        .files
        .iter()
        .filter(|f| f.statement_coverage.is_some())
        .collect();
    if !stmt_files.is_empty() {
        let total_s: usize = stmt_files
            .iter()
            .filter_map(|f| f.statement_coverage.as_ref())
            .map(|c| c.total_statements)
            .sum();
        let cov_s: usize = stmt_files
            .iter()
            .filter_map(|f| f.statement_coverage.as_ref())
            .map(|c| c.covered_statements)
            .sum();
        let pct = if total_s == 0 {
            100.0
        } else {
            cov_s as f64 / total_s as f64 * 100.0
        };
        println!("  Statement coverage:  {cov_s}/{total_s} statements executed ({pct:.1}%)");
    }

    println!();
    println!("  {:<50} {:<20} Tests", "Source file", "Test file");
    println!("  {}", "-".repeat(80));

    for f in &report.files {
        let src = shorten_path(&f.source_file, 48);
        let test_indicator = f
            .test_file
            .as_deref()
            .map(|p| shorten_path(p, 18))
            .unwrap_or_else(|| "(no test)".into());
        let stats = if let Some(cov) = &f.statement_coverage {
            format!(
                "{}/{} stmts ({:.0}%)",
                cov.covered_statements,
                cov.total_statements,
                cov.coverage_percent()
            )
        } else if f.tests_total > 0 {
            format!("{} pass / {} fail", f.tests_passed, f.tests_failed)
        } else if f.test_file.is_some() {
            "0 tests registered".into()
        } else {
            String::new()
        };
        let marker = if f.test_file.is_none() {
            "✗"
        } else if f.tests_failed > 0
            || f.statement_coverage
                .as_ref()
                .is_some_and(|c| !c.uncovered_stmts.is_empty())
        {
            "!"
        } else {
            "✓"
        };
        println!("  {} {:<50} {:<20} {}", marker, src, test_indicator, stats);

        if let Some(cov) = &f.statement_coverage {
            let lines = cov.uncovered_lines();
            if !lines.is_empty() {
                println!(
                    "       Uncovered lines: {}",
                    lines
                        .iter()
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
        }
    }

    println!();
    if report.covered_files < report.total_source_files {
        let n = report.total_source_files - report.covered_files;
        eprintln!("  ! {n} source file(s) have no test coverage.");
    }
    println!("=========================================================================\n");
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn collect_source_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_recursive(root, &mut out);
    out.sort();
    out
}

fn collect_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with('.') || matches!(name.as_ref(), "node_modules" | "dist" | "target")
            {
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

fn find_test_file(source: &Path) -> Option<PathBuf> {
    let stem = source.file_stem()?.to_string_lossy();
    let ext = source.extension()?.to_str()?;
    let dir = source.parent()?;

    for suffix in [".test", ".spec"] {
        let c = dir.join(format!("{}{}.{}", stem, suffix, ext));
        if c.exists() {
            return Some(c);
        }
    }
    for td in ["tests", "__tests__", "test"] {
        for suffix in [".test", ".spec"] {
            let c = dir.join(td).join(format!("{}{}.{}", stem, suffix, ext));
            if c.exists() {
                return Some(c);
            }
        }
    }
    if let Some(parent) = dir.parent() {
        for td in ["tests", "__tests__", "test"] {
            let c = parent.join(td).join(format!("{}.test.{}", stem, ext));
            if c.exists() {
                return Some(c);
            }
        }
    }
    None
}

fn shorten_path(p: &Path, max_len: usize) -> String {
    let s = p.to_string_lossy();
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("...{}", &s[s.len().saturating_sub(max_len - 3)..])
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use vvva_js::JsEngine;
    use vvva_permissions::PermissionState;

    async fn bare_engine() -> JsEngine {
        JsEngine::new(Arc::new(PermissionState::new()))
            .await
            .unwrap()
    }

    async fn run_and_collect(source: &str, file_id: &str) -> CoverageResult {
        let (instrumented, stmts) = instrument_source(source, file_id);
        let mut engine = bare_engine().await;
        engine
            .eval(&instrumented)
            .await
            .expect("instrumented code must evaluate");
        let json = engine
            .eval_to_string(&format!(
                r#"JSON.stringify(globalThis.__cov["{file_id}"] || {{}})"#
            ))
            .await
            .unwrap();
        let hits = parse_hit_counts(&json);
        CoverageResult::from_hits(&stmts, &hits)
    }

    // ── instrument_source ────────────────────────────────────────────────────

    #[tokio::test]
    async fn instrument_injects_counters_and_produces_stmt_info() {
        let source = "const x = 1;\nconst y = 2;\nconst z = x + y;";
        let (instrumented, stmts) = instrument_source(source, "test_file");

        assert_eq!(stmts.len(), 3, "three variable declarations");
        assert!(instrumented.contains("__cov"), "must reference __cov");
        assert!(instrumented.contains("test_file"), "must include file id");
        assert!(instrumented.contains("const x = 1"));
    }

    #[tokio::test]
    async fn instrument_covers_nested_function_body_as_separate_stmt() {
        // Both the function declaration AND the return are instrumented independently.
        let source = "function add(a, b) {\n  return a + b;\n}";
        let (_instrumented, stmts) = instrument_source(source, "fn_test");
        assert!(
            stmts.len() >= 2,
            "expected ≥2 stmts (decl + return), got {}",
            stmts.len()
        );
    }

    #[tokio::test]
    async fn instrument_empty_source_returns_empty_stmts() {
        let (_, stmts) = instrument_source("", "empty");
        assert_eq!(stmts.len(), 0);
    }

    // ── Full round-trip ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn coverage_roundtrip_all_statements_executed() {
        let result =
            run_and_collect("const a = 1;\nconst b = 2;\nconst c = a + b;", "rt_all").await;
        assert_eq!(
            result.covered_statements, result.total_statements,
            "all three top-level statements should be hit"
        );
        assert!(result.uncovered_stmts.is_empty());
        assert!((result.coverage_percent() - 100.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn coverage_roundtrip_branch_not_taken_shows_uncovered() {
        let result = run_and_collect(
            "const x = 1;\nif (false) {\n  const dead = 2;\n}\nconst live = 3;",
            "rt_branch",
        )
        .await;
        assert!(
            !result.uncovered_stmts.is_empty(),
            "dead branch must be uncovered; covered={}/{}, uncovered lines={:?}",
            result.covered_statements,
            result.total_statements,
            result.uncovered_lines()
        );
        assert!(result.coverage_percent() < 100.0);
    }

    #[tokio::test]
    async fn coverage_roundtrip_unused_function_body_is_uncovered() {
        // The function declarations are hoisted (covered), but the body of `unused`
        // is only executed on call — so it must appear uncovered.
        let result = run_and_collect(
            "function used() {\n  return 1;\n}\nfunction unused() {\n  return 2;\n}\nused();",
            "rt_fn",
        )
        .await;
        assert!(
            result.covered_statements < result.total_statements,
            "unused() body must be uncovered; covered={}/{}, uncovered lines={:?}",
            result.covered_statements,
            result.total_statements,
            result.uncovered_lines()
        );
    }

    #[tokio::test]
    async fn instrument_typescript_types_not_in_coverable_stmt_list() {
        // TS-only constructs (interface, type alias) must not appear in the
        // coverable-statement list because they generate no executable code.
        // We check instrumentation only — evaluation requires a transpiler.
        let source = "interface Foo { x: number; }\ntype Bar = string;\nconst z = 1;";
        let (_instrumented, stmts) = instrument_source(source, "ts_types");
        // Only `const z = 1` should be coverable.
        assert_eq!(
            stmts.len(),
            1,
            "only the const stmt is executable, got {:?}",
            stmts
        );
        assert_eq!(stmts[0].line, 3);
    }

    #[tokio::test]
    async fn coverage_roundtrip_js_only_variant() {
        // Equivalent of the TS test using plain JS (no transpiler needed).
        // Simulates the runtime behaviour: only the live const is executable.
        let result = run_and_collect("const z = 1;", "rt_js_only").await;
        assert_eq!(result.total_statements, 1);
        assert_eq!(result.covered_statements, 1);
    }

    // ── parse_hit_counts ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn parse_hit_counts_reads_json_correctly() {
        let hits = parse_hit_counts(r#"{"0":3,"4":1,"9":0}"#);
        assert_eq!(hits[&0], 3);
        assert_eq!(hits[&4], 1);
        assert_eq!(hits[&9], 0);
    }

    #[tokio::test]
    async fn parse_hit_counts_empty_and_invalid_json() {
        assert!(parse_hit_counts("{}").is_empty());
        assert!(parse_hit_counts("not json").is_empty());
    }

    // ── CoverageResult ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn coverage_result_percent_is_correct() {
        let stmts = vec![
            StmtInfo { index: 0, line: 1 },
            StmtInfo { index: 1, line: 2 },
            StmtInfo { index: 2, line: 3 },
            StmtInfo { index: 3, line: 4 },
        ];
        let mut hits = HashMap::new();
        hits.insert(0usize, 2u64);
        hits.insert(2, 1);
        let result = CoverageResult::from_hits(&stmts, &hits);

        assert_eq!(result.total_statements, 4);
        assert_eq!(result.covered_statements, 2);
        assert!((result.coverage_percent() - 50.0).abs() < 0.01);
        assert_eq!(result.uncovered_lines(), vec![2, 4]);
    }

    #[tokio::test]
    async fn coverage_result_empty_is_100_percent() {
        let r = CoverageResult::from_hits(&[], &HashMap::new());
        assert_eq!(r.coverage_percent(), 100.0);
    }
}
