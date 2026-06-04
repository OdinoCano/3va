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
    /// Maximum number of test files to run concurrently.
    /// 0 = number of logical CPUs (default).
    pub concurrency: usize,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            verbose: false,
            test_timeout_ms: 5000,
            update_snapshots: false,
            concurrency: 0,
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
            let files = collect_test_files_in_dir(dir);
            self.run_files_parallel(files).await
        })
    }

    /// Run `files` in parallel up to `config.concurrency` tasks at a time.
    /// Concurrency 0 → number of logical CPUs.
    pub async fn run_files_parallel(
        &mut self,
        files: Vec<std::path::PathBuf>,
    ) -> anyhow::Result<()> {
        if files.is_empty() {
            return Ok(());
        }

        let concurrency = if self.config.concurrency == 0 {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
        } else {
            self.config.concurrency
        };

        // When concurrency==1 fall through to sequential to avoid overhead.
        if concurrency == 1 || files.len() == 1 {
            for file in files {
                self.run_file(&file).await?;
            }
            return Ok(());
        }

        use std::sync::{Arc, Mutex};
        let config = self.config.clone();
        let all_results: Arc<Mutex<Vec<TestResult>>> = Arc::new(Mutex::new(Vec::new()));

        // Use a semaphore-like approach: chunk files and join.
        let chunks: Vec<_> = files
            .chunks(concurrency.max(1))
            .map(|c| c.to_vec())
            .collect();
        for chunk in chunks {
            let mut tasks = Vec::new();
            for file in chunk {
                let cfg = config.clone();
                let results_ref = all_results.clone();
                let task = tokio::spawn(async move {
                    let mut runner = TestRunner::new(cfg);
                    if let Err(e) = runner.run_file(&file).await {
                        eprintln!("[test runner] error in {}: {e}", file.display());
                    }
                    results_ref.lock().unwrap().extend(runner.results);
                });
                tasks.push(task);
            }
            for t in tasks {
                let _ = t.await;
            }
        }

        let collected = Arc::try_unwrap(all_results)
            .unwrap_or_else(|a| std::sync::Mutex::new(a.lock().unwrap().clone()))
            .into_inner()
            .unwrap_or_default();
        self.results.extend(collected);
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

/// Recursively collect `.test.js`, `.test.ts`, `.spec.js`, `.spec.ts` files.
pub fn collect_test_files_in_dir(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.starts_with('.') || name == "node_modules" {
                continue;
            }
            out.extend(collect_test_files_in_dir(&path));
        } else {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name.ends_with(".test.js")
                || name.ends_with(".test.ts")
                || name.ends_with(".spec.js")
                || name.ends_with(".spec.ts")
            {
                out.push(path);
            }
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    Json,
    Junit,
    Tap,
    Dot,
}

pub struct TestReporter {
    format: ReportFormat,
}

impl TestReporter {
    pub fn new(format: ReportFormat) -> Self {
        Self { format }
    }

    pub fn report(&self, results: &[TestResult]) -> String {
        match self.format {
            ReportFormat::Json => self.report_json(results),
            ReportFormat::Junit => self.report_junit(results),
            ReportFormat::Tap => self.report_tap(results),
            ReportFormat::Dot => self.report_dot(results),
        }
    }

    fn report_json(&self, results: &[TestResult]) -> String {
        serde_json::to_string_pretty(results).unwrap_or_default()
    }

    /// JUnit XML format compatible with Jenkins, GitHub Actions, and most CI systems.
    fn report_junit(&self, results: &[TestResult]) -> String {
        let passed = results
            .iter()
            .filter(|r| r.status == TestStatus::Passed)
            .count();
        let failed = results
            .iter()
            .filter(|r| r.status == TestStatus::Failed)
            .count();
        let skipped = results
            .iter()
            .filter(|r| r.status == TestStatus::Skipped)
            .count();
        let total_ms: u64 = results.iter().map(|r| r.duration_ms).sum();

        let mut xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuites tests="{total}" failures="{failed}" skipped="{skipped}" time="{time:.3}">
  <testsuite name="3va" tests="{total}" failures="{failed}" skipped="{skipped}" time="{time:.3}">
"#,
            total = results.len(),
            failed = failed,
            skipped = skipped,
            time = total_ms as f64 / 1000.0
        );

        for r in results {
            let time = r.duration_ms as f64 / 1000.0;
            let name = xml_escape(&r.name);
            match r.status {
                TestStatus::Passed => {
                    xml.push_str(&format!(
                        "    <testcase name=\"{name}\" time=\"{time:.3}\"/>\n"
                    ));
                }
                TestStatus::Failed => {
                    let msg = xml_escape(r.error.as_deref().unwrap_or("test failed"));
                    xml.push_str(&format!(
                        "    <testcase name=\"{name}\" time=\"{time:.3}\">\n\
                               <failure message=\"{msg}\"/>\n\
                         </testcase>\n"
                    ));
                }
                TestStatus::Skipped | TestStatus::Pending => {
                    xml.push_str(&format!(
                        "    <testcase name=\"{name}\" time=\"{time:.3}\">\n\
                         <skipped/>\n\
                         </testcase>\n"
                    ));
                }
            }
        }

        xml.push_str("  </testsuite>\n</testsuites>\n");
        let _ = passed; // suppress unused warning
        xml
    }

    /// TAP (Test Anything Protocol) version 13 output.
    fn report_tap(&self, results: &[TestResult]) -> String {
        let mut out = format!("TAP version 13\n1..{}\n", results.len());
        for (i, r) in results.iter().enumerate() {
            match r.status {
                TestStatus::Passed => {
                    out.push_str(&format!("ok {} {}\n", i + 1, r.name));
                }
                TestStatus::Failed => {
                    out.push_str(&format!("not ok {} {}\n", i + 1, r.name));
                    if let Some(err) = &r.error {
                        for line in err.lines() {
                            out.push_str(&format!("  # {line}\n"));
                        }
                    }
                }
                TestStatus::Skipped | TestStatus::Pending => {
                    out.push_str(&format!("ok {} {} # SKIP\n", i + 1, r.name));
                }
            }
        }
        out
    }

    /// Dot reporter: `.` for pass, `F` for fail, `S` for skip.
    fn report_dot(&self, results: &[TestResult]) -> String {
        let dots: String = results
            .iter()
            .map(|r| match r.status {
                TestStatus::Passed => '.',
                TestStatus::Failed => 'F',
                TestStatus::Skipped | TestStatus::Pending => 'S',
            })
            .collect();

        let failed: Vec<_> = results
            .iter()
            .filter(|r| r.status == TestStatus::Failed)
            .collect();
        let mut out = format!(
            "{dots}\n\n{} tests, {} passed, {} failed\n",
            results.len(),
            results
                .iter()
                .filter(|r| r.status == TestStatus::Passed)
                .count(),
            failed.len()
        );
        for r in &failed {
            out.push_str(&format!("  FAIL: {}\n", r.name));
            if let Some(err) = &r.error {
                out.push_str(&format!("       {err}\n"));
            }
        }
        out
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runner_new() {
        let runner = TestRunner::new(TestConfig::default());
        assert_eq!(runner.get_results().len(), 0);
    }

    fn sample_results() -> Vec<TestResult> {
        vec![
            TestResult {
                name: "passes".into(),
                status: TestStatus::Passed,
                duration_ms: 5,
                error: None,
            },
            TestResult {
                name: "fails".into(),
                status: TestStatus::Failed,
                duration_ms: 2,
                error: Some("oops".into()),
            },
            TestResult {
                name: "skips".into(),
                status: TestStatus::Skipped,
                duration_ms: 0,
                error: None,
            },
        ]
    }

    #[test]
    fn test_reporter_json() {
        let r = TestReporter::new(ReportFormat::Json);
        let out = r.report(&sample_results());
        assert!(out.contains("passes"));
        let _: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
    }

    #[test]
    fn test_reporter_junit() {
        let r = TestReporter::new(ReportFormat::Junit);
        let out = r.report(&sample_results());
        assert!(out.contains("<?xml version"));
        assert!(out.contains("<testsuites"));
        assert!(out.contains("<failure"));
        assert!(out.contains("passes"));
        assert!(out.contains("oops"));
    }

    #[test]
    fn test_reporter_tap() {
        let r = TestReporter::new(ReportFormat::Tap);
        let out = r.report(&sample_results());
        assert!(out.starts_with("TAP version 13"));
        assert!(out.contains("ok 1 passes"));
        assert!(out.contains("not ok 2 fails"));
        assert!(out.contains("# oops"));
    }

    #[test]
    fn test_reporter_dot() {
        let r = TestReporter::new(ReportFormat::Dot);
        let out = r.report(&sample_results());
        assert!(out.contains('.'));
        assert!(out.contains('F'));
    }

    #[test]
    fn test_config_has_concurrency() {
        let cfg = TestConfig::default();
        assert_eq!(cfg.concurrency, 0); // 0 = use CPU count
    }

    #[test]
    fn collect_test_files_finds_spec_files() {
        use tempfile::TempDir;
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("a.test.js"), "").unwrap();
        std::fs::write(dir.path().join("b.spec.ts"), "").unwrap();
        std::fs::write(dir.path().join("c.js"), "").unwrap(); // not a test file
        let files = collect_test_files_in_dir(dir.path());
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn xml_escape_replaces_entities() {
        assert_eq!(
            xml_escape("<foo & 'bar\">"),
            "&lt;foo &amp; &apos;bar&quot;&gt;"
        );
    }
}
