use crate::framework::{TestResult, TestStatus};
use std::path::Path;
use std::sync::Arc;
use vvva_permissions::PermissionState;

/// JS test framework injected into each test file's QuickJS context.
/// Provides describe/it/test/expect with full matcher support, then
/// `__3va_run_tests()` executes all registered cases and returns JSON.
const TEST_FRAMEWORK_JS: &str = r#"
(function() {
  var __suites = [];
  var __tests  = [];

  // Lifecycle hooks: { suite: string[], fn: function }
  // suite[] is a snapshot of __suites at registration time, used for scope matching.
  var __hooksBeforeAll  = [];
  var __hooksAfterAll   = [];
  var __hooksBeforeEach = [];
  var __hooksAfterEach  = [];

  // Build a stable string key from a suite array.
  function _key(suite) { return suite.join('\x1f'); }

  // True when hookSuite is an ancestor scope of testSuites (prefix match).
  function _ancestorOf(hookSuite, testSuites) {
    if (hookSuite.length > testSuites.length) return false;
    for (var i = 0; i < hookSuite.length; i++) {
      if (hookSuite[i] !== testSuites[i]) return false;
    }
    return true;
  }

  globalThis.describe = function(name, fn) {
    __suites.push(name);
    try { fn(); } finally { __suites.pop(); }
  };

  globalThis.it = globalThis.test = function(name, fn) {
    var prefix = __suites.length > 0 ? __suites.join(' > ') + ' > ' : '';
    __tests.push({ name: prefix + name, fn: fn, suites: __suites.slice() });
  };

  globalThis.beforeAll  = function(fn) { __hooksBeforeAll.push({ suite: __suites.slice(), fn: fn }); };
  globalThis.afterAll   = function(fn) { __hooksAfterAll.push({ suite: __suites.slice(), fn: fn }); };
  globalThis.beforeEach = function(fn) { __hooksBeforeEach.push({ suite: __suites.slice(), fn: fn }); };
  globalThis.afterEach  = function(fn) { __hooksAfterEach.push({ suite: __suites.slice(), fn: fn }); };

  function makeExpect(actual, negated) {
    var inv = negated ? ' not' : '';
    return {
      get not() { return makeExpect(actual, !negated); },
      toBe: function(expected) {
        var ok = actual === expected;
        if (negated ? ok : !ok)
          throw new Error('Expected ' + JSON.stringify(actual) + inv + ' to be ' + JSON.stringify(expected));
      },
      toEqual: function(expected) {
        var ok = JSON.stringify(actual) === JSON.stringify(expected);
        if (negated ? ok : !ok)
          throw new Error('Expected ' + JSON.stringify(actual) + inv + ' to equal ' + JSON.stringify(expected));
      },
      toStrictEqual: function(expected) {
        var ok = JSON.stringify(actual) === JSON.stringify(expected);
        if (negated ? ok : !ok)
          throw new Error('Expected ' + JSON.stringify(actual) + inv + ' to strictly equal ' + JSON.stringify(expected));
      },
      toContain: function(expected) {
        var ok;
        if (typeof actual === 'string') ok = actual.includes(String(expected));
        else if (Array.isArray(actual)) ok = actual.indexOf(expected) !== -1;
        else throw new Error('toContain requires a string or array');
        if (negated ? ok : !ok)
          throw new Error('Expected ' + JSON.stringify(actual) + inv + ' to contain ' + JSON.stringify(expected));
      },
      toThrow: function(msg) {
        var threw = false, errMsg = '';
        try { actual(); } catch(e) { threw = true; errMsg = (e && e.message) ? e.message : String(e); }
        var matches = threw && (msg === undefined || errMsg.indexOf(String(msg)) !== -1);
        if (negated ? threw : !matches)
          throw new Error(negated
            ? 'Expected function not to throw'
            : 'Expected function to throw' + (msg ? ' "'+msg+'"' : '') + (threw ? ' but got "'+errMsg+'"' : ' but it did not'));
      },
      toBeTruthy: function() {
        var ok = !!actual;
        if (negated ? ok : !ok) throw new Error('Expected ' + JSON.stringify(actual) + inv + ' to be truthy');
      },
      toBeFalsy: function() {
        var ok = !actual;
        if (negated ? ok : !ok) throw new Error('Expected ' + JSON.stringify(actual) + inv + ' to be falsy');
      },
      toBeNull: function() {
        var ok = actual === null;
        if (negated ? ok : !ok) throw new Error('Expected ' + JSON.stringify(actual) + inv + ' to be null');
      },
      toBeUndefined: function() {
        var ok = actual === undefined;
        if (negated ? ok : !ok) throw new Error('Expected ' + JSON.stringify(actual) + inv + ' to be undefined');
      },
      toBeDefined: function() {
        var ok = actual !== undefined;
        if (negated ? ok : !ok) throw new Error('Expected value' + inv + ' to be defined');
      },
      toHaveLength: function(len) {
        var actualLen = (actual != null) ? actual.length : undefined;
        var ok = actualLen === len;
        if (negated ? ok : !ok) throw new Error('Expected length ' + JSON.stringify(len) + inv + ' but got ' + actualLen);
      },
      toMatch: function(pattern) {
        var re = (typeof pattern === 'string') ? new RegExp(pattern) : pattern;
        var ok = re.test(actual);
        if (negated ? ok : !ok) throw new Error('Expected "' + actual + '"' + inv + ' to match ' + re);
      },
      toBeGreaterThan: function(n) {
        var ok = actual > n;
        if (negated ? ok : !ok) throw new Error('Expected ' + actual + inv + ' > ' + n);
      },
      toBeGreaterThanOrEqual: function(n) {
        var ok = actual >= n;
        if (negated ? ok : !ok) throw new Error('Expected ' + actual + inv + ' >= ' + n);
      },
      toBeLessThan: function(n) {
        var ok = actual < n;
        if (negated ? ok : !ok) throw new Error('Expected ' + actual + inv + ' < ' + n);
      },
      toBeLessThanOrEqual: function(n) {
        var ok = actual <= n;
        if (negated ? ok : !ok) throw new Error('Expected ' + actual + inv + ' <= ' + n);
      },
    };
  }

  globalThis.expect = function(actual) { return makeExpect(actual, false); };

  // ── Snapshot support ────────────────────────────────────────────────────
  var __snapshots = {};  // in-memory cache: snapFile -> { key: value }
  var __newSnapshots = 0;
  var __updatedSnapshots = 0;

  function _snapLoad(file) {
    if (__snapshots[file]) return __snapshots[file];
    try {
      var json = __fsReadFileSync(file);
      __snapshots[file] = JSON.parse(json);
    } catch(e) {
      __snapshots[file] = {};
    }
    return __snapshots[file];
  }

  function _snapSave(file, data) {
    // Ensure __snapshots__ directory exists
    var dir = file.replace(/[/\\][^/\\]+$/, '');
    try { __fsMkdirSync(dir); } catch(e) {}
    __fsWriteFileSync(file, JSON.stringify(data, null, 2));
  }

  function _snapFile() {
    var f = (globalThis.__snapshotFile || '__snapshots__/inline.snap') + '.snap.json';
    return f;
  }

  globalThis.expect = (function(_origExpect) {
    return function(actual) {
      var base = _origExpect(actual);
      base.toMatchSnapshot = function(hint) {
        var snapFile = _snapFile();
        var suitePrefix = __suites.length > 0 ? __suites.join(' > ') + ' > ' : '';
        var key = suitePrefix + (hint || '');
        var data = _snapLoad(snapFile);
        if (globalThis.__updateSnapshots || !(key in data)) {
          var serialized = typeof actual === 'string' ? actual : JSON.stringify(actual, null, 2);
          if (key in data) __updatedSnapshots++; else __newSnapshots++;
          data[key] = serialized;
          _snapSave(snapFile, data);
        } else {
          var stored = data[key];
          var current = typeof actual === 'string' ? actual : JSON.stringify(actual, null, 2);
          if (stored !== current)
            throw new Error('Snapshot mismatch for "' + key + '":\n  Stored : ' + stored + '\n  Current: ' + current);
        }
      };
      base.toMatchInlineSnapshot = function(expected) {
        if (expected === undefined) {
          // First run — write as console output
          console.log('[snapshot] ' + JSON.stringify(actual, null, 2));
          return;
        }
        var current = typeof actual === 'string' ? actual : JSON.stringify(actual, null, 2);
        var exp = expected.trim();
        if (current.trim() !== exp)
          throw new Error('Inline snapshot mismatch:\n  Expected: ' + exp + '\n  Received: ' + current);
      };
      return base;
    };
  })(globalThis.expect);

  globalThis.__3va_run_tests = function() {
    var results = [];

    // Count how many tests belong to each scope so we know when to fire afterAll.
    var remaining = {};
    for (var i = 0; i < __tests.length; i++) {
      var ts = __tests[i].suites;
      for (var d = 0; d <= ts.length; d++) {
        var k = _key(ts.slice(0, d));
        remaining[k] = (remaining[k] || 0) + 1;
      }
    }

    // Track which scopes have already had their beforeAll fired.
    var beforeAllDone = {};
    // Track scopes whose beforeAll threw, so we skip their tests.
    var scopeFailed   = {};

    for (var i = 0; i < __tests.length; i++) {
      var t  = __tests[i];
      var ts = t.suites;

      // ── beforeAll (outer→inner, once per scope) ───────────────────────────
      var setupErr = null;
      for (var d = 0; d <= ts.length && !setupErr; d++) {
        var scope = ts.slice(0, d);
        var k     = _key(scope);
        if (!beforeAllDone[k]) {
          beforeAllDone[k] = true;
          for (var h = 0; h < __hooksBeforeAll.length && !setupErr; h++) {
            var hook = __hooksBeforeAll[h];
            if (_key(hook.suite) === k) {
              try { hook.fn(); } catch(e) {
                setupErr = 'beforeAll failed: ' + ((e && e.message) ? e.message : String(e));
                // Mark this scope and all children as failed.
                scopeFailed[k] = setupErr;
              }
            }
          }
        } else if (scopeFailed[k]) {
          setupErr = scopeFailed[k];
        }
      }

      var start  = Date.now();
      var status, error;

      if (setupErr) {
        status = 'failed';
        error  = setupErr;
      } else {
        // ── beforeEach (outer→inner) ─────────────────────────────────────────
        var eachErr = null;
        for (var d = 0; d <= ts.length && !eachErr; d++) {
          var k = _key(ts.slice(0, d));
          for (var h = 0; h < __hooksBeforeEach.length && !eachErr; h++) {
            var hook = __hooksBeforeEach[h];
            if (_key(hook.suite) === k) {
              try { hook.fn(); } catch(e) {
                eachErr = 'beforeEach failed: ' + ((e && e.message) ? e.message : String(e));
              }
            }
          }
        }

        if (eachErr) {
          status = 'failed';
          error  = eachErr;
        } else {
          // ── test body ──────────────────────────────────────────────────────
          status = 'passed';
          error  = null;
          try { t.fn(); } catch(e) {
            status = 'failed';
            error  = (e && e.message) ? e.message : String(e);
          }
        }

        // ── afterEach (inner→outer, always runs) ─────────────────────────────
        for (var d = ts.length; d >= 0; d--) {
          var k = _key(ts.slice(0, d));
          for (var h = 0; h < __hooksAfterEach.length; h++) {
            var hook = __hooksAfterEach[h];
            if (_key(hook.suite) === k) {
              try { hook.fn(); } catch(e) { /* swallow — don't mask test failure */ }
            }
          }
        }
      }

      results.push({ name: t.name, status: status, duration_ms: Date.now() - start, error: error });

      // ── afterAll (inner→outer, when scope is exhausted) ───────────────────
      for (var d = ts.length; d >= 0; d--) {
        var k = _key(ts.slice(0, d));
        remaining[k]--;
        if (remaining[k] === 0) {
          for (var h = __hooksAfterAll.length - 1; h >= 0; h--) {
            var hook = __hooksAfterAll[h];
            if (_key(hook.suite) === k) {
              try { hook.fn(); } catch(e) { /* swallow */ }
            }
          }
        }
      }
    }

    return JSON.stringify(results);
  };
})();
"#;

pub struct TestRunner {
    results: Vec<TestResult>,
    config: TestConfig,
}

#[derive(Debug, Clone)]
pub struct TestConfig {
    pub verbose: bool,
    pub test_timeout_ms: u64,
    pub update_snapshots: bool,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            verbose: false,
            test_timeout_ms: 5000,
            update_snapshots: false,
        }
    }
}

#[derive(serde::Deserialize)]
struct RawResult {
    name: String,
    status: String,
    duration_ms: u64,
    error: Option<String>,
}

impl TestRunner {
    pub fn new(config: TestConfig) -> Self {
        Self {
            results: Vec::new(),
            config,
        }
    }

    pub async fn run_file(&mut self, path: &Path) -> anyhow::Result<()> {
        let display = path.display().to_string();
        if self.config.verbose {
            println!("\n  {}", display);
        }

        // Grant read/write to CWD and to the test file's own directory so
        // __snapshots__/ can be written even when the file lives in a TempDir.
        let perms = PermissionState::new();
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        perms.grant(vvva_permissions::Capability::FileRead(cwd.clone()));
        perms.grant(vvva_permissions::Capability::FileWrite(cwd.clone()));
        if let Some(test_dir) = path.parent() {
            let canonical = test_dir
                .canonicalize()
                .unwrap_or_else(|_| test_dir.to_path_buf());
            perms.grant(vvva_permissions::Capability::FileRead(canonical.clone()));
            perms.grant(vvva_permissions::Capability::FileWrite(canonical));
        }

        let engine = vvva_js::JsEngine::new(Arc::new(perms))
            .await
            .map_err(|e| anyhow::anyhow!("Failed to init JS engine for {}: {}", display, e))?;

        engine
            .eval(TEST_FRAMEWORK_JS)
            .await
            .map_err(|e| anyhow::anyhow!("Test framework injection failed: {}", e))?;

        // Inject snapshot globals: file path and update flag
        let snap_dir = path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("__snapshots__")
            .join(
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .as_ref(),
            );
        let snap_file = snap_dir.to_string_lossy().replace('\\', "/");
        let update = self.config.update_snapshots;
        engine
            .eval(&format!(
                "globalThis.__snapshotFile = '{}'; globalThis.__updateSnapshots = {};",
                snap_file.replace('\'', "\\'"),
                update,
            ))
            .await
            .ok();

        if let Err(e) = engine.eval_file(path).await {
            // File-level syntax/runtime error — report as a single failed test
            eprintln!("  ✗ {} — {}", display, e);
            self.results.push(TestResult {
                name: display,
                status: TestStatus::Failed,
                duration_ms: 0,
                error: Some(e.to_string()),
            });
            return Ok(());
        }

        let json = engine
            .eval_to_string("globalThis.__3va_run_tests()")
            .await
            .map_err(|e| anyhow::anyhow!("__3va_run_tests() failed: {}", e))?;

        let raw: Vec<RawResult> = serde_json::from_str(&json)
            .map_err(|e| anyhow::anyhow!("Could not parse test results JSON: {}", e))?;

        if raw.is_empty() {
            println!("  (no tests found in {})", display);
            return Ok(());
        }

        for r in raw {
            let status = match r.status.as_str() {
                "passed" => TestStatus::Passed,
                "failed" => TestStatus::Failed,
                _ => TestStatus::Skipped,
            };

            match status {
                TestStatus::Passed => println!("  ✓ {}", r.name),
                TestStatus::Failed => eprintln!(
                    "  ✗ {} — {}",
                    r.name,
                    r.error.as_deref().unwrap_or("unknown error")
                ),
                _ => println!("  - {} (skipped)", r.name),
            }

            self.results.push(TestResult {
                name: r.name,
                status,
                duration_ms: r.duration_ms,
                error: r.error,
            });
        }

        Ok(())
    }

    pub fn run_directory<'a>(
        &'a mut self,
        dir: &'a Path,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<()>> + 'a>> {
        Box::pin(async move {
            let mut entries: Vec<_> = std::fs::read_dir(dir)?.flatten().collect();
            entries.sort_by_key(|e| e.path());

            for entry in entries {
                let path = entry.path();
                if path.is_dir() {
                    // Skip node_modules and hidden directories
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    if name.starts_with('.') || name == "node_modules" {
                        continue;
                    }
                    self.run_directory(&path).await?;
                } else {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    if name.ends_with(".test.js")
                        || name.ends_with(".test.ts")
                        || name.ends_with(".spec.js")
                        || name.ends_with(".spec.ts")
                    {
                        self.run_file(&path).await?;
                    }
                }
            }
            Ok(())
        })
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
        let skipped = self
            .results
            .iter()
            .filter(|r| r.status == TestStatus::Skipped)
            .count();
        let total = self.results.len();

        println!("\n=============================");
        println!("Tests:   {total}");
        println!("Passed:  {passed}");
        if failed > 0 {
            eprintln!("Failed:  {failed}");
        }
        if skipped > 0 {
            println!("Skipped: {skipped}");
        }
        println!("=============================\n");

        if failed > 0 {
            eprintln!("Failed tests:");
            for r in self
                .results
                .iter()
                .filter(|r| r.status == TestStatus::Failed)
            {
                eprintln!("  ✗ {}", r.name);
                if let Some(err) = &r.error {
                    eprintln!("      {}", err);
                }
            }
        }
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
    fn test_runner_new() {
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
