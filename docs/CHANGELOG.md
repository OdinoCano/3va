# Changelog

All notable changes to **3va** are documented here.
Format: [Keep a Changelog 1.0.0](https://keepachangelog.com/en/1.0.0/) · Versioning: [SemVer](https://semver.org/).

---

## [Unreleased]

### Added

- **ML-KEM-768 (FIPS 203 / Kyber)** — post-quantum Key Encapsulation Mechanism in `vvva_crypto`.
  `MlKemKeypair::generate()`, `encapsulate(&ek)`, `decapsulate(&dk, ct)`. Key sizes: EK=1184 B,
  DK=64 B (seed), CT=1088 B, SS=32 B. Wrong-key decapsulation returns a different shared secret
  (implicit rejection per spec). Hex serialization helpers included.
  (`crates/crypto/src/kem.rs`)

- **ML-DSA-65 (FIPS 204 / Dilithium)** — post-quantum digital signature scheme in `vvva_crypto`.
  `generate_signing_key()`, `sign(&sk, msg)`, `verify(&vk, msg, &sig)`. Key sizes: SK=32 B (seed),
  VK=1952 B, sig=3309 B. Stateless — safe to reuse the same key for multiple messages.
  (`crates/crypto/src/dsa.rs`)

- **`crypto.subtle` (Web Crypto API)** — full `SubtleCrypto` object on `globalThis.crypto.subtle`
  and `require('crypto').subtle`. Backed by `builtins/crypto.rs` (Rust) + JS for HKDF/PBKDF2.
  Supported operations: `digest` (SHA-1/224/256/384/512), `generateKey` (AES-GCM-128/256, AES-CBC,
  AES-CTR, HMAC), `importKey`/`exportKey` (`raw` + `jwk`), `sign`/`verify` (HMAC), `encrypt`/`decrypt`
  (AES-GCM), `deriveBits`/`deriveKey` (HKDF, PBKDF2). `wrapKey`/`unwrapKey` throw `NotSupportedError`.

- **`response.formData()`** — `fetch` responses now parse their body into a `FormData` object.
  Supports `application/x-www-form-urlencoded` (percent-decoding, `+`→space) and
  `multipart/form-data` (boundary splitting, `Content-Disposition` parsing, file parts become `File`
  objects). Any other `Content-Type` rejects with `TypeError`. (`builtins/fetch.rs`)

- **`net` / `tls` — real TCP/TLS sockets** — `require('net')` and `require('tls')` now return
  Rust-backed implementations. `Socket` class wraps `TcpStream` (plain) or `TlsStream` (TLS via
  `native-tls`). API: `connect()`, `write()`, `end()`, `destroy()`, `setEncoding()`, `setTimeout()`,
  `on('data'|'end'|'error'|'close')`, `pipe()`. Server-side `listen()` is not implemented.
  Requires `--allow-net=<host>`. (`builtins/tcp.rs`, `modules.rs`)

- **`http2` client** — `require('http2').connect(authority)` returns an `Http2Session`. Sessions
  expose `request(headers)` which returns an `Http2Request` that emits `response`, `data`, and `end`
  events. NGHTTP2 constants available as `http2.constants`. Backed by `__fetchAsync`; does not
  implement real HTTP/2 framing. (`modules.rs`)

- **`--allow-env=VAR[,VAR,...]` scoped environment access** — `--allow-env` now
  accepts an optional comma-delimited list of variable names.
  - `--allow-env=` (no value) → grants `EnvAccess` (all variables, previous behaviour).
  - `--allow-env=NODE_ENV` → grants `EnvVar("NODE_ENV")` only; all other variables
    are hidden from `process.env`.
  - `--allow-env=NODE_ENV,PORT` → grants only the two listed names.
  - Not providing the flag → `process.env` is an empty object `{}`.
- **`Capability::EnvVar(String)`** — new capability variant for per-variable scoping.
  `EnvAccess` (all) covers any `EnvVar(x)` via `caps_match`; the reverse does not hold.
- **`process.env` permission enforcement** — `process.env` is now populated by
  filtering the host environment through `PermissionState::check(&Capability::EnvVar(key))`
  at injection time. Variables not granted are absent from the object regardless of
  whether they exist in the host environment. Previously all variables were exposed
  unconditionally even without `--allow-env`.

### Fixed

- **Package extraction robustness** (`crates/pm/src/fetcher.rs`) — `PackageFetcher::extract`
  no longer aborts the entire installation on the first entry error. Changes:
  - Entries with `..` or absolute path components are rejected (path traversal).
  - Resolved output paths are verified to stay within the destination directory
    (prevents canonical-escape attacks).
  - `EntryType::Symlink` / `EntryType::Link` are always skipped (supply-chain risk).
  - Directory entries are handled with `create_dir_all` rather than `unpack`.
  - Per-entry IO errors are logged as `WARN` and skipped; extraction continues.
  - Fixes silent install failures for large packages that include native code
    (e.g. `react-native`, `canvas`, `sharp`) — these now extract correctly to
    `node_modules/` instead of being left absent.



- **`3va run` script arguments** — arguments after `--` are forwarded to the script via `process.argv[2+]`. Example: `3va run server.ts -- --port 3000 --dev`. `process.argv[0]` = binary path, `process.argv[1]` = absolute script path, `process.argv[2+]` = script args.
- **`3va install` multi-package** — `install` now accepts multiple packages in one invocation: `3va install next react react-dom`. Previously only one package was accepted.
- **`--allow-net=` without value** — passing `--allow-net=` (empty value via `=`) grants network access to all hosts (equivalent to `*`). Same semantics for `--allow-read=` (all paths) and `--allow-write=` (all paths). Multiple flags can be combined: `3va run app.js --allow-net= --allow-read= --allow-write=`.
- **`process.cwd()`** — returns the real working directory. Previously `undefined`.
- **`process.chdir()`** — no-op stub (sandboxed runtime does not change working directory).
- **`process.nextTick(cb, ...args)`** — schedules `cb` in a microtask via `Promise.resolve().then()`, matching Node.js semantics. Multiple callbacks queued in the same tick are flushed in order.
- **`process.hrtime.bigint()`** — returns `BigInt(Date.now()) * 1_000_000n`.
- **`setImmediate` / `clearImmediate`** — exposed as globals; `setImmediate` is backed by `setTimeout(fn, 0)`.
- **`process.versions` expanded** — now includes `node: "20.0.0"`, `v8: "11.3.244.8-node.20"`, `uv: "1.44.2"`, `zlib: "1.2.13"`, `openssl: "3.0.0"`, `modules: "115"`. Packages that inspect `process.versions.node` no longer crash.
- **`process.stdout.fd` / `process.stderr.fd`** — set to `1` and `2` respectively. `isTTY` set to `false`.
- **`global` / `GLOBAL` globals** — `globalThis.global` and `globalThis.GLOBAL` are now aliases for `globalThis`, unblocking packages that use `global.xxx` (e.g. `node-polyfill-crypto`).
- **`require('module')` shim** — the built-in `module` package now exposes `Module._resolveFilename()`, `Module._cache`, `Module._load()`, `Module.prototype.require()`, `Module.createRequire()`, `Module.createRequireFromPath()`, `Module.builtinModules`, `Module.isBuiltin()`, and `Module.syncBuiltinESMExports()`. Required by Next.js `require-hook.js` and many other packages.
- **`fs` expanded** — 15 new functions (sync + async + `fs.promises`):
  - `existsSync(path)` — now exposed on the `fs` object (was on `globalThis` only).
  - `statSync(path)` / `stat(path[, cb])` — returns a stat object with `isFile()`, `isDirectory()`, `isSymbolicLink()`, `size`, `mode`, `mtime`, `atime`, `ctime`, `birthtime`, `mtimeMs`, `atimeMs`, `ctimeMs`.
  - `lstatSync(path)` / `lstat(path[, cb])` — same as `stat` but does not follow symlinks.
  - `accessSync(path[, mode])` / `access(path[, mode][, cb])` — checks existence and sandbox read/write permissions. `mode` flags: `fs.constants.F_OK` (0), `R_OK` (4), `W_OK` (2), `X_OK` (1).
  - `realpathSync(path)` / `realpath(path[, cb])` — calls `std::fs::canonicalize`.
  - `unlinkSync(path)` / `unlink(path[, cb])` — removes a file.
  - `renameSync(from, to)` / `rename(from, to[, cb])` — moves/renames.
  - `copyFileSync(src, dest)` / `copyFile(src, dest[, cb])` — copies a file.
  - `chmodSync(path, mode)` / `chmod(path, mode[, cb])` — changes Unix permissions.
  - `symlinkSync(target, path)` / `symlink(target, path[, cb])` — creates a symlink.
  - `appendFileSync(path, data)` / `appendFile(path, data[, cb])` — appends to a file.
  - `createReadStream(path[, opts])` — returns an EventEmitter that emits `data`/`end`/`error`. Reads are lazy (fired via `setTimeout(0)` so the event loop can drain first).
  - `createWriteStream(path[, opts])` — returns an object with `write(chunk)` and `end([chunk])`. Flushes the entire buffer to disk on `end()`.
  - `watch(path[, opts][, cb])` — returns an EventEmitter stub (no inotify; sandbox limitation).
  - `readdirSync(path, { withFileTypes: true })` — returns `Dirent`-like objects with `name`, `isFile()`, `isDirectory()`, `isSymbolicLink()`.
  - `fs.constants` — `{ F_OK: 0, R_OK: 4, W_OK: 2, X_OK: 1, COPYFILE_EXCL: 1 }`.
  - `fs.promises.*` — all async methods mirrored (readFile, writeFile, readdir, mkdir, rm, stat, lstat, access, realpath, rename, unlink, copyFile, chmod, symlink, appendFile).
  - `require('fs')` and `require('node:fs')` now return the full expanded object; `require('fs/promises')` returns `fs.promises`.
- **JSX transform** — the Oxc transpiler now supports JSX via the Classic runtime (`React.createElement`):
  - `.jsx` / `.tsx` files: always transformed.
  - `.ts` / `.mts` / `.cts` files: TypeScript strip only (no JSX).
  - `.js` / `.mjs` / unknown extensions: auto-detection via `looks_like_jsx()` heuristic — if the source contains `<Tag` or `</Tag`, JSX transform is applied automatically.
  - JSX fragments use `React.Fragment`.
  - `transpile_jsx(source)` and `transpile_js(source)` are now public API in `vvva_js::transpiler`.
  - `looks_like_jsx(source) -> bool` is public for callers that want to pre-check.
- **Flow type stripping** — `transpile_js()` includes a two-pass Flow fallback:
  1. Strips `@flow`, `@format`, `import type`, `import typeof` pragmas.
  2. If Oxc still fails, falls back to `strip_inline_flow_types()` which removes `: Type` annotations from `const`/`let`/`var` declarations and function parameters at character level (no regex). Enables basic Flow-annotated `.js` files from React Native packages to be loaded via `require()`.

### Changed

- `--allow-read`, `--allow-net`, `--allow-write` in `run`, `install`, `update`, `reinstall` now use `require_equals = true` and `value_delimiter = ','`:
  - **Old:** `--allow-net registry.npmjs.org` (space-separated, consumed next positional arg as value — broken with `--allow-net` followed by FILE).
  - **New:** `--allow-net=registry.npmjs.org` or `--allow-net=host1,host2` (equals sign required; comma-delimited list; omitting value after `=` grants wildcard).
- `process.argv` construction moved from `inject_process` (captured all raw CLI args) to `eval_file` / `eval_file_with_args`:
  - `process.argv[0]` = path to the `3va` binary.
  - `process.argv[1]` = absolute path to the script being run (set just before execution).
  - `process.argv[2+]` = script arguments passed after `--` (set by `eval_file_with_args`).
- `3va install` `package` field renamed from `Option<String>` to `Vec<String>` (`packages`). Backward-compatible: omitting all packages still installs from manifest.

### Fixed

- `--allow-net=` followed immediately by a positional argument (`<FILE>`) no longer silently consumed the file path as the network host value.
- `--allow-read=` and `--allow-write=` combining multiple empty flags in one command (`--allow-net= --allow-read= --allow-write=`) no longer errors with "a value is required".
- `process.argv` no longer duplicated script args when `eval_file_with_args` was called (the raw `std::env::args()` snapshot included the `--` args, causing double-appending).
- `fs.statSync().isFile()` and `fs.statSync().isDirectory()` returned the method function body instead of a boolean (the boolean values from JSON were overwritten before being captured in the closure). Fixed by saving raw booleans before creating method functions.

### Added (previous session)

- `3va dev` — full development server with HMR, SPA fallback, static serving.
- `3va audit --secrets` — Phase 3 audit for hardcoded secrets in dependencies.
- `3va audit --json` — machine-readable JSON output.
- `3va sandbox` — interactive REPL with multi-line support, session commands, TTY detection.
- `3va test --watch` / `--coverage` / `--update-snapshots`.
- `3va bundle --split` / `--minify` / `--source-map`.
- Full ESM support with `EsmResolver` and `EsmLoader`.
- `3va update` with per-package registry tracking.

- `3va dev` — full development server:
  - Flags `--port <N>` (default 3000), `--host <H>` (default 127.0.0.1), `--open`, `--public-dir <D>`.
  - HMR via Server-Sent Events at the `/__hmr` endpoint.
  - HMR client script injected automatically before `</body>` in all served HTML.
  - Static file serving with correct MIME types (15 supported types).
  - SPA fallback: unknown routes serve `public/index.html`.
  - Automatic rebuild with 300 ms debounce when detecting changes in `.js`, `.ts`, `.jsx`, `.tsx` files.
  - Built-in development page when `public/index.html` does not exist.
- `3va audit --secrets` — Phase 3 audit: detection of hardcoded secrets in dependencies (AWS keys, GitHub tokens, PEM private keys, JWT tokens, Stripe keys and other common patterns) via `SecretsScanner`.
- `3va audit --json` — machine-readable output with `{ passed, phases: { malware, osv, secrets } }` structure; completely suppresses human-readable output.
- `audit_packages_silent()` in `vvva_pm` — audit variant without console output, used internally in `--json` mode.
- `3va sandbox` — full interactive REPL:
  - Multi-line support with balanced bracket detection (parentheses, brackets, braces).
  - Session commands: `.help`, `.exit`, `.clear`, `.allow-read <path>`, `.allow-net <host>`, `.permissions`.
  - Node.js-style output formatting: objects as JSON, explicit `undefined` for statements.
  - TTY detection: in pipes and CI environments (stdin non-TTY), exits immediately without blocking.
- `3va test --watch` — automatically re-runs the suite when detecting file changes.
- `3va test --coverage` — line and branch coverage report upon test completion.
- `3va test --update-snapshots` / `-u` — overwrites existing snapshots with current values.
- `3va bundle --split` — code splitting; `--minify` — minification; `--source-map` — source map generation.
- Full ESM: `EsmResolver` and `EsmLoader` in `vvva_js::esm`; `import`/`export` support with relative paths, re-exports and TypeScript modules.
- Full async/await and Promise chain support via the `execute_pending_job` microtask loop.
- Bundler watch mode (`start_watch_mode`) with real `notify` watcher (previously was a stub without implementation).
- `describe` blocks and snapshot support (`toMatchSnapshot`) in the test runner.
- `list_granted()` in `PermissionState` — exposes the list of capabilities granted in the current session.
- `3va update` subcommand with per-package registry tracking.
- `registry` field in `3va-lock.json` (in `packages` and `dependencies`) to record the origin of each installed package.
- Registry preservation logic in the lockfile upon regeneration: registries of already installed packages are not lost when installing new packages.
- `--allow-net` validation in `3va update`: the CLI inspects the lockfile, groups packages by registry and displays the exact command to run if any authorized host is missing.
- Multi-registry support in the same project (e.g., `axios` from `registry.npmjs.org` and `@std/path` from `jsr.io`).
- Methods `registry_for()`, `registries_needed()` and `set_registry()` in `Lockfile` (`crates/pm/src/lockfile.rs`).
- 11 integration tests in `crates/test/tests/runner_integration.rs`.
- 12 unit tests in `crates/pm/src/auditor.rs`.
- 28 tests in `crates/js/tests/pipeline.rs` (ESM, async/await, TypeScript, permissions).
- Integration suite `scripts/integration_tests.sh`: 58 tests in 12 phases (100% passing).

### Fixed

- `is_esm_source()` stopped scanning upon finding the first line that was not an import; now scans the entire file with block comment tracking.
- Snapshot permission failed when the test file was in `/tmp/` (TempDir); now `FileRead`/`FileWrite` is granted to the test file's parent directory.
- `audit --json` emitted human-readable output before JSON because the malware scanner wrote directly to stdout; resolved via `audit_packages_silent()`.
- `run_audit_human` returned before reaching Phase 3 if Phase 1 (malware) or Phase 2 (OSV) produced an error; now all three phases are resilient to individual failures and always execute independently.

---

## [0.1.0-dev] - 2026-05-19

> Active development version. Not yet published as a stable release.

### Added

#### Package Manager (`crates/pm`)
- `3va install <package>[@<version>] --allow-net=<registry-host>` — secure installation from npm, Yarn or JSR.
- `3va reinstall <package> --allow-net=<registry-host>` — forced reinstallation.
- Automatic registry derivation from the host in `--allow-net` (no separate `--registry` flag needed).
- Support for three integrated registries: `registry.npmjs.org`, `registry.yarnpkg.com`, `jsr.io`.
- Support for scoped packages (`@scope/name`, mandatory in JSR).
- Package existence verification before installing.
- Version resolution: if not specified, uses `dist-tags.latest`; if the requested version does not exist, shows the 5 closest by semver distance.
- Version suggestions in `name@version` format.
- Security gate: any attempt to install without `--allow-net` shows an explanatory error and suggests the correct command — no silent network calls.
- Already installed package detection: prevents accidental reinstallation and suggests `reinstall`.
- `package.json` and `3va-lock.json` update after each successful installation.
- Signature verification via `SignatureVerifier` (SHA-256/SHA-512).
- JSR API: `/api/scopes/{scope}/packages/{name}/versions` endpoint.
- Semver distance algorithm: score = `major × 1_000_000 + minor × 1_000 + patch`.

#### CLI (`crates/cli`)
- Subcommands: `run`, `install`, `reinstall`, `update`, `dev`, `bundle`, `test`, `audit`, `doctor`, `sandbox`.
- Global `--accessible` flag for accessible mode (no colors or animations, EN 301 549 compliant).
- Granular permissions in `run`: `--allow-read`, `--allow-write`, `--allow-net`, `--allow-env`, `--allow-child-process`.
- Interactive permission prompt enabled by default in `run`.

#### JavaScript Engine (`crates/js`)
- QuickJS integration via `rquickjs`.
- Automatic TypeScript transpilation when executing `.ts`.
- CommonJS-compatible module system.
- Global APIs: `console`, `fetch`, `fs` (restricted by permissions), timers.

#### Bundler (`crates/bundler`)
- `3va bundle <input> --output <output>` — application bundling.
- TypeScript transpilation in the bundle process.

#### Test Runner (`crates/test`)
- `3va test [paths]` — test suite execution.
- Automatic discovery of `*.test.ts`, `*.test.js`, `*.spec.*` files.

#### Security
- `SignatureVerifier`: SHA-256 and SHA-512 hash calculation and verification of files.
- `MalwareScanner`: static analysis of dependencies.
- `AuditLogger`: sensitive operation logging.
- Interactive prompting for runtime permission requests.
- Post-install scripts disabled by default.

### Changed
- The `--registry` flag was removed from the design. The registry is determined exclusively by the authorized host in `--allow-net` — consistent with 3va's capability model.

### Architecture
- Cargo workspace with crates: `vvva_core`, `vvva_cli`, `vvva_permissions`, `vvva_js`, `vvva_pm`, `vvva_bundler`, `vvva_test`.
- Rust edition 2024.
- Async runtime: Tokio.

---

## Entry format

Each version follows the structure:

```
## [X.Y.Z] - YYYY-MM-DD

### Added        — new functionality
### Changed      — changes in existing functionality
### Deprecated   — functionality to be removed in future versions
### Removed      — removed functionality
### Fixed        — bug fixes
### Security     — vulnerability patches (reference CVE if applicable)
```

---

*Compliant with Keep a Changelog 1.0.0 and SemVer 2.0.0.*
