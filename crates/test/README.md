# vvva_test

Test framework crate for 3va — runner, matchers, coverage tracking, and security test utilities.

## Key types

- **`TestRunner`** — discovers and executes test files; integrates with the JS engine
- **`Matchers`** — assertion helpers (`expect`, `toBe`, `toEqual`, `toThrow`, …)
- **`CoverageCollector`** — tracks JS line/branch coverage during test runs
- **`SecurityTestHelper`** — utilities for writing permission-boundary tests

## Running tests

```bash
3va test                  # run all tests in the project
3va test --coverage       # with coverage report
```

## Docs

`docs/09-testing/`
