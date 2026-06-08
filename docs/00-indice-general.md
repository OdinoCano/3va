# GENERAL INDEX - 3va Technical Documentation

## Volume 1: Introduction and Project Vision

- [01-resumen-ejecutivo.md](01-intro/01-resumen-ejecutivo.md) - Executive summary of the project
- [02-alcance.md](01-intro/02-alcance.md) - Scope and objectives
- [03-definiciones.md](01-intro/03-definiciones.md) - Definitions and abbreviations

## Volume 2: System Architecture

- [01-arquitectura-general.md](02-arquitectura/01-arquitectura-general.md) - General system architecture
- [02-diseno-componentes.md](02-arquitectura/02-diseno-componentes.md) - Component design
- [03-interfaces.md](02-arquitectura/03-interfaces.md) - Interfaces and communication
- [04-execucion-del-script.md](02-arquitectura/04-execucion-del-script.md) - Script execution flow

## Volume 3: CLI Specification

- [01-interfaz-linea-comandos.md](03-cli/01-interfaz-linea-comandos.md) - Command line interface
- [02-comandos.md](03-cli/02-comandos.md) - Available commands
- [03-opciones.md](03-cli/03-opciones.md) - Options and flags
- [04-ejemplos.md](03-cli/04-ejemplos.md) - Usage examples

## Volume 4: Core Runtime

- [01-event-loop.md](04-core/01-event-loop.md) - Event loop and scheduler
- [02-modulo-system.md](04-core/02-modulo-system.md) - System modules
- [03-globals.md](04-core/03-globals.md) - Global objects
- [04-process.md](04-core/04-process.md) - Process management

## Volume 5: JavaScript Engine

- [01-quickjs-integration.md](05-js-engine/01-quickjs-integration.md) - QuickJS integration
- [02-modulo-loader.md](05-js-engine/02-modulo-loader.md) - Module loading
- [03-polyfills.md](05-js-engine/03-polyfills.md) - Polyfills and shims
- [04-web-apis.md](05-js-engine/04-web-apis.md) - Compatible web APIs
- [05-node-compat.md](05-js-engine/05-node-compat.md) - Node.js compatibility — EventEmitter, path, os, fs-fd, zlib streams, process

## Volume 6: Permissions System

- [01-capability-model.md](06-permissions/01-capability-model.md) - Capability model
- [02-enforcement.md](06-permissions/02-enforcement.md) - Policy enforcement
- [03-sandboxing.md](06-permissions/03-sandboxing.md) - Sandboxing and isolation
- [04-audit.md](06-permissions/04-audit.md) - Audit and logging
- [05-interactive-prompts.md](06-permissions/05-interactive-prompts.md) - Interactive permission prompts (TTY, session scope)

## Volume 7: Package Manager

- [01-especificacion-pm.md](07-pm/01-especificacion-pm.md) - PM specification
- [02-resolucion.md](07-pm/02-resolucion.md) - Dependency resolution
- [03-sandboxing.md](07-pm/03-sandboxing.md) - Package sandboxing
- [04-lockfile.md](07-pm/04-lockfile.md) - Lockfile format
- [05-global-dedup.md](07-pm/05-global-dedup.md) - Global content-addressable store and deduplication

## Volume 8: Bundler

- [01-especificacion-bundler.md](08-bundler/01-especificacion-bundler.md) - Bundler specification
- [02-arquitectura.md](08-bundler/02-arquitectura.md) - Internal architecture and output formats
- [03-transpilation.md](08-bundler/03-transpilation.md) - TS/JSX transpilation
- [04-tree-shaking.md](08-bundler/04-tree-shaking.md) - Tree shaking
- [05-code-splitting.md](08-bundler/05-code-splitting.md) - Code splitting

## Volume 9: Test Runner

- [01-especificacion-tests.md](09-testing/01-especificacion-tests.md) - Test specification
- [02-matchers.md](09-testing/02-matchers.md) - Matchers and assertions
- [03-coverage.md](09-testing/03-coverage.md) - Coverage report
- [03-snapshots.md](09-testing/03-snapshots.md) - Snapshots
- [04-watch-mode.md](09-testing/04-watch-mode.md) - Watch mode
- [05-scripts.md](09-testing/05-scripts.md) - Test and verification scripts

## Volume 10: Advanced Security Features

- [01-static-analysis.md](10-security/01-static-analysis.md) - Static analysis
- [02-malware-scanner.md](10-security/02-malware-scanner.md) - Malware scanner
- [03-secrets-detection.md](10-security/03-secrets-detection.md) - Secrets detection
- [04-fuzzing.md](10-security/04-fuzzing.md) - Integrated fuzzing
- [05-post-quantum.md](10-security/05-post-quantum.md) - Post-quantum cryptography
- [06-osv-audit.md](10-security/06-osv-audit.md) - Vulnerability audit (OSV)
- [07-http-performance.md](10-security/07-http-performance.md) - HTTP benchmarks, connection rate limiting, Slowloris protection
- [08-firewall.md](10-security/08-firewall.md) - Firewall interno HTTP: Slowloris, RUDY, rate limiting, blocklist, configuración

## Volume 11: APIs and Reference

- [01-js-api.md](11-api/01-js-api.md) - Public JavaScript API
- [02-interna-api.md](11-api/02-interna-api.md) - Internal runtime API
- [03-error-codes.md](11-api/03-error-codes.md) - Error codes

## Volume 12: Roadmap and LTS

- [01-roadmap.md](12-roadmap/01-roadmap.md) - Development roadmap (updated for v1.0.0)
- [02-lts-criteria.md](12-roadmap/02-lts-criteria.md) - LTS criteria
- [03-release-process.md](12-roadmap/03-release-process.md) - Release process
- [04-compatibility.md](12-roadmap/04-compatibility.md) - Backward compatibility
- [05-v2-roadmap.md](12-roadmap/05-v2-roadmap.md) - v2.0.0 roadmap and feature goals

## Volume 13: v2.0.0 Technical Specification

- [00-overview.md](13-v2/00-overview.md) - v2.0.0 overview and compatibility promise
- [01-profiler.md](13-v2/01-profiler.md) - CPU profiler, flamegraph, heap snapshot (`--prof`)
- [02-node-compat-v2.md](13-v2/02-node-compat-v2.md) - New Node.js modules: `worker_threads`, `dgram`, `dns`, `webcrypto`, `stream/web`, `timers/promises`
- [03-repl-plugins.md](13-v2/03-repl-plugins.md) - REPL plugin API and built-in plugins
- [04-workspace-v2.md](13-v2/04-workspace-v2.md) - Topological execution, affected mode, graph, hoisting
- [05-config-file.md](13-v2/05-config-file.md) - `3va.config.ts` schema and loading order
- [06-security-v2.md](13-v2/06-security-v2.md) - RUSTSEC-2023-0071 resolution, SLSA level 2, CSP, `pq` API alignment
- [07-migration.md](13-v2/07-migration.md) - Migration tool CLI, codemods, and breaking changes transition
- [08-testing-v2.md](13-v2/08-testing-v2.md) - Parallel test execution, mocking APIs, and CI reporting

## Changelog

- [CHANGELOG.md](CHANGELOG.md) — Version history (Keep a Changelog 1.0.0)
- [changelog-2026-06-01.md](changelog-2026-06-01.md) — Security/correctness fixes from code-review (10 findings)
- [changelog-2026-06-03.md](changelog-2026-06-03.md) — Expo/React Native package support + ESM→CJS engine fixes (45/45 tests)
- [changelog-2026-06-05.md](changelog-2026-06-05.md) — Firewall interno HTTP (`vvva_firewall`): Slowloris/RUDY/rate-limit/blocklist + 13 nuevos tests

---

**Document Identifier:** 3VA-SPEC-2026-001
**Version:** 2.0.0
**Date:** 2026-06-05
**Classification:** Public
**Status:** v1.0.0 Released · v2.0.0 Draft

---

*Document conforming to ISO/IEC/IEEE 29148 and European technical documentation standards.*
