# 3VA v2.0.0 — Technical Specification Overview

**Version:** 2.0.0
**Base:** v1.0.0 LTS (2026-06-01)
**Released:** 2026-06-04
**Status:** Released

---

## Index

| Document | Content | Status |
|----------|---------|--------|
| [01-profiler.md](01-profiler.md) | CPU profiler, flamegraph, `3va run --prof` CLI | ✅ Implemented in v1.x |
| —  | CDP Inspector (`--inspect`), Chrome DevTools Protocol WebSocket server | ✅ Implemented in v1.x |
| — | NAPI v8 module loading (`require('./addon.node')`), `--allow-ffi` | ✅ Implemented in v1.x |
| [02-node-compat-v2.md](02-node-compat-v2.md) | `worker_threads` (real OS threads), `dgram` (UDP), `timers/promises`, `readline`, `stream/web` | ✅ Implemented in v2.0.0 |
| [03-repl-plugins.md](03-repl-plugins.md) | REPL plugin API and built-in plugins (`inspect`, `history`) | ✅ Implemented in v2.0.0 |
| [04-workspace-v2.md](04-workspace-v2.md) | Topological execution, `--affected` mode, `workspace graph` | ✅ Implemented in v2.0.0 |
| [05-config-file.md](05-config-file.md) | `3va.config.ts` schema, loading, env overrides, `3va config` subcommand | ✅ Implemented in v2.0.0 |
| [06-security-v2.md](06-security-v2.md) | CSP for `3va dev`, PQ API alignment (`generateKeyPair`), `--no-csp` | ✅ Implemented in v2.0.0 |
| [07-migration.md](07-migration.md) | `3va codemod --from=1 --to=2`: `generateKeyPair`, `sign/verify` named params | ✅ Implemented in v2.0.0 |
| [08-testing-v2.md](08-testing-v2.md) | Parallel execution (`--concurrency`), mock API (`3va:test`), JUnit/TAP reporters | ✅ Implemented in v2.0.0 |

---

## Compatibility Promise

v2.0.0 is a **compatible** major release with respect to v1.0.0. All scripts and packages that work on v1.0.0 will continue to work on v2.0.0 without changes, with the sole exception of the `require('crypto').pq` API alignment documented in [06-security-v2.md](06-security-v2.md). An automated codemod is provided for that migration.

---

*Document identifier: 3VA-SPEC-2027-002 · Classification: Public*
