# 02 - COMPONENT DESIGN

## 2.1 Component: vvva_core

### 2.1.1 Description
Provides the async runtime infrastructure: task scheduling, timer management, and coordination between components. Wraps Tokio and exposes a deterministic `TimerWheel` for JS timer semantics.

### 2.1.2 Structure

```rust
pub struct Runtime {
    pub permissions: PermissionState,
    task_queue: TaskQueue,    // priority queue for async tasks
    timer_wheel: TimerWheel,  // hierarchical timer structure
}
```

### 2.1.3 Responsibilities
- Enqueuing and draining async tasks (`schedule_task`)
- Managing JS timer semantics (`set_timeout`, `set_interval`, `poll_timers`)
- Exposing `pending_task_count` and `next_timer_duration` to the JS event loop

### 2.1.4 Key Methods

```rust
pub async fn run(&self) -> anyhow::Result<()>
pub fn schedule_task<F>(&mut self, task_type: TaskType, future: F)
pub fn set_timeout<F>(&mut self, delay: Duration, callback: F) -> TimerId
pub fn set_interval<F>(&mut self, interval: Duration, callback: F) -> TimerId
pub fn clear_timeout(&mut self, id: TimerId) -> bool
pub fn poll_timers(&mut self) -> Vec<Timer>
pub fn pending_task_count(&self) -> usize
```

### 2.1.5 Dependencies
- `tokio` (async runtime)
- `vvva_permissions` (capability verification)

---

## 2.2 Component: vvva_cli

### 2.2.1 Description
The command line interface, implemented with `clap`. Parses arguments and routes to the appropriate crate.

### 2.2.2 Subcommands

| Command | Description | Example |
|---------|-------------|---------|
| `run` | Executes a JS/TS file | `3va run app.ts` |
| `install` | Installs a package | `3va install axios --allow-net=registry.npmjs.org` |
| `update` | Updates packages | `3va update --allow-net=registry.npmjs.org` |
| `reinstall` | Force-reinstalls a package | `3va reinstall axios --allow-net=registry.npmjs.org` |
| `test` | Runs tests | `3va test --coverage` |
| `bundle` | Bundles code | `3va bundle src/index.ts` |
| `dev` | Dev server with HMR | `3va dev --port 3000` |
| `audit` | Security audit | `3va audit --deny` |
| `sandbox` | Interactive REPL | `3va sandbox` |
| `doctor` | Environment check | `3va doctor` |

### 2.2.3 Permission Flags

| Flag | Description |
|------|-------------|
| `--allow-read[=PATH]` | Grants file read access |
| `--allow-write[=PATH]` | Grants file write access |
| `--allow-net[=HOST]` | Grants network access / selects registry |
| `--allow-env` | Grants environment variable access |
| `--allow-child-process` | Grants process spawning |

---

## 2.3 Component: vvva_permissions

### 2.3.1 Description
Implements the deny-by-default capability model. Every I/O operation is gated through `PermissionState::check`.

### 2.3.2 Structure

```rust
pub enum Capability {
    FileRead(PathBuf),
    FileWrite(PathBuf),
    Network(String),
    SpawnProcess,
    EnvAccess,
    FFI,
}

pub struct PermissionState { /* internal, thread-safe */ }
```

### 2.3.3 Verification Algorithm

```
1. Receive (op_type, resource)
2. FOR each capability IN granted:
   3. IF capability matches op_type AND resource → ALLOW
4. IF interactive mode → prompt user
5. DENY
```

---

## 2.4 Component: vvva_js

### 2.4.1 Description
Embeds QuickJS via `rquickjs`. Provides JavaScript/TypeScript execution with built-in Node.js-compatible APIs and an integrated event loop.

### 2.4.2 Built-in APIs

| API | Implementation |
|-----|---------------|
| `console` | `builtins/console.rs` |
| `setTimeout`/`setInterval` | `builtins/timers.rs` + `TimerManager` |
| `fetch` | `builtins/fetch.rs` (HTTP via `ureq`) |
| `fs` | `builtins/fs.rs` (permission-gated) |
| `Buffer` | `builtins/buffer.rs` |
| `process` | `builtins/process.rs` |
| `require()` / ESM | `builtins/modules.rs` + `EsmLoader` |
| `WebSocket` | `builtins/websocket.rs` |

### 2.4.3 Node.js Compatibility Stubs
`http`, `https`, `net`, `tls`, `child_process`, `zlib`, `http2` are registered as no-op stubs so packages that import them don't throw at load time. Full implementations are planned for v0.2.0.

---

## 2.5 Component: vvva_pm

### 2.5.1 Description
Handles dependency installation with mandatory security verification. Network access requires explicit `--allow-net`.

### 2.5.2 Structure

```rust
pub struct PackageManager {
    cache_dir: PathBuf,
    // internal: fetcher, resolver, verifier, lockfile
}
```

### 2.5.3 Security Pipeline
1. `SignatureVerifier` — SHA-256/SHA-512 integrity check
2. `MalwareScanner` — static pattern analysis (fork bombs, curl|sh, crypto miners, backdoors)
3. `SecretsScanner` — 16 secret patterns (AWS, GitHub, Stripe, JWT, PEM, etc.)
4. `Auditor` — OSV CVE query with 24 h local cache

---

## 2.6 Component: vvva_bundler

### 2.6.1 Description
Bundles JS/TS applications from a single entry point. Implemented in pure Rust using the Oxc parser.

### 2.6.2 Features

| Feature | Status |
|---------|--------|
| TypeScript transpilation | ✅ |
| Tree shaking (dead code elimination) | ✅ |
| Code splitting (`--split`) | ✅ |
| Minification (`--minify`) | ✅ |
| Source maps (`--source-map`) | ✅ |
| Watch mode | ✅ |
| Output formats: IIFE, UMD, CJS, ESM | ✅ |

---

## 2.7 Component: vvva_test

### 2.7.1 Description
Jest-compatible test runner. Each test file runs in its own isolated `JsEngine` instance.

### 2.7.2 Features
- `describe`, `test`/`it`, `expect` with 20+ matchers
- Lifecycle hooks: `beforeAll`, `afterAll`, `beforeEach`, `afterEach`
- Snapshot testing (`toMatchSnapshot`, `toMatchInlineSnapshot`)
- Statement-level coverage via Oxc instrumentation
- Watch mode with 500 ms debounce

---

## 2.8 Component: vvva_crypto

### 2.8.1 Description
Post-quantum cryptography utilities. Standalone crate for future TLS integration.

### 2.8.2 Status
- Lamport one-time signatures: implemented
- HKDF key derivation: implemented
- ML-KEM / ML-DSA (NIST PQC): interface defined, implementation returns `NotAvailable` — planned for v0.3.0

---

*Design conforming to IEEE 1012 and component architecture.*
