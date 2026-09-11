# 07 — Test Runner

Jest-compatible tests run directly with `3va test` — no Jest install needed.

## Run

```bash
3va test
```

### Other useful commands

```bash
3va test tests/unit          # run a specific directory
3va test --watch             # re-run on file changes
3va test --coverage          # coverage report
3va test --update-snapshots  # update stored snapshots
```

## What it demonstrates

- `describe`, `test`, `expect` — the full Jest API
- All standard matchers: `toBe`, `toThrow`, etc.
- Snapshot testing with `toMatchSnapshot`
- Watch mode and coverage support