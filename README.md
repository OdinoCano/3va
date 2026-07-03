# 3va

[![CI](https://github.com/OdinoCano/3va/actions/workflows/ci.yml/badge.svg)](https://github.com/OdinoCano/3va/actions/workflows/ci.yml)
[![Security](https://github.com/OdinoCano/3va/actions/workflows/security.yml/badge.svg)](https://github.com/OdinoCano/3va/actions/workflows/security.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust Edition 2024](https://img.shields.io/badge/Rust-2024-orange.svg)](https://doc.rust-lang.org/edition-guide/)

> *Veni, Vidi, Vici, Abiit — He came, he saw, he conquered, he left.*

**3va** is a JavaScript and TypeScript runtime written in Rust with deny-by-default security. It bundles a package manager, process manager, test runner, bundler, and dev server — no pm2, no separate build tool, no extra config.

---

## Philosophy

The JavaScript ecosystem has a supply chain security problem. Post-install scripts run arbitrary code at install time. Packages silently access the filesystem and network. There is no enforced boundary between trusted application code and untrusted dependencies.

3va starts from a different premise: deny everything, grant explicitly. The design draws from QubesOS, WASI, and the Chrome sandbox — not from Node.js. Every capability (filesystem, network, env vars, child processes, native addons) is blocked by default and must be declared at the command line. This applies uniformly to application code and to every dependency it pulls in.

---

## Comparison

| Feature | Node.js 25 | Bun 1.3 | **3va 2.0** |
|---|---|---|---|
| JavaScript runtime | ✓ | ✓ | ✓ |
| TypeScript (no config) | ✗ | ✓ | ✓ |
| Package manager | via npm | ✓ | ✓ |
| Process manager | via pm2 | ✗ | ✓ built-in |
| Test runner | via Jest | ✓ | ✓ Jest-compatible |
| Bundler | via webpack/esbuild | ✓ | ✓ |
| Dev server + HMR | ✗ | ✓ | ✓ |
| Deny-by-default permissions | ✗ | ✗ | ✓ |
| Post-install scripts blocked | ✗ | ✗ | ✓ always |
| Malware + secrets audit | ✗ | ✗ | ✓ |
| OSV CVE scan (24h cache) | ✗ | ✗ | ✓ |
| CDP Debugger (`--inspect`) | ✓ | ✗ | ✓ |
| NAPI native addons | ✓ | ✓ | ✓ |
| WebAssembly (WASI) | partial | ✓ | ✓ |
| Post-quantum TLS (ML-KEM-768) | ✗ | ✗ | ✓ |
| Startup time (hello world) | 175 ms | **16 ms** | 94 ms¹ |
| HTTP throughput (100k req) | — ² | **20,758 req/s** | 1,572 req/s¹ |
| Memory (minimal HTTP server) | 44 MB | 32 MB | **29 MB** |
| Install time (warm cache) | 984 ms | **7.8 ms** | 16.8 ms |

¹ Measured with a debug build (`cargo build`, not `--release`). Release performance is higher.  
² Node.js was not running an HTTP server during the throughput measurement; connection refused on all requests.

**Where Bun wins:** startup latency and raw HTTP throughput. Bun's JSC engine and native HTTP stack are faster in both metrics. 3va trades throughput for security guarantees that Bun does not provide.

**Where 3va wins:** memory footprint, integrated security tooling, and permission isolation that applies at runtime — not just at install time.

---

## Installation

### npm (all platforms)

```bash
npm install -g @edge_166/3va
```

### Scoop (Windows)

```bash
scoop bucket add 3va https://github.com/OdinoCano/3va
scoop install 3va
```

### Chocolatey (Windows)

```bash
choco install 3va
```

### winget (Windows)

```bash
winget install OdinoCano.3va
```

### Snap (Linux)

```bash
snap install vvva-cli
```

Build locally from source (for testing):

```bash
snapcraft --use-lxd   # uses dist/snap/snapcraft.yaml
sudo snap install ./vvva-cli_*.snap --dangerous
```

### Nix

The flake lives under `dist/nix/`:

```bash
nix run "github:OdinoCano/3va?dir=dist/nix"
```

### Termux (Android)

```bash
bash <(curl -fsSL https://github.com/OdinoCano/3va/releases/latest/download/termux-install.sh)
```

### Build from source

```bash
git clone https://github.com/OdinoCano/3va.git
cd 3va
cargo build --release
sudo cp target/release/3va /usr/local/bin/
```

### Direct binary download

Pre-built binaries for every platform are attached to each [GitHub Release](https://github.com/OdinoCano/3va/releases/latest):

| Platform | File |
|----------|------|
| Linux x64 | `3va-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` |
| Linux arm64 | `3va-vX.Y.Z-aarch64-unknown-linux-gnu.tar.gz` |
| macOS x64 | `3va-vX.Y.Z-x86_64-apple-darwin.tar.gz` |
| macOS arm64 (M-series) | `3va-vX.Y.Z-aarch64-apple-darwin.tar.gz` |
| Windows x64 | `3va-vX.Y.Z-x86_64-pc-windows-msvc.zip` |
| Android arm64 | `3va-vX.Y.Z-aarch64-linux-android.tar.gz` |

---

## Quick Start

```bash
git clone https://github.com/OdinoCano/3va.git
cd 3va
cargo build --release
sudo cp target/release/3va /usr/local/bin/
```

```bash
# Run a script (no permissions granted by default)
3va run app.ts        # or: 3va r app.ts

# Grant specific capabilities
3va run app.ts --allow-net=api.example.com --allow-read=./data

# Install packages
3va install express --allow-net=registry.npmjs.org   # or: 3va i express …

# Audit the dependency tree
3va audit --secrets

# Start as a background daemon
3va start app.js --name my-api
```

> **Common aliases:** `r`=run · `i`/`add`=install · `t`/`spec`=test · `d`=dev · `b`=bundle · `ws`=workspace · `sh`/`shell`=sandbox
>
> **Output:** `3va run` only prints your script's own output. Pass `-v` / `--verbose` to also see runtime status messages.

---

## Permissions

Every capability is blocked by default. Permissions are granted per-invocation via flags and apply equally to application code and all loaded dependencies.

```bash
3va run app.ts \
  --allow-read=/app/config \        # filesystem read, scoped to a path
  --allow-write=/tmp \              # filesystem write, scoped to a path
  --allow-net=api.stripe.com \      # outbound network, scoped to a host
  --allow-env=DATABASE_URL \        # env var access, scoped to a variable
  --allow-child-process \           # spawn child processes
  --allow-ffi=./build/addon.node    # load native addon (NAPI)
```

Omitting a flag means the capability is blocked and cannot be enabled from inside the script. In an attended terminal (stderr is a TTY), the runtime asks interactively at the point of first access; in CI, pipes, or redirected output, ungranted capabilities are denied silently.

Permission scopes can be widened to cover all values:

```bash
3va run app.ts --allow-read --allow-net  # unrestricted read + network
```

### Permission analysis commands

Two commands help derive the minimum permission set:

```bash
3va permissions suggest          # static analysis of source files → suggested flags
3va permissions learn app.ts     # run with all permissions, report which were used
```

### Package-level permission declarations

Besides CLI flags, `3va run` reads permission grants from `package.json` under
a `"3va"` key — a pattern already established by `"jest"`, `"eslint"`, and
`"prettier"`. Node.js, Bun, pnpm, and Yarn ignore unknown keys, so there is no
conflict. CLI flags and `package.json` grants are merged (CLI only adds, never
revokes a `package.json` grant).

```json
{
  "name": "my-app",
  "dependencies": {
    "express": "^4.18.0",
    "axios": "^1.6.0"
  },
  "3va": {
    "no-prompt": true,
    "permissions": {
      ".": {
        "allow-net": ["api.example.com"],
        "allow-read": ["./config", "${NODE_MODULES_ROOT}/express@4.22.2"],
        "deny-read": ["${NODE_MODULES_ROOT}/express@4.22.2/node_modules/express/lib/express.js"]
      },
      "axios": {
        "allow-net": ["api.example.com"]
      },
      "express": {
        "allow-net": ["*"]
      }
    }
  }
}
```

- Scope keys (`.`, `axios`, `express`) are for human readability only — every
  scope's grants are merged into one flat set for the whole process (the
  capability engine has no notion of "which module is calling").
- `deny-*` fields win over any broader `allow-*`, letting you grant a vendored
  directory by prefix while excluding one file with a known CVE.
- Relative paths resolve against the directory containing `package.json`, not
  the invocation `cwd` — the same file works from any working directory.
- `${VAR}` in path fields expands against the environment of the host process
  running `3va run` (evaluated before any capability exists), so one absolute
  root that differs per server/team (`/var/node_module`, `/local/bin/node_modules`, …)
  can be declared once and switched per environment without editing
  `package.json`. An undefined variable is left as a literal placeholder
  (fails closed) rather than collapsing to an empty string.
- `"no-prompt": true` is equivalent to passing `--no-prompt` on every
  invocation: any capability not covered by `allow-*` is denied silently
  instead of prompting.

`3va permissions suggest` (available today) will be extended to generate this
section directly, and `3va permissions learn` to persist the observed
permission set into it. Full reference:
[`docs/06-permissions/06-package-json-permissions.md`](docs/06-permissions/06-package-json-permissions.md).

---

## Package Manager

```bash
3va install axios --allow-net=registry.npmjs.org      # from npm
3va install react@18 --allow-net=registry.npmjs.org   # specific version
3va install @std/path --allow-net=jsr.io              # from JSR
3va reinstall axios --allow-net=registry.npmjs.org    # force-reinstall one package
3va update --allow-net=registry.npmjs.org             # update to latest versions
```

**Post-install scripts are never executed.** There are no exceptions. This is enforced at the package manager level, not as a flag.

Package storage uses global deduplication (same model as pnpm): a package at version X is stored once on disk; multiple projects reference it and declare their own permissions independently.

### Install benchmarks (warm cache, 10 runs each)

| Tool | Mean | Range |
|------|------|-------|
| bun install | 7.8 ms | 4.9–23.0 ms |
| **3va install** | **16.8 ms** | 12.5–31.1 ms |
| npm install | 984 ms | 911–1,307 ms |
| pnpm install | 1,368 ms | 1,204–1,603 ms |

### Audit

```bash
3va audit                  # malware scan + OSV CVE scan (24h cache)
3va audit --secrets        # also scan for leaked credentials
3va audit --deny           # exit non-zero on CRITICAL/HIGH findings
3va audit --json           # machine-readable output
```

Audit runs in three phases:

1. **Malware scan** — static analysis of `node_modules` for known malicious patterns
2. **OSV CVE scan** — queries `api.osv.dev` for known vulnerabilities; results cached 24 hours
3. **Secrets detection** (opt-in via `--secrets`) — 21 patterns covering AWS keys, GitHub tokens, Stripe keys, private certificates, JWT secrets, database connection strings, and more, scanned over the current project's source files

---

## Process Manager

Built into the runtime. No pm2, no separate daemon process.

```bash
3va start server.js --name api
3va start server.js --name worker -- --port 4000   # pass args after --
3va status                                          # all processes
3va status api                                      # one process
3va logs api --lines 200
3va restart api
3va stop api                                        # SIGTERM → SIGKILL after 1.5s
3va delete api                                      # stop + remove logs
```

Process metadata and logs live in `~/.3va/processes/`. If a managed process dies unexpectedly, `3va status` reports it as `error`; restart it with `3va restart <name>`. (Automatic restart on crash is on the roadmap.)

---

## HTTP Performance and Load Behavior

Baseline benchmarks, 100,000 requests at 1,000 concurrent connections:

| Runtime | Req/s | P50 | P99 | Success |
|---------|-------|-----|-----|---------|
| Bun 1.3 | 20,758 | 4.4 ms | 16.0 ms | 100% |
| **3va 2.0** (debug) | **1,572** | **61 ms** | **143 ms** | **100%** |

At 2,000 concurrent connections (stress test, 1,000,000 requests):

| Runtime | Success rate | Req/s | Notes |
|---------|-------------|-------|-------|
| Bun 1.3 | 100% | 21,650 | No connection limiting |
| Node.js 25 | 99.97% | 8,869 | 281 connection errors |
| **3va 2.0** (debug) | **70.4%** | **327** | Rate-limited by design |

3va deliberately limits active connections to protect the process from overload. At 2,000 concurrent connections the rate limiter drops excess connections rather than queuing them indefinitely. This is the intended behavior for production deployments. Slowloris protection is also built into the HTTP layer.

v2 roadmap targets RUDY (R-U-Dead-Yet) detection and adaptive rate limiting.

---

## Dev Tooling

### `package.json` scripts fallback

`3va <name>` runs `package.json.scripts.<name>` (via the project's actual package manager — `pnpm`/`yarn`/`bun`/`npm`, detected by lockfile) whenever `<name>` isn't one of 3va's own subcommands — the same convention `npm run <name>`/`pnpm <name>` follow:

```bash
3va build   # not a 3va subcommand → runs `pnpm run build` (or npm/yarn/bun)
```

Built-in subcommands always win: `3va dev`/`3va test`/etc. run 3va's own implementation, never a same-named script.

**This is not sandboxed** — the delegated script is a real external process (the actual package manager, running arbitrary shell), completely outside `vvva_permissions`' capability model (which only governs JS executed inside 3va's own QuickJS engine). Consistent with `3va install` never running postinstall scripts, running it requires explicit consent: a `[y/N]` prompt in a TTY, `--yes` to skip it, `"3va": { "no-prompt": true }` in `package.json` to skip it permanently, or a hard deny with no prompt at all outside a TTY (CI, pipes).

### Test runner

Jest-compatible. No configuration required.

```bash
3va test
3va test tests/unit
3va test --watch
3va test --coverage
3va test --update-snapshots
```

Supports `describe`, `test`, `expect`, all standard matchers, `toMatchSnapshot`, watch mode, and coverage reporting. The test runner is implemented in `vvva_test` — no Jest install needed.

### Bundler

```bash
3va bundle src/index.ts
3va bundle src/index.ts -o dist/bundle.js --minify
```

Walks the real import graph from the entry — project files, `node_modules` (both ESM and CommonJS packages), `.json`, and `.css` — and inlines everything into one self-contained file, runnable standalone via `3va run dist/bundle.js` or in a browser `<script>` tag. `.css` imports inject a `<style>` tag when a DOM is present (browser) and no-op otherwise (CLI/server); asset imports (images, fonts) embed the original path as a string, not a copied/hashed file — there's no production asset pipeline yet. The output directory is created automatically if it doesn't exist.

`--minify` works; `--source-map` and `--split` are not yet implemented for this real multi-file bundling path (only for the legacy single-file path reachable via the library API, not the CLI) — the output runs correctly without them, just without a source map or separate chunks. Tree shaking also isn't applied to the multi-file graph yet. For automatic rebuilds on change, use `3va dev` (the dev server watches and rebuilds with a 300 ms debounce) — it also serves the project directly via on-demand transpilation without needing a full bundle at all.

### Dev server with HMR

```bash
3va dev
3va dev --port 3000 --host 0.0.0.0 --open
```

Automatically detects the project framework (Next.js, Astro, Nuxt, SvelteKit, Remix, Gatsby, SolidStart, Qwik) and delegates to its native dev server. For custom/unrecognized setups, runs a built-in dev server:

- **On-demand ESM serving** (Vite-style): a root-level `index.html` referencing `<script type="module" src="/src/main.jsx">` works directly — `.js`/`.jsx`/`.ts`/`.tsx` files are transpiled per request (JSX/TS stripped via oxc) and their import specifiers rewritten so the browser's native ES module loader can resolve them: project-relative imports stay project-relative, bare specifiers (`"react"`) and anything under `node_modules` resolve to `/@fs/<path>`. `import "./x.css"` is wrapped in a tiny style-injecting module; `.css` requested directly (a `<link>` tag) is served as-is.
- A full bundle to `dist/bundle.js` still runs on start and on every source change (300ms debounce) — served at `/bundle.js`, useful for a hand-rolled `public/index.html` that references it directly instead of the raw entry.
- SPA fallback checks `public/index.html`, then a root-level `index.html`, then a built-in default page — HMR is full-page reload via Server-Sent Events (`/__hmr`), not granular per-module hot replacement.

Two Ctrl+C within the 30s drain window force an immediate shutdown — useful since a browser tab's open `/__hmr` connection otherwise keeps the server "draining" for the full timeout.

### CPU profiler

```bash
3va run app.ts --prof                           # writes profile.cpuprofile
3va run app.ts --prof --flamegraph=flame.svg    # also emit SVG flamegraph
3va prof profile.cpuprofile --top 20            # post-hoc analysis
```

Output is V8-compatible `.cpuprofile` JSON, loadable in Chrome DevTools and [speedscope.app](https://speedscope.app). Flamegraphs use the Inferno format.

### Debugger

```bash
3va run app.ts --inspect         # CDP on 127.0.0.1:9229
3va run app.ts --inspect=0.0.0.0:9230
```

Opens a WebSocket CDP server. Connect via `chrome://inspect` or any DAP-compatible IDE. `debugger;` statements pause execution and emit `Debugger.paused` CDP events.

### Interactive sandbox

```bash
3va sandbox
```

REPL with permission management. Inside the session: `.allow-read=PATH`, `.allow-write=PATH`, `.allow-net=HOST`, `.allow-env`, and `.permissions` to list current grants. Leave with `exit`, `quit`, or `^D`.

### Other commands

```bash
3va doctor          # environment health check
3va --accessible    # EN 301 549 mode: no ANSI, no animations, screen-reader friendly
```

---

## Environment Variables

3va respects the following environment variables. Priority: CLI flags > environment variables > config file > built-in defaults.

### `TOKIO_WORKER_THREADS`

Number of async I/O worker threads (default: one per logical CPU). 3va uses Tokio's multi-threaded runtime; this variable controls its worker pool size.

```bash
TOKIO_WORKER_THREADS=2 3va run app.ts
TOKIO_WORKER_THREADS=1 3va run server.js      # single-threaded mode
```

Documented in detail at [`docs/04-core/05-threading.md`](docs/04-core/05-threading.md).

### `3VA_<SECTION>_<KEY>` (config overrides)

Override any field from `3va.config.ts` at invocation time. Format: `3VA_<SECTION>_<KEY>` (uppercase, underscores for camelCase).

| Variable | Overrides |
|----------|-----------|
| `3VA_DEV_PORT` | `config.dev.port` |
| `3VA_DEV_HOST` | `config.dev.host` |
| `3VA_DEV_PUBLIC_DIR` | `config.dev.public_dir` |
| `3VA_DEV_OPEN` | `config.dev.open` |
| `3VA_DEV_CSP` | `config.dev.csp.enabled` |
| `3VA_TEST_COVERAGE` | `config.test.coverage` |
| `3VA_TEST_WATCH` | `config.test.watch` |
| `3VA_TEST_UPDATE_SNAPSHOTS` | `config.test.update_snapshots` |
| `3VA_TEST_CONCURRENCY` | `config.test.concurrency` |
| `3VA_AUDIT_DENY` | `config.audit.deny` |
| `3VA_AUDIT_SECRETS` | `config.audit.secrets` |
| `3VA_AUDIT_UPDATE_CACHE` | `config.audit.update_cache` |
| `3VA_BUNDLE_OUT_DIR` | `config.bundle.out_dir` |
| `3VA_BUNDLE_MINIFY` | `config.bundle.minify` |
| `3VA_BUNDLE_SOURCE_MAP` | `config.bundle.source_map` |
| `3VA_BUNDLE_SPLIT` | `config.bundle.split` |
| `3VA_WORKSPACE_HOISTING` | `config.workspace.hoisting` |
| `3VA_WORKSPACE_PARALLELISM` | `config.workspace.parallelism` |

```bash
3VA_DEV_PORT=8080 3va dev
3VA_TEST_CONCURRENCY=8 3va test
3VA_BUNDLE_MINIFY=true 3va bundle src/index.ts
```

### `3VA_ALLOW_SCRIPTS`

:warning: **Security — handle with care.** By default, 3va **never** executes npm lifecycle scripts (preinstall, install, postinstall). This is a security guarantee, not a configurable policy. Set `3VA_ALLOW_SCRIPTS=1` to re-enable them:

```bash
3VA_ALLOW_SCRIPTS=1 3va install express --allow-net=registry.npmjs.org
```

Only set this for trusted packages and registries.

### `_3VA_STORE`

Relocate the global content-addressable package store (default: `~/.3va/store/`). Useful in containers, CI, or when `$HOME` is ephemeral:

```bash
export _3VA_STORE=/mnt/cache/3va-store
3va install axios --allow-net=registry.npmjs.org
```

### `NO_COLOR`

Disable ANSI color output (per [no-color.org](https://no-color.org)). Equivalent to `--accessible` for output formatting; screen-reader and Braille display safe.

```bash
NO_COLOR=1 3va run app.ts
```

---

## Post-Quantum Cryptography

The `vvva_crypto` crate implements ML-KEM-768 (key encapsulation) and ML-DSA-65 (signatures), both exposed to JS via `require('crypto').pq`:

```js
const { pq } = require('crypto');

// ML-KEM-768 key exchange
const { publicKey, secretKey } = pq.kem.generateKeyPair();
const { ciphertext, sharedSecret } = pq.kem.encapsulate(publicKey);
const recovered = pq.kem.decapsulate(ciphertext, secretKey);

// ML-DSA-65 signatures
const kp = pq.dsa.generateKeyPair();
const sig = pq.dsa.sign(kp.secretKey, message);
const valid = pq.dsa.verify(kp.publicKey, message, sig);
```

> **Compatibility alias:** `generateKeypair` (lowercase `p`) is accepted as an alias for `generateKeyPair` for v1 source compatibility. New code should use `generateKeyPair`. The `3va codemod` command migrates old call sites automatically.

**Hybrid TLS** — `__pqTlsConnect` establishes a classical TLS connection with an additional ML-KEM-768 key exchange, producing a 32-byte hybrid shared secret:

```js
// Requires --allow-net=<host>
const { connId, pqSharedSecret } = await __pqTlsConnect('example.com', 443);
// pqSharedSecret: hex-encoded 32-byte shared secret derived via ML-KEM-768
```

Full post-quantum TLS in production is the v3 roadmap target.

---

## Compatibility

### Package registries

| Registry | Access |
|----------|--------|
| npmjs.org | `--allow-net=registry.npmjs.org` |
| registry.yarnpkg.com | `--allow-net=registry.yarnpkg.com` |
| jsr.io | `--allow-net=jsr.io` |

### Module formats

- CommonJS (`require`) and ESM (`import`/`export`) — both supported
- TypeScript — transpiled before execution, no `tsconfig.json` needed
- NAPI v8 native addons — `.node` files via `require('./addon.node')` with `--allow-ffi`
- WebAssembly — `.wasm` and `.wat` files, WASI preview1 compatible

### Frameworks

Framework detection in `3va dev` supports: Next.js, Astro, Nuxt, SvelteKit, Remix, Gatsby, SolidStart, Qwik. For Express, Fastify, Koa and similar server frameworks — they run under `3va run` with appropriate permission flags.

**Node.js compatibility note:** v2.0.0 covers the core API surface (fs, net, http, https, crypto, path, os, events, stream, buffer, timers, url, util, child_process, worker_threads, cluster). `dgram`, full DNS record resolution (MX/TXT/SRV/NS/CNAME), and some advanced APIs are partially implemented — see [`docs/05-js-engine/05-node-compat.md`](docs/05-js-engine/05-node-compat.md) for details.

---

## Architecture

`3va` is a Cargo workspace. Each crate has a single responsibility.

| Crate | Responsibility |
|-------|----------------|
| `vvva_core` | Tokio async event loop and scheduler |
| `vvva_cli` | `clap`-based CLI, subcommand routing, `--accessible` mode |
| `vvva_permissions` | Capability-based deny-by-default permission engine |
| `vvva_js` | QuickJS via `rquickjs`; ESM loader, TypeScript transpiler, async/await, Promise microtask loop, CDP inspector, profiler |
| `vvva_pm` | Package manager, malware scanner, secrets scanner, OSV auditor, lockfile |
| `vvva_bundler` | Bundler with tree shaking, code splitting, watch mode |
| `vvva_test` | Test runner, matchers, snapshot engine, coverage |
| `vvva_crypto` | ML-KEM-768, ML-DSA-65, HKDF, AES-GCM, classical crypto wrappers |
| `vvva_wasm` | WebAssembly/WASI runtime via `wasmtime` |

The JavaScript engine is QuickJS embedded via `rquickjs`. QuickJS is a small, embeddable engine with full ES2023 support, which makes it straightforward to intercept syscalls at the Rust boundary. This is the primary mechanism for permission enforcement.

**Note on `unsafe`:** The NAPI layer (`crates/js/src/builtins/napi.rs`) uses `unsafe extern "C"` to implement the ~30 NAPI v8 ABI functions. This is unavoidable for binary addon compatibility. All other crates aim to be `unsafe`-free; the CI runs `cargo-geiger` as an informational check on every commit.

---

## Roadmap

| Version | Target | Focus |
|---------|--------|-------|
| **2.0.0** | 2026-06-01 ✅ | Full runtime + PM + toolchain + Inspector + NAPI + WASM + PQ-TLS |
| **2.1.0** | 2026-07 ✅ | package.json `"3va"` permissions key (`allow-*`/`deny-*`, `${VAR}` expansion, `--no-prompt`) — see [`docs/06-permissions/06-package-json-permissions.md`](docs/06-permissions/06-package-json-permissions.md) |
| **2.2.0** | 2027 | `permissions suggest`/`learn` generate/persist the `package.json` section directly; permission profiles for common frameworks; Node.js compat v2 (full dns record resolution, repl, wasi, trace_events, WHATWG Streams), REPL plugins, workspace v2, adaptive rate limiting |
| **3.0** | TBD | Post-quantum TLS in full production mode |

Full roadmap: [`docs/12-roadmap/`](docs/12-roadmap/)

---

## Contributing

```sh
git clone https://github.com/OdinoCano/3va.git
cd 3va
./scripts/dev-setup.sh   # installs pre-commit hooks (fmt + clippy) and pre-push (test)
```

Every PR must pass all CI gates before merge:

| Gate | Blocks merge |
|------|-------------|
| `cargo fmt --check` | Yes |
| `cargo clippy -D warnings` | Yes |
| `cargo test` (1008 tests) | Yes |
| `cargo deny check` (CVEs + licenses) | Yes |
| Gitleaks secret scanning | Yes |
| Semgrep SAST (ERROR severity) | Yes |
| Fuzz build + 30s smoke run | Yes |

There is no way to bypass CI. Branch protection on `main` requires all checks green and at least one maintainer approval. Maintainers cannot push directly to `main`.

**Security-sensitive changes** (permissions engine, JS builtins, WASM sandbox, CI config, `Cargo.toml`) require maintainer review regardless of author — enforced via `CODEOWNERS`.

To report a vulnerability: open a [GitHub Security Advisory](https://github.com/OdinoCano/3va/security/advisories/new). Do not open a public issue.

---

## License

[MIT](LICENSE)
