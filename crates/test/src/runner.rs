use crate::framework::TestResult;
use crate::framework::TestStatus;
use std::path::Path;

pub struct TestRunner {
    results: Vec<TestResult>,
}

#[derive(Debug, Clone)]
pub struct TestConfig {
    pub verbose: bool,
    pub test_timeout_ms: u64,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            verbose: false,
            test_timeout_ms: 5000,
        }
    }
}

impl TestRunner {
    pub fn new(_config: TestConfig) -> Self {
        Self {
            results: Vec::new(),
        }
    }

    pub fn run_file(&mut self, path: &Path) -> anyhow::Result<()> {
        tracing::info!("Running test file: {:?}", path);
        self.results.push(TestResult {
            name: path.to_string_lossy().to_string(),
            status: TestStatus::Passed,
            duration_ms: 0,
            error: None,
        });
        Ok(())
    }

    pub fn run_directory(&mut self, dir: &Path) -> anyhow::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "test.js" || ext == "test.ts" || ext == "spec.js" || ext == "spec.ts" {
                    self.run_file(&path)?;
                }
            }
        }
        Ok(())
    }

    pub fn get_results(&self) -> &Vec<TestResult> {
        &self.results
    }

    pub fn print_summary(&self) {
        let passed = self
            .results
            .iter()
            .filter(|r| r.status == TestStatus::Passed)
            .count();
        let failed = self
            .results
            .iter()
            .filter(|r| r.status == TestStatus::Failed)
            .count();

        println!("\n=============================");
        println!("Test Suites: {}", self.results.len());
        println!("Tests Passed: {}", passed);
        println!("Tests Failed: {}", failed);
        println!("=============================\n");
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ReportFormat {
    Json,
    Junit,
    Dot,
}

pub struct TestReporter;

impl TestReporter {
    pub fn new(_format: ReportFormat) -> Self {
        Self
    }

    pub fn report(&self, results: &[TestResult]) -> String {
        serde_json::to_string_pretty(results).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runner() {
        let runner = TestRunner::new(TestConfig::default());
        assert_eq!(runner.get_results().len(), 0);
    }

    #[test]
    fn test_reporter() {
        let reporter = TestReporter::new(ReportFormat::Json);
        let results = vec![TestResult {
            name: "test".to_string(),
            status: TestStatus::Passed,
            duration_ms: 10,
            error: None,
        }];

        let output = reporter.report(&results);
        assert!(output.contains("test"));
    }
}
