use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatcherResult {
    pub passed: bool,
    pub message: String,
}

pub struct Matchers;

impl Matchers {
    pub fn to_be<T: std::fmt::Debug>(actual: &T, expected: &T) -> MatcherResult {
        let passed = format!("{:?}", actual) == format!("{:?}", expected);
        MatcherResult {
            passed,
            message: if passed { "PASS".to_string() } else { format!("Expected {:?} to be {:?}", actual, expected) },
        }
    }

    pub fn to_equal<T: std::fmt::Debug>(actual: &T, expected: &T) -> MatcherResult {
        Self::to_be(actual, expected)
    }

    pub fn toBeTruthy<T: std::fmt::Debug>(value: &T) -> MatcherResult {
        let passed = !format!("{:?}", value).is_empty();
        MatcherResult {
            passed,
            message: if passed { "PASS".to_string() } else { "Expected value to be truthy".to_string() },
        }
    }

    pub fn toBeFalsy<T: std::fmt::Debug>(value: &T) -> MatcherResult {
        let passed = format!("{:?}", value).is_empty();
        MatcherResult {
            passed,
            message: if passed { "PASS".to_string() } else { "Expected value to be falsy".to_string() },
        }
    }

    pub fn to_contain(haystack: &str, needle: &str) -> MatcherResult {
        let passed = haystack.contains(needle);
        MatcherResult {
            passed,
            message: if passed { "PASS".to_string() } else { format!("Expected '{}' to contain '{}'", haystack, needle) },
        }
    }

    pub fn toBeGreaterThan(actual: f64, expected: f64) -> MatcherResult {
        let passed = actual > expected;
        MatcherResult {
            passed,
            message: if passed { "PASS".to_string() } else { format!("Expected {} > {}", actual, expected) },
        }
    }

    pub fn toBeLessThan(actual: f64, expected: f64) -> MatcherResult {
        let passed = actual < expected;
        MatcherResult {
            passed,
            message: if passed { "PASS".to_string() } else { format!("Expected {} < {}", actual, expected) },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_be() {
        let result = Matchers::to_be(&"hello", &"hello");
        assert!(result.passed);
    }

    #[test]
    fn test_to_contain() {
        let result = Matchers::to_contain("hello world", "world");
        assert!(result.passed);
    }
}