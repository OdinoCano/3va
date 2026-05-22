# 04 - WATCH MODE

## 4.1 Description

Watch mode monitors the filesystem and automatically re-runs all tests when it detects changes in source code files. It requires no user interaction during execution.

## 4.2 Usage

```bash
# Activate watch mode
3va test --watch

# Watch with coverage report on each re-run
3va test --watch --coverage
```

## 4.3 Behavior

- On startup, runs all discovered tests once.
- Monitors changes in files with `.js`, `.ts`, `.jsx` and `.tsx` extensions.
- Applies a **500 ms debounce**: if multiple changes are detected in a burst, it waits for activity to stop before launching the next run.
- On detecting a change, re-runs the entire suite from scratch.
- The process stays running until interrupted with `Ctrl+C`.

## 4.4 Monitored Files

| Extension | Description |
|-----------|-------------|
| `.js` | JavaScript |
| `.ts` | TypeScript |
| `.jsx` | JavaScript + JSX |
| `.tsx` | TypeScript + JSX |

## 4.5 Limitations

- No interactive keyboard controls. The `a`, `f`, `p`, `t` and `q` keys found in Jest's interface are **not implemented**.
- It is not possible to filter tests by name or path from watch mode.
- No integration with Watchman; the watcher uses the Rust `notify` crate directly.
- No exclusion patterns are read from configuration files.

---

*Watch mode implemented in `crates/cli/src/main.rs` (`run_test_watch_mode`) using the `notify` crate. The `TestRunner` from `crates/test/src/runner.rs` is invoked on each re-run.*
