# 01 - DEVELOPMENT ROADMAP

## 1.1 Vision

3va aims to be the most secure JavaScript/TypeScript runtime, surpassing Bun in cybersecurity features and permission model.

---

## 1.2 Current Status (v2.6.0 · 2026-08-18)

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
| Bundler | 🟡 | Real multi-file graph bundling + `--minify` + watch mode with real notifier are done. `--source-map`, `--split`, and tree shaking exist only on a legacy single-file path reachable via the library API, not `3va bundle` — see [README § Known Limitations](../../README.md#known-limitations--roadmap) |
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

## 1.6 Security Hardening Backlog (found by internal audit, 2026-08-19)

Gaps identified against 3va's own attack surface (its HTTP/WS/MQTT/IMAP builtins, PM, and crypto), verified against code — status tracked per row below. See `README.md` § Known Limitations & Roadmap for the user-facing summary, and `docs/SECURITY.md` § Level 6 for the compression-bomb item.

| Item | Area | Status |
|------|------|--------|
| CRLF sanitization in `res.setHeader()`/`res.writeHead()` | HTTP server | ✅ Implemented (`ERR_INVALID_HTTP_TOKEN`/`ERR_INVALID_CHAR`, enforced in JS layer and native writer; tests in `crates/js/tests/http_server.rs`) |
| `Transfer-Encoding: chunked` support in request parser | HTTP server | ✅ Implemented (chunked decoding, CL+TE smuggling rejected `400`; tests in `crates/js/tests/http_server.rs`) |
| Decompression ratio/size cap (`zlib` builtin + PM tarball extraction) | zlib, PM | ✅ Implemented (`MAX_DECOMPRESSED_OUTPUT_BYTES`/`MAX_DECOMPRESSION_RATIO` in `zlib.rs`; `MAX_EXTRACTED_FILE_BYTES`/`MAX_EXTRACTED_TOTAL_BYTES` in `lib.rs`/`fetcher.rs`) |
| Response size cap on `fetch()` | fetch | ✅ Implemented (`MAX_RESPONSE_BODY_BYTES` = 512 MiB streaming cap + early `Content-Length` reject; per-call `{ maxResponseSize }` option; tests in `crates/js/tests/fetch_response_cap.rs`) |
| Connect/read timeouts on MQTT and IMAP client sockets | MQTT, IMAP | ✅ Implemented (`MQTT_CONNECT_TIMEOUT`/`MQTT_IO_TIMEOUT`, `IMAP_CONNECT_TIMEOUT`/`IMAP_IO_TIMEOUT`; per-client `connectTimeout`; tests `connect_tcp_bounded_times_out_against_blackholed_host`, `establish_connection_read_times_out_against_silent_server`, `mqtt_connect_times_out_against_blackholed_host`) |
| Automatic malware/secrets scan during `3va install` | PM | ✅ Implemented (new downloads scanned after integrity check, before store/link; CRITICAL/HIGH aborts; `--no-scan` opt-out; tests `security_scan_passes_a_clean_package`, `security_scan_aborts_package_with_embedded_aws_key`, `security_scan_respects_skip_flag` in `crates/pm/src/lib.rs`; docs `docs/10-security/01-static-analysis.md`) |
| Typosquatting detection (edit-distance vs. popular package names) | PM | ✅ Implemented (`typosquat.rs` + embedded `popular_packages.txt`, `TYPOSQUAT_MAX_DISTANCE` = 2; warns during install resolution and `warn_for_manifest_deps` for audit flows; tests `classic_typos_are_flagged`, `exact_popular_names_never_warn`, `unrelated_names_do_not_warn`, `warn_for_manifest_deps_reports_count`) |
| Dependency-confusion protection (scoped vs. unscoped resolution preference) | PM | ✅ Implemented (`pinned_scope_registry` in `npmrc.rs`; scopes pinned via `@scope:registry=` resolve only against the private registry — public fallback refused with a fatal error, plus a notice when `.npmrc registry=` differs from `--allow-net`; tests `scoped_pin_resolves_only_against_private_registry`, `pinned_scope_with_dead_private_registry_refuses_public_fallback` in `lib.rs`, `pinned_scope_registry_matches_only_configured_scope` in `npmrc.rs`) |
| npm provenance / Sigstore signature verification | PM | ✅ Implemented (`provenance.rs`: fetches `/-/npm/v1/attestations/{pkg}@{version}`, verifies DSSE PAE ECDSA signature against the bundle X.509 certificate's P-256/P-384 key, checks in-toto subject = `pkg:npm/{name}@{version}`; invalid attestation aborts install, missing attestation is soft unless `--require-provenance`; tests `real_npm_provenance_fixture_verifies`, `tampered_signature_fails_hard`, `install_aborts_on_invalid_provenance`. Not yet verified: Fulcio chain-to-root and Rekor inclusion proofs — presence-only check today) |
| Adaptive rate limiting (auto-tune from observed traffic) | Firewall | ✅ Implemented (`adaptive_rate_limit` + `ewma_alpha_pct` knobs; per-IP EWMA over 1 s windows raises the effective threshold to `max(static, ceil(ewma×1.5))`, capped at `static × 4`; formula in `docs/10-security/08-firewall.md` §Rate limiting adaptativo; tests `ewma_update_tracks_samples_with_configurable_smoothing`, `effective_rps_rises_with_baseline_and_stays_capped`, `growing_legitimate_traffic_raises_limit_without_violations`) |
| `X-Forwarded-For` support for `remoteAddress` behind a reverse proxy | Firewall | ✅ Implemented (`trustedProxies` config (IPs/CIDRs) + `resolve_forwarded_for` in `crates/firewall/src/lib.rs`; rightmost-untrusted walk, header ignored from untrusted peers; feeds both rate-limit accounting and `req.socket.remoteAddress`; tests `xff_resolves_client_behind_trusted_proxy`, `firewall_client_ip_uses_xff_only_through_trusted_proxies`, e2e `trusted_proxy_forwards_client_ip_to_remote_address`, `untrusted_xff_header_is_ignored`; docs §8.5/§8.9 of `docs/10-security/08-firewall.md`) |

---

*Roadmap subject to change based on feedback and project priorities.*
