# 3VA v2.0.0 — Technical Specification Overview

**Version:** 2.0.0 (planned)
**Base:** v1.0.0 LTS (2026-06-01)
**Status:** Draft

---

## Index

| Document | Content | Status |
|----------|---------|--------|
| [01-profiler.md](01-profiler.md) | CPU profiler, flamegraph, `3va run --prof` CLI | ✅ Implemented in v1.x |
| —  | CDP Inspector (`--inspect`), Chrome DevTools Protocol WebSocket server | ✅ Implemented in v1.x |
| — | NAPI v8 module loading (`require('./addon.node')`), `--allow-ffi` | ✅ Implemented in v1.x |
| [02-node-compat-v2.md](02-node-compat-v2.md) | New Node.js modules: `worker_threads`, `dgram`, `dns`, `stream/web` (require alias), `timers/promises`, `readline` (full) | 🔲 Planned v2.0.0 |
| [03-repl-plugins.md](03-repl-plugins.md) | REPL plugin API and built-in plugins | 🔲 Planned v2.0.0 |
| [04-workspace-v2.md](04-workspace-v2.md) | Topological execution, affected mode, graph, hoisting | 🔲 Planned v2.0.0 |
| [05-config-file.md](05-config-file.md) | `3va.config.ts` schema and loading order | 🔲 Planned v2.0.0 |
| [06-security-v2.md](06-security-v2.md) | RUSTSEC fix, SLSA level 2, CSP, automated audit, PQ SubtleCrypto | 🔲 Planned v2.0.0 |
| [07-migration.md](07-migration.md) | Migration tool CLI, codemods, and breaking changes transition | 🔲 Planned v2.0.0 |
| [08-testing-v2.md](08-testing-v2.md) | Parallel test execution, mocking APIs, and CI reporting | 🔲 Planned v2.0.0 |

---

## Compatibility Promise

v2.0.0 is a **compatible** major release with respect to v1.0.0. All scripts and packages that work on v1.0.0 will continue to work on v2.0.0 without changes, with the sole exception of the `require('crypto').pq` API alignment documented in [06-security-v2.md](06-security-v2.md). An automated codemod is provided for that migration.

---

*Document identifier: 3VA-SPEC-2027-002 · Classification: Public*
