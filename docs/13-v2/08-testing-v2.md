# 08 - Testing runner Improvements v2.0.0

## 8.1 Overview

To scale up testing for complex enterprise applications and Monorepos, v2.0.0 introduces major enhancements to the native test runner (`3va test`):
- **Isolated Parallel Execution:** Concurrent test running across multiple isolated JS threads.
- **Testing Mocking API (`3va:test`):** Programmatic spies, mocks, and fake timers.
- **Reporters and Integration:** Structured reporting formats for CI/CD environments.

---

## 8.2 Parallel Execution

In v1.0.0, test files were executed sequentially on a single thread. v2.0.0 implements an isolated parallel runner:

- Each test file is loaded and run in its own OS thread with a clean `JsEngine` instance (QuickJS context isolation).
- Concurrency levels default to the system's logical CPU core count (via `os.availableParallelism()`).

```bash
# Run tests in parallel (default concurrency)
3va test

# Specify concurrency count manually
3va test --concurrency=4

# Run sequentially (similar to v1.0.0)
3va test --concurrency=1
```

---

## 8.3 Mocking API (`3va:test`)

v2.0.0 introduces a native mocking framework under the standard `3va:test` built-in module:

```js
const { mock, test, expect } = require('3va:test');

// 1. Spying and Mocking Functions
const spy = mock.fn((x) => x + 1);
spy(1);
spy(2);

expect(spy.mock.calls.length).toBe(2);
expect(spy.mock.calls[0].arguments).toEqual([1]);
expect(spy.mock.calls[1].result).toBe(3);

// 2. Mocking Object Methods
const userService = {
  fetchName: async (id) => 'John'
};

const methodMock = mock.method(userService, 'fetchName', async (id) => 'Mocked User');

const name = await userService.fetchName(1);
expect(name).toBe('Mocked User');

// Restore original implementation automatically after the test run
methodMock.mock.restore();
```

### 8.3.1 Fake Timers

To test time-dependent logic without synchronous blocks, fake timers allow manually stepping the clock:

```js
const { mock, test, expect } = require('3va:test');

test('test with fake timers', () => {
  mock.timers.enable();
  
  let fired = false;
  setTimeout(() => { fired = true; }, 1000);
  
  expect(fired).toBe(false);
  
  // Advance the clock by 1000 ms synchronously
  mock.timers.tick(1000);
  
  expect(fired).toBe(true);
  
  mock.timers.reset(); // restore real timers
});
```

---

## 8.4 Reporters

For CI/CD pipelines (e.g. GitHub Actions, GitLab CI), `3va test` supports structured reporting formats:

```bash
# Output spec-compliant JUnit XML file for CI ingestion
3va test --reporter=junit --reporter-file=junit.xml

# Output minimal dot reporter for massive test suites
3va test --reporter=dot
```

Supported reporter formats (`--reporter`, default `terminal`):
* `terminal` (default): Colored hierarchical outline.
* `json`: Machine-readable JSON results.
* `junit`: Standard XML format compatible with CI test dashboards.
* `tap`: Test Anything Protocol output.
* `dot`: Minimalist single-character progression (`.`, `F`, `S`).
