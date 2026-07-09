# vvva_js

JavaScript/TypeScript engine crate. Wraps [V8](https://v8.dev/) via the [`v8`](https://docs.rs/v8) crate and exposes `JsEngine` plus all built-in modules.

## Key types

- **`JsEngine`** — async runtime handle; create with `JsEngine::new(permissions)`, run with `eval()` / `eval_file()` / `run_event_loop()`
- **`transpiler`** — oxc-backed TS/TSX/JSX → JS transpiler (`transpile`, `transpile_jsx`, `transpile_js`)

## Built-in modules

`console`, `timers`, `buffer`, `process`, `fetch`, `fs`, `tcp`, `http`, `websocket`, `zlib`, `child_process`, `crypto`, `require`

## Docs

`docs/05-js-engine/`
