# 05 - THREADING AND CONCURRENCY MODEL

## 5.1 Overview

3va uses **Tokio's multi-threaded runtime** by default. The `#[tokio::main]` entry point in `vvva_cli` spawns a work-stealing thread pool that distributes async tasks across all available CPU cores.

## 5.2 Default Thread Count

| Runtime | Default workers | Mechanism |
|---------|----------------|-----------|
| **Tokio** (async I/O) | One per logical CPU | `#[tokio::main]` |
| **V8** (JS execution) | Single-threaded per isolate | Full isolate-per-thread model |

Tokio's worker pool handles all I/O, timers, and async tasks. V8 executes JavaScript on the thread that calls it — each isolate is independent with its own heap.

## 5.3 Controlling Tokio Worker Threads

Set the `TOKIO_WORKER_THREADS` environment variable to override the default:

```bash
TOKIO_WORKER_THREADS=2 3va run app.ts
TOKIO_WORKER_THREADS=8 3va test --concurrency=4
TOKIO_WORKER_THREADS=1 3va run server.js      # single-threaded mode
```

Without this variable, Tokio spawns one worker thread per logical CPU (including hyper-threads).

## 5.4 Per-Command Concurrency Flags

Some subcommands accept a `--concurrency` flag that controls parallelism independently of Tokio's worker pool:

| Command | Flag | Default | Description |
|---------|------|---------|-------------|
| `3va test` | `--concurrency <N>` | `0` (CPU count) | Max concurrent test files, each in its own `JsEngine` instance |
| `3va workspace run` | `--concurrency <N>` | config or `4` | Max concurrent packages during script execution |

```bash
3va test --concurrency=8
3va workspace run build --parallel --concurrency=4
```

When `--concurrency` is `0`, the runtime uses the number of logical CPUs (same default as Tokio).

## 5.5 Threading Model by Component

| Component | Threading |
|-----------|-----------|
| **I/O, timers, networking** | Tokio work-stealing pool (multi-threaded) |
| **JS evaluation** | Single-threaded per `JsEngine` instance |
| **Test runner** | One OS thread + `JsEngine` per test file (controlled by `--concurrency`) |
| **`worker_threads` (JS API)** | Each `new Worker(file)` spawns a real OS thread with its own `JsEngine` and Tokio runtime; message passing via `std::sync::mpsc` |
| **File watching** (`3va dev`, `3va test --watch`) | Dedicated OS thread via `std::thread::spawn` |
| **Package resolver** | `tokio::spawn` per uncached package (batch fetch) |

## 5.6 Related

- `worker_threads` API: `docs/13-v2/02-node-compat-v2.md`
- Event loop: `docs/04-core/01-event-loop.md`
- Parallel test execution: `docs/13-v2/08-testing-v2.md`
- Workspace parallel execution: `docs/13-v2/04-workspace-v2.md`
