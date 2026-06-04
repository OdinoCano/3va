# 03 - COVERAGE

## 3.1 Overview

`3va test --coverage` generates a **statement-level** coverage report using Oxc AST instrumentation. Each JS/TS source file is parsed, executable statements are identified, and hit counters are injected before each one. After the test run the counters are collected and compared against the full statement list.

No external tool (Istanbul, nyc, c8) is required. The entire pipeline runs in Rust inside `crates/test/src/coverage.rs`.

## 3.2 Usage

```bash
# Run tests and print coverage report to stdout
3va test --coverage

# Combine with watch mode
3va test --coverage --watch
```

## 3.3 What is measured

| Metric | Measured | Notes |
|--------|----------|-------|
| Statements | ✅ | Every executable statement gets its own counter |
| Lines | ✅ | Derived from statement positions; a line is covered when any statement on it is hit |
| Branches | ❌ | Not tracked; planned for a future version |
| Functions | ❌ | Not tracked separately; function bodies are covered as statements |

### Excluded from instrumentation

TypeScript-only constructs that produce no executable code are excluded:
- `interface` declarations
- `type` aliases
- `declare` statements
- `import type` / `export type`
- `enum` declarations (TS enums)

## 3.4 Instrumentation approach

The instrumenter (`instrument_source`) works in pure Rust using the Oxc parser:

1. Parse the source with `oxc_parser` (supports JS and TS).
2. Walk the AST with a `Visit` impl that collects every executable `Statement`.
3. Insert a counter increment **before** each statement in reverse byte-offset order so earlier offsets remain valid:
   ```js
   globalThis.__cov["file_id"][idx] = (globalThis.__cov["file_id"][idx] || 0) + 1;
   ```
4. Prepend a preamble that initialises the counter object:
   ```js
   (globalThis.__cov = globalThis.__cov || {});
   (globalThis.__cov["file_id"] = globalThis.__cov["file_id"] || {});
   ```

After each test file runs, the test runner reads the counters:
```js
JSON.stringify(globalThis.__cov["file_id"] || {})
```
and passes the JSON to `parse_hit_counts`, which returns a `HashMap<usize, u64>` of statement index → hit count.

## 3.5 Output format

```
=============================== Coverage ================================
  File coverage     :  3/4 files have tests (75.0%)
  Test pass rate    :  12/12 tests passed (100.0%)
  Statement coverage:  47/52 statements executed (90.4%)

  Source file                                        Test file            Tests
  ────────────────────────────────────────────────────────────────────────────────
  ✓ src/math.ts                                      math.test.ts         3 pass / 0 fail
  ✓ src/utils.ts                                     utils.test.ts        9 pass / 0 fail
  ! src/parser.ts                                    parser.test.ts       0 pass / 0 fail
       Uncovered lines: 14, 22, 38
  ✗ src/legacy.ts                                    (no test)

  ! 1 source file(s) have no test coverage.
=========================================================================
```

**Row markers:**
- `✓` — all statements covered (or no statement data available but tests pass)
- `!` — some statements uncovered, or tests failed
- `✗` — no test file found for this source file

## 3.6 Types

```rust
// crates/test/src/coverage.rs

pub struct StmtInfo {
    pub index: usize,   // counter key inside __cov["file_id"][index]
    pub line:  u32,     // 1-indexed source line
}

pub struct CoverageResult {
    pub total_statements:   usize,
    pub covered_statements: usize,
    pub stmt_hits:          Vec<(StmtInfo, u64)>,   // all stmts with hit counts
    pub uncovered_stmts:    Vec<StmtInfo>,           // stmts never executed
}

impl CoverageResult {
    pub fn coverage_percent(&self) -> f64 { ... }
    pub fn uncovered_lines(&self) -> Vec<u32> { ... }
}
```

## 3.7 Limitations

- **No branch coverage.** `if/else`, ternary, and `&&`/`||` short-circuits are not tracked as separate branches.
- **Single-file granularity.** Coverage counters are per test file, not per source module. If one test file imports multiple modules, all their statements are counted together.
- **No HTML report.** Output is text-only to stdout.
- **No threshold enforcement.** There is no `--coverage-threshold` flag; the command always exits 0 if all tests pass, regardless of coverage percentage.

---

*Implemented in `crates/test/src/coverage.rs`. Tests in the same file under `#[cfg(test)]`.*
