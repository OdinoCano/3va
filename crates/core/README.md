# vvva_core

Shared runtime primitives used by `vvva_js` and the CLI — async task queue and timer management.

## Key types

- **`TaskQueue`** — priority queue for async microtasks and macrotasks; drives the JS event loop
- **`TimerManager`** — implements `setTimeout` / `setInterval` / `clearTimeout` semantics; returns expired timers on each `poll_timers()` tick

## Usage

These types are internal to the runtime. Direct use from application code is not expected; prefer the JS `setTimeout`/`setInterval` APIs.

## Docs

`docs/04-core/`
