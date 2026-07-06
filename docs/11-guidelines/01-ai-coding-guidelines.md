# 01 - DEVELOPMENT GUIDELINES FOR AI (AI GUIDELINES)

Any Artificial Intelligence system (LLMs, Code Assistants) or human developer generating code for the **3va** project MUST strictly adhere to the following rules.

---

## 1. Golden Rule: Terminal Accessibility by Default

The `3va` runtime is used by developers with visual impairments who depend on *Braille Displays* and *Screen Readers*. The CLI output must respect the global accessibility module `crates/cli/src/accessibility.rs`.

### 1.1 Excessive ASCII Art Prohibited
If the CLI needs to print boxes, tables or decorative elements, the AI MUST wrap such logic in a check of the `--accessible` flag.
Characters like `┌`, `─`, `│`, `└` overwhelm Braille hardware, which will read them literally.
- **Incorrect:** Always print tables drawn with bars.
- **Correct:** Check `accessibility::is_accessible_mode()` and if true, print a flat list.

### 1.2 Careful with Animations (Spinners/Progress bars)
When creating commands that take time (e.g. `3va install`, `3va bundle`), it is forbidden to send animations that depend on continuous carriage returns (`\r`) if accessible mode is active.
- A continuous carriage return freezes the Braille line.
- In accessible mode, print static messages: `INFO: Downloading...` and `INFO: Download complete`.

### 1.3 Colors are NOT Exclusive Semantics
The logging system (`tracing`) is already configured to disable ANSI codes (`NO_COLOR`) in accessible mode. Therefore, the AI must never use "colors" as the only way to denote a status (e.g. painting text red assuming the user will understand it is an error).
- **Mandatory:** Always prepend clear text prefixes like `ERROR:`, `WARN:`, `SUCCESS:`.

### 1.4 Implementation Example

When adding logic to `main.rs` or CLI commands:

```rust
// CORRECT
let is_accessible = accessibility::is_accessible_mode(cli.accessible);

if is_accessible {
    println!("INFO: Compiling project...");
} else {
    // Here you can use spinners, indicatif crate, or ASCII tables
    start_spinner("Compiling...");
}
```

> **AI Attention**: If the user asks you to "make the console prettier" or "add a progress bar", you **must** respect these guidelines and include the `if` branch for accessible mode.

---

## 2. Anti-Hallucination: Module API Reality Map

The AI MUST verify that any API it references, documents, or generates actually exists in the current codebase. This section is the authoritative reference. **If an API is not listed here, it does not exist — do not invent it.**

### 2.1 Module Status

| Module | Status | Source |
|--------|--------|--------|
| `console` | **Real** — full implementation | `builtins/console.rs` |
| `fs` | **Real** — permission-gated; full sync+async+promises+streams API | `builtins/fs.rs` |
| `Buffer` | **Real** — prototype-swapped subclass of `Uint8Array` | `builtins/buffer.rs` |
| `process` | **Real** — full API including memoryUsage, cpuUsage, and signals | `builtins/process.rs` |
| `fetch` | **Real** — accepts `Request` or URL string, returns `Response` | `builtins/fetch.rs` |
| `Request` / `Response` / `Headers` | **Real** — full WinterCG-compatible classes | `builtins/modules.rs` |
| `WebSocket` | **Real** — requires `--allow-net` | `builtins/websocket.rs` |
| `setTimeout`/`setInterval`/`clearTimeout`/`clearInterval` | **Real** | `builtins/timers.rs` |
| `zlib` | **Real** — sync + async + Transform streams (gzip, gunzip, deflate, inflate, deflateRaw, inflateRaw, etc.) | `builtins/zlib.rs` + `flate2` |
| `child_process` | **Real** — sync + async (exec, spawn, execFile, execSync, spawnSync); requires `--allow-child-process` | `builtins/child_process.rs` |
| `http` / `https` — client | **Real** — `request()`, `get()` backed by `__fetchAsync` | `builtins/modules.rs` |
| `http.createServer()` | **Real** — real Tokio TCP listener, HTTP/1.1 parser; requires `--allow-net` | `builtins/http_server.rs` |
| `net.createServer()` | **Real** — raw TCP server via `__netListen`/`__netAcceptAsync`; requires `--allow-net` | `builtins/tcp.rs` |
| `net.connect()` / `Socket` | **Real** — `TcpStream`-backed; requires `--allow-net` | `builtins/tcp.rs` |
| `tls.connect()` / `TlsSocket` | **Real** — `TlsStream` via `native-tls`; requires `--allow-net` | `builtins/tcp.rs` |
| `crypto.subtle` / `crypto.webcrypto` | **Real** — digest, generateKey, importKey, exportKey, sign/verify, encrypt/decrypt, deriveBits/deriveKey | `builtins/crypto.rs` |
| `crypto.getRandomValues` | **Real** — CSPRNG backed by Rust `rand::RngCore` | `builtins/crypto.rs` |
| `crypto.randomBytes(n)` | **Real** — CSPRNG, returns `Buffer`; **not** `Math.random()` | `builtins/crypto.rs` |
| `crypto.randomUUID()` | **Real** — CSPRNG UUID v4 | `builtins/crypto.rs` |
| `crypto.createHash(alg)` | **Real** — SHA-1/256/384/512; `.update(data)` / `.digest(enc)` returns a `Promise` | `builtins/crypto.rs` |
| `crypto.createHmac(alg, key)` | **Real** — HMAC-SHA variants; `.update(data)` / `.digest(enc)` returns a `Promise` | `builtins/crypto.rs` |
| `crypto.timingSafeEqual(a, b)` | **Real** — constant-time comparison | `builtins/crypto.rs` |
| `crypto.pbkdf2(...)` / `pbkdf2Sync()` | **Real** — PBKDF2 KDF derivation (async and sync forms) | `builtins/crypto.rs` |
| `crypto.scrypt(...)` / `scryptSync()` | **Real** — scrypt KDF derivation (async and sync forms) | `builtins/crypto.rs` |
| `crypto.createCipheriv()` / `createDecipheriv()` | **Real** — AES-128-GCM and AES-256-GCM streaming encryption/decryption | `builtins/crypto.rs` |
| `crypto.generateKeyPair()` / `generateKeyPairSync()` | **Real** — asymmetric RSA and EC key pair generation | `builtins/crypto.rs` |
| `crypto.createPrivateKey()` / `createPublicKey()` / `createSecretKey()` | **Real** — imports PEM keys into compatible KeyObject wrappers | `builtins/crypto.rs` |
| `worker_threads` | **Real** — each `new Worker(file)` spawns a real OS thread with its own `JsEngine`; `postMessage` passes JSON via `std::sync::mpsc`; `SharedArrayBuffer`/`Atomics` are **not supported** (non-goal: isolated heaps) | `builtins/worker_threads.rs` |
| `cluster` | **Real** — single-process emulation; `isPrimary: true`, `fork()` returns a mock `ClusterWorker` that emits `online`/`exit` so `if (cluster.isPrimary)` guards work | `builtins/modules.rs` |
| `http2` | **Partial stub** — client API backed by `__fetchAsync`; no real HTTP/2 framing | `builtins/modules.rs` |
| `events` | **Real** — full `EventEmitter` class (prependListener, rawListeners, etc.) | `builtins/modules.rs` |
| `stream` | **JS implementation** — `Readable`/`Writable`/`Transform` | `builtins/modules.rs` |
| `path` | **Real** — full Node.js-compatible path utilities (relative, normalize, resolve, etc.) | `builtins/modules.rs` |
| `url` | **Real** — `URL`, `URLSearchParams`; full parsing + iteration | `builtins/modules.rs` |
| `util` | **Real** — `inherits`, `promisify`, `format`, `inspect` (circular refs), `parseArgs` (Node 18+) | `builtins/modules.rs` |
| `os` | **Real** — system metrics (Linux: hostname, cpus, totalmem, freemem, uptime, platform/arch, constants) | `builtins/modules.rs` |
| `structuredClone` | **Real** — JSON-based deep clone; throws `DataCloneError` for non-serializable | `builtins/modules.rs` |
| `navigator` | **Real** — `userAgent`, `language`, `languages`, `onLine`, `hardwareConcurrency` | `builtins/modules.rs` |
| `AbortController` / `AbortSignal` | **Real** — integrates with `fetch` via `Promise.race` | `builtins/modules.rs` |
| `Blob` / `File` | **Real** — `text()`, `arrayBuffer()`, `bytes()`, `slice()`, `stream()` | `builtins/modules.rs` |
| `FormData` | **Real** — `append`, `set`, `get`, `getAll`, `delete`, `forEach`, iteration | `builtins/modules.rs` |
| `ReadableStream` / `WritableStream` / `TransformStream` | **Real** — WinterCG pull model | `builtins/modules.rs` |
| `FileReader` | **Real** — `readAsText`, `readAsDataURL`, `readAsArrayBuffer`, `abort` | `builtins/modules.rs` |
| `ffi` (native libs) | **Real** — loads shared libraries via `dlopen`; requires `--allow-ffi=<path>` | `builtins/ffi.rs` |
| `napi` (`.node` addons) | **Real** — ~30 NAPI v8 functions; `require('./addon.node')` delegates to `__napiRequire`; requires `--allow-ffi` | `builtins/napi.rs` |

### 2.2 APIs That THROW — Never Suggest These as Working

The following methods exist in the require cache but **throw at call time**. The AI must never suggest them as working alternatives:

| Call | Error thrown | Use instead |
|------|-------------|-------------|
| `crypto.subtle.wrapKey()` | `wrapKey not implemented` (`NotSupportedError`) | use `exportKey` + encrypt manually |
| `crypto.subtle.unwrapKey()` | `unwrapKey not implemented` (`NotSupportedError`) | use `importKey` + decrypt manually |
| `crypto.createHash().copy()` | `Hash.copy() is not supported` | create a new `createHash()` instance |
| `fs.watch()` | **implemented** — inotify-backed on Linux; `crates/js/tests/fs_watch.rs` covers the full event cycle | — |

### 2.3 APIs That Are Partial Stubs — Document Limitations

| Call | Actual behavior |
|------|----------------|
| `http2.connect()` | Client API only; backed by `__fetchAsync`, no real HTTP/2 framing |
| `process.chdir()` | No-op stub; sandboxed runtime does not change working directory |
| `fs.watch()` | inotify-backed; emits `change`/`rename` events; requires `FileRead` permission on the watched path |
| `AbortController.abort()` | Races against `__fetchAsync` via `Promise.race`; cannot cancel in-flight I/O at socket level |
| `tty.isatty(fd)` | Calls real `__isatty` (Rust `std::io::IsTerminal`); `process.stdout.isTTY` / `process.stdin.isTTY` reflect actual TTY state |
| `v8.getHeapStatistics()` | Returns a zeroed object; `getHeapSpaceStatistics()` returns `[]` |
| `vm.runInNewContext(code, sandbox)` | Uses `with(sandbox)` so sandbox vars shadow globals; `globalThis`/`require`/`process` still reachable — no real V8-style context isolation. Use `worker_threads` for process-level sandboxing |
| `dns.resolveMx/Txt/Srv/Ns/Cname/Naptr/Ptr/Soa/reverse` | Real DNS queries via `hickory-resolver` (native `__dnsQuery`) |
| `dns.lookupService(addr, port, cb)` | Callback receives `ENOTSUP` error |
| `readline.createInterface()` | Backed by real `process.stdin` (native `__stdinRead`); `Interface` consumes Node-style `'data'` events or a WHATWG `getReader()` |
| `irc.Client` | **Real** — `TcpStream`/TLS connect, RFC 2812 line protocol (PING→PONG, PRIVMSG parsing) |
| `ftp.Client` | **Real** — `TcpStream`/TLS connect, RFC 959 commands (USER/PASS auth, PASV data channel, LIST/RETR/STOR) |
| `pop3.Client` | **Real** — `TcpStream`/TLS connect, RFC 1939 line protocol (USER/PASS, LIST/RETR/DELE) |
| `mqtt.connect()` | **Real** — `TcpStream`/TLS connect, MQTT 3.1.1 binary protocol (QoS 0 only, no keepalive PINGREQ) |
| `ssh.Client` | **Real** — `russh`/`russh-sftp`, password auth only (no public-key), no host key verification (accepts any server key) |
| `webrtc.RTCPeerConnection` | **Mocked** — API shape only; no real ICE/DTLS/SRTP (requires STUN/TURN servers for P2P) |
| `worker_threads.Worker` | No `SharedArrayBuffer`/`Atomics` — all data sharing must use `postMessage` |
| `repl`, `wasi`, `trace_events` | Not implemented; `require()` throws `MODULE_NOT_FOUND` |

### 2.4 Behavior Differences from Node.js

The AI must document and respect these deviations when writing code or docs:

- **`child_process.spawn()` is not streaming.** All stdout/stderr is delivered in one `data` event after the process exits. There is no real byte-level streaming mid-execution.
- **`zlib` callbacks receive `Uint8Array`, not `Buffer`.** Wrap with `Buffer.from(result)` if `Buffer` methods are needed.
- **`crypto.createHash().digest(enc)`** returns a `Promise` (async), not a synchronous string. Await it.
- **`crypto.createHmac().digest(enc)`** same: returns a `Promise`.
- **`http.createServer()` handler** receives Node.js-compatible `IncomingMessage` and `ServerResponse`, but the connection model is sequential (one request at a time per accept loop iteration), not fully concurrent.
- **`net.createServer()` socket** — `write(data)` accepts `string` or `Uint8Array`; `on('data')` receives `Uint8Array`, not `Buffer`.
- **`fs.createReadStream()`** starts emitting `data` events automatically when a `'data'` listener is registered (flowing mode), matching Node.js behavior.

### 2.5 Internal Primitives — Never Expose to Users

The following globals are implementation details injected by Rust builtins. The AI must not document them as public API or suggest users call them directly:

- `__execAsync(cmd, args, timeout_ms)` — used by `child_process.execFile`/`spawn`
- `__execShellAsync(command)` — used by `child_process.exec`
- `__zlibGzip`, `__zlibGunzip`, `__zlibDeflate`, `__zlibInflate`, `__zlibRawDeflate`, `__zlibRawInflate`
- `__fetchAsync(url, method, headers, body)` — used by `fetch` and `http.request()`
- `__httpListen`, `__httpAcceptAsync`, `__httpRespond`, `__httpClose` — used by `http.createServer()`
- `__netListen`, `__netAcceptAsync`, `__netWrite`, `__netClose` — used by `net.createServer()`
- `__tcpConnect`, `__tcpConnectTls`, `__tcpRead`, `__tcpWrite`, `__tcpClose` — used by `net.connect()` / `tls.connect()`
- `__fsReadFileSync`, `__fsWriteFileSync`, `__fsStatSync`, `__fsMkdirSync`, etc. — used by `fs`
- `__napiRequire(path)` — used by `require('./addon.node')` (NAPI loader); do not call directly

---

## 3. Anti-Hallucination: CLI Flags and Options

### 3.1 Permission Flags That Actually Exist

The AI must only reference flags that are defined in `crates/cli/src/main.rs` and enforced by `crates/permissions/`. Invented flags cause silent security bypasses.

| Flag | Grants |
|------|--------|
| `--allow-read[=<path>]` | File read access (`Capability::FileRead(PathBuf)`) |
| `--allow-write[=<path>]` | File write access (`Capability::FileWrite(PathBuf)`) |
| `--allow-net[=<host>]` | Network access (`Capability::Network(String)`) — fetch, WebSocket, http server, net |
| `--allow-env[=VAR,...]` | `process.env` access; no value = all vars, with value = scoped vars only |
| `--allow-child-process` | `child_process` exec/spawn (`Capability::SpawnProcess`) |
| `--allow-ffi[=<path>]` | FFI / NAPI native library loading (`Capability::FFI(PathBuf)`); no value = all paths; required by `require('./addon.node')` |
| `--allow-all` / `-A` | All permissions (dangerous) |
| `--accessible` | Enable accessible output mode (EN 301 549) |

**Flags that do NOT exist** (do not invent them): `--allow-hrtime`, `--allow-sys`, `--allow-plugin`, `--allow-run` (correct flag is `--allow-child-process`).

### 3.2 Permission Checks in Rust

When writing new Rust builtins that need permission checks, use the existing `PermissionState::check()` API:

```rust
if !permissions.check(&Capability::Network(host.clone())) {
    return Err(rquickjs::Error::new_from_js_message(
        "permission", "permission",
        format!("Denied. Run with --allow-net={host}"),
    ));
}
```

Available `Capability` variants (defined in `crates/permissions/src/capability.rs`):

| Variant | Usage |
|---------|-------|
| `Capability::FileRead(PathBuf)` | Read access to a path prefix |
| `Capability::FileWrite(PathBuf)` | Write access to a path prefix |
| `Capability::Network(String)` | Network access to a host string |
| `Capability::EnvAccess` | All environment variables |
| `Capability::EnvVar(String)` | Single named environment variable |
| `Capability::SpawnProcess` | Spawn child processes |
| `Capability::FFI` | Foreign function interface (reserved) |

> **CRITICAL**: The variant is `Capability::Network(String)`, **not** `Capability::NetworkAccess(String)`. Using the wrong name causes a compile error.

Do not invent new `Capability` variants without adding them to `crates/permissions/src/capability.rs` and updating `caps_match`.

---

## 4. Anti-Hallucination: Crate Structure

### 4.1 Crate Names

| Directory | Crate name (in Cargo.toml) |
|-----------|---------------------------|
| `crates/cli` | `vvva_cli` |
| `crates/core` | `vvva_core` |
| `crates/js` | `vvva_js` |
| `crates/permissions` | `vvva_permissions` |
| `crates/pm` | `vvva_pm` |
| `crates/bundler` | `vvva_bundler` |
| `crates/test` | `vvva_test` |
| `crates/crypto` | `vvva_crypto` |
| `crates/firewall` | `vvva_firewall` |
| `crates/wasm` | `vvva_wasm` |
| `crates/config` | `vvva_config` |

### 4.2 Where to Add New Built-in Modules

If the user asks to add a new Node.js built-in:
1. Create `crates/js/src/builtins/<name>.rs` with an `inject_<name>(ctx: &Ctx, ...) -> Result<()>` function.
2. Register in `crates/js/src/builtins/mod.rs`: add `pub mod <name>;` and call `<name>::inject_<name>(ctx, permissions.clone())?;` in `inject_all`, **after** `inject_require` so it can overwrite the placeholder stub.
3. Register the JS-level object in `globalThis.__requireCache['<name>']` at the end of the Rust inject function (see `child_process.rs`, `zlib.rs`, `tcp.rs` as reference).
4. Update `docs/04-core/02-modulo-system.md` Module Status table.
5. Update `docs/11-guidelines/01-ai-coding-guidelines.md` §2.1 Module Status.
6. Update `docs/CHANGELOG.md` Added section.

### 4.3 Injection Order in `inject_all`

The injection order in `crates/js/src/builtins/mod.rs` matters. The current order is:

```
console → timers → buffer → process → global alias → fetch → fs → tcp
  → http_server → modules::inject_require → websocket → zlib → child_process
  → crypto → ffi → napi
```

**Rule**: any builtin that overwrites a `modules.rs` stub must be called **after** `inject_require`. Builtins that provide primitives needed by the JS stubs (e.g. `http_server`, `tcp`) must be called **before** `inject_require`.

---

## 5. Verification Before Claiming Something Works

Before stating that an API, flag, or feature is available, the AI must verify:

1. **For JS APIs**: check that the method exists in `builtins/modules.rs` require cache OR in a dedicated `builtins/<name>.rs` inject function.
2. **For CLI flags**: check `crates/cli/src/main.rs` `#[command]` / `#[arg]` definitions.
3. **For Capability variants**: check `crates/permissions/src/capability.rs`.
4. **For crate imports**: check `Cargo.toml` `[dependencies]` — never assume a crate is available without verifying it is listed.

> **Rule**: If you cannot locate the implementation in the source, say "this is not yet implemented" rather than generating code that will silently fail or produce wrong results.

---

## 6. Quality Gates

All code contributions must pass the quality pipeline before merging:

```bash
# Formatting (non-negotiable)
cargo fmt --check

# Lints (zero warnings policy)
cargo clippy --all-targets -- -D warnings

# Tests (all must pass)
cargo test

# Documentation (no broken links or missing docs on pub items)
cargo doc --no-deps --document-private-items 2>&1 | grep -c "^warning:" || true

# Coverage gate (run via scripts/security_verify.sh)
cargo tarpaulin --out Lcov --skip-clean

# Mutation testing (spot-check critical paths)
cargo mutants -p vvva_permissions -p vvva_js
```

Full pipeline: `bash scripts/security_verify.sh`
Git hooks: installed automatically on `cargo test` via `cargo-husky` (see §7).

---

## 7. Git Hooks (cargo-husky)

Pre-commit and pre-push hooks are managed by `cargo-husky`. They are installed automatically when `cargo test` runs for the first time.

Hook scripts live in `.cargo-husky/hooks/`:

| Hook | Runs |
|------|------|
| `pre-commit` | `cargo fmt --check` + `cargo clippy -D warnings` |
| `pre-push` | `cargo test` |

If hooks are not installed, run:
```bash
cargo test -p vvva_js --test crypto_module
```

This triggers cargo-husky's hook installation step.
