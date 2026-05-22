# 01 - DEVELOPMENT ROADMAP

## 1.1 Vision

3va aims to be the most secure JavaScript/TypeScript runtime, surpassing Bun in cybersecurity features and permission model.

---

## 1.2 Current Status (v0.1.0-dev · 2026-05-21)

### Implemented and functional

| Module | Status | Notes |
|--------|--------|-------|
| CLI with granular permissions | ✅ | `run`, `install`, `reinstall`, `update`, `bundle`, `test`, `audit`, `doctor`, `sandbox`, `dev` — complete |
| Accessible mode (`--accessible`) | ✅ | EN 301 549 compliant |
| JS Engine (QuickJS) | ✅ | Automatic TS transpilation |
| CommonJS + ESM Modules | ✅ | `EsmResolver` + `EsmLoader`; static and dynamic import/export |
| async/await and Promise chains | ✅ | Complete microtask loop |
| Permission system (deny-by-default) | ✅ | `FileRead`, `FileWrite`, `Network`, `EnvAccess`, `SpawnProcess`, `FFI` |
| Interactive permission prompt | ✅ | `PermissionState`; enabled by default in `run` |
| Package Manager — `install` | ✅ | npm, Yarn, JSR; specific version; close suggestions |
| Package Manager — `reinstall` | ✅ | Forced |
| Package Manager — `update` | ✅ | Registry-aware; multi-registry; `--allow-net` validation |
| Lockfile with `registry` field | ✅ | Source traceability per package; semver resolution |
| Signature verification (SHA-256/SHA-512) | ✅ | `SignatureVerifier` |
| Malware scanner | ✅ | Static analysis of `node_modules` |
| Secrets scanner | ✅ | `SecretsScanner`; 16 patterns (AWS, GitHub, GitLab, Stripe, Slack, SendGrid, Twilio, private keys, JWT, npm tokens, passwords, API keys, DB connection strings) |
| OSV audit | ✅ | 3 phases (malware + CVE + secrets); 24 h cache; `--deny`/`--json`/`--secrets`/`--update-cache` flags |
| Bundler | ✅ | Tree shaking, code splitting (`--split`), minification (`--minify`), source maps (`--source-map`), watch mode with real notifier |
| Test runner | ✅ | `describe`/`test`/`expect`; complete matchers; snapshots (`toMatchSnapshot` + `--update-snapshots`); `--watch`; `--coverage`; snapshot file I/O |
| Sandbox REPL | ✅ | Multi-line; `.help`/`.exit`/`.clear`/`.allow-read`/`.allow-net`/`.permissions`; TTY detection |
| Development server (`dev`) | ✅ | `--port`/`--host`/`--open`/`--public-dir`; HMR via SSE (`/__hmr`); HMR client injection; static files; SPA fallback; rebuild with 300 ms debounce |
| Audit logger | ✅ | Sensitive operation logging |
| Crate `vvva_crypto` | ✅ | Post-quantum readiness utilities (standalone crate) |
| Test suite | ✅ | 58 integration tests (12 phases, 100 % passing); 287 unit tests |

---

## 1.3 Development Phases

### Phase 1: Foundation (Q2 2026) — ✅ COMPLETED

| Item | Status |
|----------|--------|
| Full CLI with permissions | ✅ |
| Core runtime (Tokio event loop) | ✅ |
| QuickJS JS engine integrated | ✅ |
| TypeScript transpilation | ✅ |
| CommonJS + ESM Modules | ✅ |
| async/await and Promise chains | ✅ |
| EN 301 549 accessible mode | ✅ |

### Phase 2: Package Manager (Q3 2026) — ✅ COMPLETED AHEAD OF SCHEDULE

| Item | Status |
|----------|--------|
| Basic functional PM (install/reinstall/update) | ✅ |
| Multi-registry (npm, Yarn, JSR) | ✅ |
| Lockfile with `registry` field and semver resolution | ✅ |
| Signature verification (SHA-256/SHA-512) | ✅ |
| Malware scanner (static analysis) | ✅ |
| Secrets scanner (16 patterns) | ✅ |
| OSV audit 3 phases + 24 h cache | ✅ |
| Audit logger | ✅ |
| Post-install scripts disabled | ✅ |

### Phase 3: Tools (Q4 2026) — ✅ COMPLETED AHEAD OF SCHEDULE

| Item | Status |
|----------|--------|
| Bundler (tree shaking, code splitting, minification, source maps) | ✅ Completed |
| Watch mode in bundler (real notifier) | ✅ Completed |
| Test runner (matchers, snapshots, coverage, watch) | ✅ Completed |
| Sandbox REPL with TTY detection | ✅ Completed |
| Development server with HMR | ✅ Completed |
| Inspector / debugger / breakpoints | 🔲 Pending |

### Phase 4: LTS (2027)

| Item | Status |
|----------|--------|
| Inspector / debugger / breakpoints | 🔲 |
| WebAssembly (WASM) module loading | 🔲 |
| Performance profiling / flamegraph | 🔲 |
| Native module support (NAPI) | 🔲 |
| Post-quantum cryptography integrated in TLS | 🔲 |
| Public API stabilization | 🔲 |
| Release 1.0 LTS | 🔲 |

---

## 1.4 Milestones

| Version | Target date | Features | Status |
|---------|----------------|----------|--------|
| 0.1.0 | Jun 2026 | CLI + Core + JS (ESM/CJS/async) + PM + Bundler + Test runner + Dev server + Security (malware + secrets + OSV) | ✅ Feature-complete; stabilizing |
| 0.2.0 | Sep 2026 | Inspector/debugger + WASM + performance profiling | 🔲 |
| 0.3.0 | Nov 2026 | NAPI support + post-quantum TLS + public benchmarks | 🔲 |
| 1.0.0 | Mar 2027 | LTS + stable API + post-quantum crypto fully integrated | 🔲 |

---

## 1.5 Advantages vs Competition

| Feature | Node.js | Deno | Bun | **3va** |
|---------|---------|------|-----|---------|
| Granular permissions | No | Yes | No | **Yes** |
| Network denied by default in PM | No | Yes | No | **Yes** |
| Multi-registry with source traceability | No | No | No | **Yes** |
| Post-install scripts disabled | No | No | No | **Yes** |
| Integrated malware analysis | No | No | No | **Yes** |
| Mandatory signature verification | No | No | No | **Yes** |
| Integrated secrets detection | No | No | No | **Yes** |
| OSV audit 3 phases with cache | No | Partial | No | **Yes** |
| Development server with HMR | No | Yes | Yes | **Yes** |
| Accessible mode (EN 301 549) | No | No | No | **Yes** |
| Post-quantum cryptography (crate ready) | No | No | No | **Yes** |

---

*Roadmap subject to change based on feedback and project priorities.*
