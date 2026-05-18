use serde::{Serialize, Deserialize};

pub type TestFn = Box<dyn Fn() + Send + Sync>;

pub struct TestCase {
    pub name: String,
    pub fn_name: String,
    pub test_fn: TestFn,
}

impl TestCase {
    pub fn new(name: String, fn_name: String, test_fn: TestFn) -> Self {
        Self { name, fn_name, test_fn }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub name: String,
    pub status: TestStatus,
    pub duration_ms: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TestStatus {
    Passed,
    Failed,
    Skipped,
    Pending,
}

impl Default for TestStatus {
    fn default() -> Self { TestStatus::Pending }
}

pub struct TestState {
    root: Vec<TestCase>,
    results: Vec<TestResult>,
}

impl TestState {
    pub fn new() -> Self {
        Self { root: Vec::new(), results: Vec::new() }
    }

    pub fn it(&mut self, name: String, test_fn: TestFn) {
        self.root.push(TestCase::new(name.clone(), format!("it: {}", name), test_fn));
    }

    pub fn test(&mut self, name: String, test_fn: TestFn) {
        self.root.push(TestCase::new(name.clone(), format!("test: {}", name), test_fn));
    }

    pub fn run_all(&mut self) -> Vec<TestResult> {
        for test in &self.root {
            let start = std::time::Instant::now();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                (test.test_fn)();
            }));
            let duration = start.elapsed().as_millis() as u64;

            let status = match result {
                Ok(_) => TestStatus::Passed,
                Err(_) => TestStatus::Failed,
            };

            self.results.push(TestResult {
                name: test.name.clone(),
                status,
                duration_ms: duration,
                error: None,
            });
        }
        self.results.clone()
    }
}

lazy_static::lazy_static! {
    static ref STATE: std::sync::Mutex<TestState> = std::sync::Mutex::new(TestState::new());
}

pub fn describe(_name: &str, _f: impl FnOnce()) {}

pub fn it(name: String, test_fn: TestFn) {
    STATE.lock().unwrap().it(name, test_fn);
}

pub fn test(name: String, test_fn: TestFn) {
    STATE.lock().unwrap().test(name, test_fn);
}

pub fn run_all_tests() -> Vec<TestResult> {
    STATE.lock().unwrap().run_all()
}

pub fn expect<T: std::fmt::Debug + 'static>(actual: T) -> Expect<T> {
    Expect { actual }
}

pub struct Expect<T> {
    actual: T,
}

impl<T: std::fmt::Debug> Expect<T> {
    pub fn to_be(&self, expected: &T) -> bool {
        format!("{:?}", self.actual) == format!("{:?}", expected)
    }

    pub fn to_equal(&self, expected: &T) -> bool {
        self.to_be(expected)
    }
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