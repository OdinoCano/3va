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
| `fs` | **Real** — permission-gated | `builtins/fs.rs` |
| `Buffer` | **Real** | `builtins/buffer.rs` |
| `process` | **Real** | `builtins/process.rs` |
| `fetch` | **Real** | `builtins/fetch.rs` |
| `WebSocket` | **Real** | `builtins/websocket.rs` |
| `setTimeout`/`setInterval`/`clearTimeout`/`clearInterval` | **Real** | `builtins/timers.rs` |
| `zlib` | **Real** — async only | `builtins/zlib.rs` + `flate2` |
| `child_process` | **Real** — requires `--allow-child-process` | `builtins/child_process.rs` |
| `http` / `https` | **Partial stub** — client requests only via `__fetchAsync` | `builtins/modules.rs` |
| `net` / `tls` | **Stub** — no-op emitters, no real TCP | `builtins/modules.rs` |
| `http2` | **Stub** — no-op | `builtins/modules.rs` |
| `events` | **JS implementation** | `builtins/modules.rs` |
| `stream` | **JS implementation** | `builtins/modules.rs` |
| `path` | **JS implementation** | `builtins/modules.rs` |
| `url` | **JS implementation** | `builtins/modules.rs` |
| `util` | **JS implementation** | `builtins/modules.rs` |
| `crypto` | **Minimal stub** — no real crypto primitives | `builtins/modules.rs` |
| `os` | **Stub** — static hardcoded values | `builtins/modules.rs` |

### 2.2 APIs That THROW — Never Suggest These

The following methods exist in the require cache but **throw at call time**. The AI must never suggest them as working alternatives:

| Call | Error thrown | Use instead |
|------|-------------|-------------|
| `child_process.execSync(cmd)` | `execSync is not available in 3va` | `exec()` with callback or `promisify` |
| `child_process.spawnSync(cmd)` | `spawnSync is not available in 3va` | `spawn()` |
| `zlib.gzipSync(buf)` | `gzipSync not available` | `zlib.gzip(buf, cb)` |
| `zlib.gunzipSync(buf)` | `gunzipSync not available` | `zlib.gunzip(buf, cb)` |
| `zlib.deflateSync(buf)` | `deflateSync not available` | `zlib.deflate(buf, cb)` |
| `zlib.inflateSync(buf)` | `inflateSync not available` | `zlib.inflate(buf, cb)` |

### 2.3 APIs That Are Stubs (No-Op) — Never Document as Functional

| Call | Actual behavior |
|------|----------------|
| `zlib.createGzip()` | Returns `{}` — not a real Transform stream |
| `zlib.createGunzip()` | Returns `{}` |
| `zlib.createDeflate()` | Returns `{}` |
| `zlib.createInflate()` | Returns `{}` |
| `http.createServer()` | Returns `{ listen: fn, close: fn }` — never fires callbacks |
| `net.createConnection()` | Returns bare `EventEmitter` — no real socket |
| `crypto.createHash()` | Returns an object whose `.digest('hex')` returns base64 of the raw string, **not** a real hash |

### 2.4 Behavior Differences from Node.js

The AI must document and respect these deviations when writing code or docs:

- **`child_process.spawn()` is not streaming.** All stdout/stderr is delivered in one `data` event after the process exits. There is no real byte-level streaming mid-execution.
- **`zlib` callbacks receive `Uint8Array`, not `Buffer`.** Wrap with `Buffer.from(result)` if Buffer methods are needed.
- **`http.request()` / `http.get()` make real HTTP calls** via the internal `__fetchAsync` primitive, but `http.createServer()` is a no-op stub.
- **`crypto.randomBytes(n)`** uses `Math.random()`, not a CSPRNG. It is **not cryptographically secure**.
- **`crypto.createHash()`** does NOT implement SHA-256 or any real hash. It returns `btoa(data)` which is base64 encoding. Do not use for security-sensitive operations.

### 2.5 Internal Primitives — Never Expose to Users

The following globals are implementation details injected by Rust builtins. The AI must not document them as public API or suggest users call them directly:

- `__execAsync(cmd, args, timeout_ms)` → used by `child_process.execFile`/`spawn`
- `__execShellAsync(command)` → used by `child_process.exec`
- `__zlibGzip`, `__zlibGunzip`, `__zlibDeflate`, `__zlibInflate`, `__zlibRawDeflate`, `__zlibRawInflate`
- `__fetchAsync(url, method, headers, body)`
- `__readFile(path)`, `__resolvePath(path, basedir)`

---

## 3. Anti-Hallucination: CLI Flags and Options

### 3.1 Permission Flags That Actually Exist

The AI must only reference flags that are defined in `crates/cli/src/main.rs` and enforced by `crates/permissions/`. Invented flags cause silent security bypasses.

| Flag | Grants |
|------|--------|
| `--allow-read[=<path>]` | File read access |
| `--allow-write[=<path>]` | File write access |
| `--allow-net[=<host>]` | Network access (fetch, WebSocket) |
| `--allow-env` | `process.env` access |
| `--allow-child-process` | `child_process` exec/spawn |
| `--allow-all` / `-A` | All permissions (dangerous) |
| `--accessible` | Enable accessible output mode (EN 301 549) |

**Flags that do NOT exist** (do not invent them): `--allow-ffi`, `--allow-hrtime`, `--allow-sys`, `--allow-plugin`, `--allow-run` (the correct flag is `--allow-child-process`).

### 3.2 Permission Checks in Rust

When writing new Rust builtins that need permission checks, use the existing `PermissionState::check()` API:

```rust
if !permissions.check(&Capability::SpawnProcess) {
    return Err(rquickjs::Error::new_from_js_message(
        "permission", "permission",
        "Denied. Run with --allow-child-process".to_string(),
    ));
}
```

Available `Capability` variants (defined in `crates/permissions/src/capability.rs`):
- `Capability::FileRead(PathBuf)`
- `Capability::FileWrite(PathBuf)`
- `Capability::NetworkAccess(String)` — host string
- `Capability::EnvAccess`
- `Capability::SpawnProcess`

Do not invent new `Capability` variants without adding them to the permissions crate.

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

### 4.2 Where to Add New Built-in Modules

If the user asks to add a new Node.js built-in:
1. Create `crates/js/src/builtins/<name>.rs` with an `inject_<name>(ctx: &Ctx, ...) -> Result<()>` function.
2. Call it from `crates/js/src/lib.rs` in the runtime setup, after the `inject_require` call.
3. Register the JS-level object in `globalThis.__requireCache['<name>']` at the end of the Rust inject function (see `child_process.rs` and `zlib.rs` as reference).
4. Update `docs/04-core/02-modulo-system.md` Module Status table.
5. Update `docs/02-arquitectura/02-diseno-componentes.md` Built-in APIs table.

---

## 5. Verification Before Claiming Something Works

Before stating that an API, flag, or feature is available, the AI must verify:

1. **For JS APIs**: check that the method exists in `builtins/modules.rs` require cache OR in a dedicated `builtins/<name>.rs` inject function.
2. **For CLI flags**: check `crates/cli/src/main.rs` `#[command]` / `#[arg]` definitions.
3. **For Capability variants**: check `crates/permissions/src/capability.rs`.
4. **For crate imports**: check `Cargo.toml` `[dependencies]` — never assume a crate is available without verifying it is listed.

> **Rule**: If you cannot locate the implementation in the source, say "this is not yet implemented" rather than generating code that will silently fail or produce wrong results.
