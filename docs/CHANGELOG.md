# Changelog

All notable changes to **3va** are documented here.
Format: [Keep a Changelog 1.0.0](https://keepachangelog.com/en/1.0.0/) · Versioning: [SemVer](https://semver.org/).

---

## [Unreleased]

### Added

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
