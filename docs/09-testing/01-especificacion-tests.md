# 01 - TEST SPECIFICATION

## 1.1 Test Runner

3va's test runner executes tests written in JavaScript or TypeScript, injecting a Jest-compatible global API into each test file. Each file runs in its own isolated JS engine instance with read and write permissions limited to the file's directory.

## 1.2 Usage

```bash
# Discover and run all tests in the current directory
3va test

# Run tests in specific paths
3va test tests/ src/lib/

# Watch mode: re-runs on file changes
3va test --watch

# Line and branch coverage report
3va test --coverage

# Update existing snapshots on disk
3va test --update-snapshots
```

No support for `--bail`, `--test-name-pattern`, or any `jest.config.js` configuration file.

## 1.3 File Discovery

The runner recursively searches for files matching the following name patterns:

| Pattern | Description |
|--------|-------------|
| `*.test.js` | JavaScript test |
| `*.test.ts` | TypeScript test |
| `*.spec.js` | JavaScript spec |
| `*.spec.ts` | TypeScript spec |

## 1.4 Injected Global API

The following functions are available as globals within each test file; they do not need to be imported.

```javascript
// Groups related tests; is nestable
describe("suite name", () => {
  // Registers an individual test
  test("test name", () => {
    expect(1 + 1).toBe(2);
  });

  // Exact alias of test
  it("also registers a test", () => {
    expect("hello").toHaveLength(5);
  });
});
```

## 1.5 expect and Matchers

`expect(value)` creates an assertion chain. All matchers support negation via `.not`:

```javascript
expect(value).toBe(expected);
expect(value).not.toBe(unexpected);
```

The full list of implemented matchers is documented in `02-matchers.md`.

## 1.6 Snapshots

The first time a test calls `.toMatchSnapshot()`, the serialized value is saved to disk. Subsequent runs compare against that saved value.

- File location: `__snapshots__/<test-name>.snap` alongside the test file.
- Format: Plain JSON with the structure `{ "test name": <serialized value>, ... }`.
- To update outdated snapshots: `3va test --update-snapshots`.

```javascript
test("object has the expected shape", () => {
  const result = { id: 1, active: true };
  expect(result).toMatchSnapshot();
});
```

## 1.7 Runner Behavior

- Each test file runs in its own `JsEngine` instance with isolated `PermissionState`.
- `FileRead` and `FileWrite` permissions are granted to the file's directory (needed for reading and writing snapshots).
- Output reports `PASS` / `FAIL`, the suite name, the assertion message, and elapsed time.
- Syntax errors in the test file are caught and reported as a failed test.
- `run_directory(path)` discovers and recursively executes all test files in a directory.

## 1.8 Coverage

The `--coverage` flag generates a **line** and **branch** coverage report. See `03-coverage.md` for details.

---

*Implemented in `crates/test/src/` (`runner.rs`, `framework.rs`, `matchers.rs`, `coverage.rs`).*
