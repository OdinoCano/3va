# 3va

[![CI](https://github.com/OdinoCano/3va/actions/workflows/ci.yml/badge.svg)](https://github.com/OdinoCano/3va/actions/workflows/ci.yml)
[![Security](https://github.com/OdinoCano/3va/actions/workflows/security.yml/badge.svg)](https://github.com/OdinoCano/3va/actions/workflows/security.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust Edition 2021](https://img.shields.io/badge/Rust-2021-orange.svg)](https://doc.rust-lang.org/edition-guide/rust-2021/index.html)

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

## Quick Start

```bash
git clone https://github.com/OdinoCano/3va.git
cd 3va
cargo build --release
sudo cp target/release/3va /usr/local/bin/
```

```bash
# Run a script (no permissions granted by default)
3va run app.ts

# Grant specific capabilities
3va run app.ts --allow-net=api.example.com --allow-read=./data

# Install packages
3va install express --allow-net=registry.npmjs.org

# Audit the dependency tree
3va audit --secrets

# Start as a background daemon
3va start app.js --name my-api
```

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

Omitting a flag means the capability is hard-blocked — not prompted at runtime, not configurable inside the script.

Permission scopes can be widened to cover all values:

```bash
3va run app.ts --allow-read --allow-net  # unrestricted read + network
```

Interactive permission prompts are enabled by default in `run`. The runtime asks at the point of first access if a needed permission is not granted.

### Package-level permission declarations (v1.5 roadmap)

Currently, permissions are CLI flags. The planned v1.5 model moves them into `package.json` under a `"3va"` key — a pattern already established by `"jest"`, `"eslint"`, and `"prettier"`. Node.js, Bun, pnpm, and Yarn ignore unknown keys, so there is no conflict.

The target schema:

```json
{
  "name": "my-app",
  "dependencies": {
    "express": "^4.18.0",
    "axios": "^1.6.0"
  },
  "3va": {
    "permissions": {
      ".": {
        "allow-net": ["api.example.com"],
        "allow-read": ["./config"]
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

The `3va permissions suggest` command (v1.5) will statically analyze the project and generate this section. `3va permissions learn` will run the app under syscall interception and produce the minimum set of permissions actually needed.

---

## Package Manager

```bash
3va install axios                          # from npm
3va install react@18                       # specific version
3va install @std/path --allow-net=jsr.io   # from JSR
3va reinstall                              # reinstall from lockfile
3va update                                 # update to latest compatible versions
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
3. **Secrets detection** (opt-in via `--secrets`) — 20 patterns covering AWS keys, GitHub tokens, Stripe keys, private certificates, JWT secrets, database connection strings, and more

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

Auto-restart on crash is enabled by default. The process manager state is local to the machine and survives reboots.

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
3va bundle src/index.ts -o dist/bundle.js --minify --source-map
3va bundle src/index.ts --split   # code splitting
```

Tree shaking, code splitting, minification, source maps, and watch mode.

### Dev server with HMR

```bash
3va dev
3va dev --port 3000 --host 0.0.0.0 --open
```

Automatically detects the project framework (Next.js, Astro, Nuxt, SvelteKit, Remix, Gatsby, SolidStart, Qwik) and delegates to its native dev server. For custom servers, runs a built-in dev server with HMR via Server-Sent Events (`/__hmr`), 300ms debounce, and SPA fallback.

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

REPL with permission management. Inside the session: `.allow-read <path>`, `.allow-net <host>`, `.permissions` to list current grants.

### Other commands

```bash
3va doctor          # environment health check
3va --accessible    # EN 301 549 mode: no ANSI, no animations, screen-reader friendly
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
| **2.1.0** | 2026 Q3 | `permissions suggest` (static analysis), `permissions learn` (syscall interception), package.json `"3va"` key, permission profiles for common frameworks |
| **2.2.0** | 2027 | Node.js compat v2 (full dns record resolution, repl, wasi, trace_events, WHATWG Streams), REPL plugins, workspace v2, adaptive rate limiting |
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
| `cargo test` (872 tests) | Yes |
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
