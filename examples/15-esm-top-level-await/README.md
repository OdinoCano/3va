# 15 — ESM + top-level await

`export`, `await import()` and top-level `await` in your own modules, the way a
real app's `db/index.js` uses them.

## Run

```bash
3va run --allow-read=. app.js
```

## What it demonstrates

- `const x = await import("pkg")` at module top level — `x` is the resolved module,
  not a Promise
- `export` of values computed after a top-level `await`
- `await` inside `if`/`for` at top level, and `import()` of a module that awaits a timer
- Only read access to the project directory is needed

Regression test: `crates/js/tests/esm_tla.rs`.
