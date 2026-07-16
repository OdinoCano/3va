# 01 - DEVELOPMENT ROADMAP

## 1.1 Vision

3va aims to be the most secure JavaScript/TypeScript runtime, surpassing Bun in cybersecurity features and permission model.

---

## 1.2 Current Status (v2.4.0 · 2026-07-16)

### Implemented and functional

| Module | Status | Notes |
|--------|--------|-------|
| CLI with granular permissions | ✅ | `run`, `install`, `reinstall`, `update`, `bundle`, `test`, `audit`, `doctor`, `sandbox`, `dev`, `start`, `stop`, `restart`, `status`, `logs`, `delete` — complete |
| Accessible mode (`--accessible`) | ✅ | EN 301 549 compliant |
| JS Engine (V8) | ✅ | Automatic TS transpilation |
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
| Secrets scanner | ✅ | `SecretsScanner`; 21 patterns (AWS, GitHub, GitLab, Stripe, Slack, SendGrid, Twilio, private keys, JWT, npm tokens, passwords, API keys, DB connection strings) |
| OSV audit | ✅ | 3 phases (malware + CVE + secrets); 24 h cache; `--deny`/`--json`/`--secrets`/`--update-cache` flags |
| Bundler | ✅ | Tree shaking, code splitting (`--split`), minification (`--minify`), source maps (`--source-map`), watch mode with real notifier |
| Test runner | ✅ | `describe`/`test`/`expect`; complete matchers; snapshots (`toMatchSnapshot` + `--update-snapshots`); `--watch`; `--coverage`; snapshot file I/O |
| Sandbox REPL | ✅ | Multi-line; `.help`/`.clear`/`.allow-read=`/`.allow-write=`/`.allow-net=`/`.allow-env`/`.permissions`; `exit`/`quit` to leave; TTY detection |
| Development server (`dev`) | ✅ | `--port`/`--host`/`--open`/`--public-dir`; HMR via SSE (`/__hmr`); HMR client injection; static files; SPA fallback; rebuild with 300 ms debounce |
| CDP Inspector (`--inspect`) | ✅ | WebSocket CDP server; `debugger;` rewrite; pause via `block_in_place` + `Condvar`; Chrome DevTools / DAP compatible |
| NAPI module loading (`--allow-ffi`) | ✅ | ~30 NAPI v8 functions; `.node` addons via `require()`; `napi_register_module_v1` ABI |
| WebAssembly (WASM) | ✅ | WASI-compatible; `.wasm` and `.wat` files; full permission integration |
| Post-quantum cryptography | ✅ | ML-KEM-768 + ML-DSA-65 via `vvva_crypto`; exposed under `require('crypto').pq` |
| Post-quantum TLS (`__pqTlsConnect`) | ✅ | Hybrid classical TLS + ML-KEM-768; async (non-blocking); `{ connId, pqSharedSecret }` |
| Audit logger | ✅ | Sensitive operation logging |
| CPU profiler (`--prof`) | ✅ | Sampling via `setInterval`+`Error.stack`; `.cpuprofile` JSON; SVG flamegraph; `3va prof` CLI |
| Fuzz targets in CI | ✅ | 6 targets (`fuzz/fuzz_targets/`: bundler codegen, permission sandbox, pm resolver, js import-meta, js esm-resolver, js transpiler). Every PR (`ci.yml`) smoke-runs 1 target for 30s; the full set of 6 runs for 30s each in the **weekly** (Monday cron) scheduled job in `security.yml` — not nightly. |
| Doc-tests | ✅ | Runnable doctests currently in `vvva_core`, `vvva_permissions` (×2), `vvva_crypto` (×2), `vvva_config`, `vvva_firewall`; `vvva_js` has doc comments but none with a runnable code block, so it contributes 0 |
| Test suite | ✅ | 1256 tests (unit + integration + doc) as of 2026-07-13, 0 failures — verify current count with `cargo test --workspace` before citing, this number drifts every PR |

---

## 1.3 Development Phases

### Phase 1: Foundation (Q2 2026) — ✅ COMPLETED

| Item | Status |
|----------|--------|
| Full CLI with permissions | ✅ |
| Core runtime (Tokio event loop) | ✅ |
| V8 JS engine integrated | ✅ |
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
| Secrets scanner (21 patterns) | ✅ |
| OSV audit 3 phases + 24 h cache | ✅ |
| Audit logger | ✅ |
| Post-install scripts disabled | ✅ |

### Phase 3: Tools (Q2 2026) — ✅ COMPLETED AHEAD OF SCHEDULE

| Item | Status |
|----------|--------|
| Bundler (tree shaking, code splitting, minification, source maps) | ✅ |
| Watch mode in bundler (real notifier) | ✅ |
| Test runner (matchers, snapshots, coverage, watch) | ✅ |
| Sandbox REPL with TTY detection | ✅ |
| Development server with HMR | ✅ |

### Phase 4: LTS (Q2 2026) — ✅ COMPLETED AHEAD OF SCHEDULE

| Item | Status | Notes |
|----------|--------|-------|
| Inspector / debugger / breakpoints | ✅ | CDP WebSocket server; `debugger;` rewrite; Chrome DevTools / DAP |
| WebAssembly (WASM) module loading | ✅ | WASI-compatible; `.wasm` + `.wat`; permission integration |
| Native module support (NAPI) | ✅ | ~30 NAPI v8 functions; `.node` addons via `require()` |
| Post-quantum cryptography integrated in TLS | ✅ | Hybrid TLS + ML-KEM-768; `__pqTlsConnect` global; async |
| Public API stabilization | ✅ | Doc-tests on all public crate surfaces |
| Release 1.0 LTS | ✅ | **Released 2026-06-01** |
| Performance profiling / flamegraph | ✅ | `--prof` + `3va prof`; `.cpuprofile` JSON + SVG via inferno |

---

## 1.4 Milestones

| Version | Release date | Features | Status |
|---------|----------------|----------|--------|
| 0.1.0-dev | May 2026 | CLI + Core + JS (ESM/CJS/async) + PM + Bundler + Test runner + Dev server + Security (malware + secrets + OSV) | ✅ |
| 1.0.0 LTS | **2026-06-01** | Inspector/CDP + WASM + NAPI + PQ-TLS + stable API | ✅ **Released** |
| 2.0.0 | **2026-06-10** | Performance profiling + Node.js compat improvements + workspace v2 + REPL plugins | ✅ **Released** |
| 2.1.0 | **2026-06-22** | timers/promises + stream/web + dns + readline (full Node.js compat) + `3va create` + heap snapshot + SLSA Level 2 + automated security audit | ✅ **Released** |
| 2.4.0 | **2026-07-16** | PM feature-parity roadmap Phases A+B (overrides, `licenses`, `sbom`, peer autoinstall, hoisted linker, config deps, zero-installs, `patch`/`patch-commit`, `dlx`, auto-install-before-run) + self-hash in `3va -V` + dynamic `import()` fix + HTTP server memory-under-load fix + reproducible benchmark suite | ✅ **Released** |

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
| Post-quantum cryptography (ML-KEM-768 + PQ-TLS) | No | No | No | **Yes** |
| CDP Inspector / debugger | No | Yes | No | **Yes** |
| NAPI native module loading | Yes | Yes | Yes | **Yes** |
| WebAssembly (WASI) | No | Yes | Yes | **Yes** |
| WHATWG Streams (stream/web) | Yes | Yes | Yes | **Yes** |
| timers/promises (Node.js 18+) | Yes | Yes | Yes | **Yes** |
| SLSA Level 2 provenance | No | No | No | **Yes** |
| Automated security audit CI | No | No | No | **Yes** |
| Framework scaffolding (`3va create`) | No | No | No | **Yes** |
| Heap snapshots for memory profiling | Yes | Yes | No | **Yes** |

---

*Roadmap subject to change based on feedback and project priorities.*
