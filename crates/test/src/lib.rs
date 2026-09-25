// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Test framework for 3va — runner, matchers, coverage, and security test utilities.

pub mod coverage;
pub mod framework;
pub mod runner;
pub mod security;
pub mod test262;

pub use coverage::{generate_coverage_report, print_coverage_report, CoverageReport};
pub use framework::{
    describe, expect, it, run_all_tests, test, Expect, TestResult, TestState, TestStatus,
};
pub use runner::{ReportFormat, TestConfig, TestReporter, TestRunner};

use std::path::PathBuf;

pub async fn run_tests(
    paths: Vec<PathBuf>,
    config: Option<TestConfig>,
) -> anyhow::Result<Vec<TestResult>> {
    let cfg = config.unwrap_or_default();
    let mut runner = TestRunner::new(cfg);

    for path in &paths {
        if path.is_file() {
            runner.run_file(path).await?;
        } else if path.is_dir() {
            runner.run_directory(path).await?;
        }
    }

    runner.print_summary();

    Ok(runner.get_results().clone())
}

/// Like [`run_tests`], but also returns statement coverage per source file
/// (canonical path) when `config.coverage_root` is set.
pub async fn run_tests_with_coverage(
    paths: Vec<PathBuf>,
    config: Option<TestConfig>,
) -> anyhow::Result<(
    Vec<TestResult>,
    std::collections::HashMap<PathBuf, coverage::CoverageResult>,
)> {
    let cfg = config.unwrap_or_default();
    let mut runner = TestRunner::new(cfg);
    for path in &paths {
        if path.is_file() {
            runner.run_file(path).await?;
        } else if path.is_dir() {
            runner.run_directory(path).await?;
        }
    }
    runner.print_summary();
    Ok((runner.get_results().clone(), runner.statement_coverage()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expect() {
        let e = expect(5);
        assert!(e.to_be(&5));
    }
}
