pub mod framework;
pub mod matchers;
pub mod runner;

pub use framework::{describe, it, test, run_all_tests, expect, Expect, TestState, TestResult, TestStatus};
pub use matchers::{MatcherResult, Matchers};
pub use runner::{TestRunner, TestConfig, TestReporter, ReportFormat};

use std::path::PathBuf;

pub fn run_tests(paths: Vec<PathBuf>, config: Option<TestConfig>) -> anyhow::Result<Vec<TestResult>> {
    let cfg = config.unwrap_or_default();
    let mut runner = TestRunner::new(cfg);
    
    for path in paths {
        if path.is_file() {
            runner.run_file(&path)?;
        } else if path.is_dir() {
            runner.run_directory(&path)?;
        }
    }
    
    runner.print_summary();
    
    Ok(runner.get_results().clone())
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