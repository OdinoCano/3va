# 05 - TEST AND VERIFICATION SCRIPTS

## 5.1 Integration Script (`integration_tests.sh`)

This script validates the full functioning of 3va with all supported registries.

### Location
```
scripts/integration_tests.sh
```

### Usage
```bash
./scripts/integration_tests.sh
```

### Test Phases

| Phase | Description | Verifies |
|------|-------------|----------|
| FASE 1 | NPM Registry | lodash from registry.npmjs.org |
| FASE 2 | Yarn Registry | axios from registry.yarnpkg.com |
| FASE 3 | JSR Registry | @std/path from jsr.io |
| FASE 4 | Import Verification | Package coexistence |
| FASE 5 | Basic Execution | Pure JS/TypeScript |
| FASE 6 | Diagnostics | doctor, help, version |
| FASE 7 | Bundle | basic, minify, split, source-map |
| FASE 8 | Test Runner | runner, --watch, --coverage, snapshots |
| FASE 9 | Update/Reinstall | update, reinstall commands |
| FASE 10 | Sandbox Mode | secure-by-default, REPL commands |
| FASE 11 | Audit | malware scan, OSV CVE query, secrets, --json |
| FASE 12 | Dev Server | HTTP serving, HMR SSE endpoint, static files |

### Requirements
- Binary compiled at `target/debug/3va`
- Network access to:
  - registry.npmjs.org
  - registry.yarnpkg.com
  - jsr.io

### Expected Output
```
╔════════════════════════════════════════════════════════════════╗
║                         FINAL SUMMARY                        ║
╚════════════════════════════════════════════════════════════════╝

  Total Tests:   <N>
  Passed:        <N>
  Failed:        0
  Success Rate: 100.0%

✓ ALL INTEGRATION TESTS PASSED

Registries verified:
  - npm (registry.npmjs.org) ✓
  - yarn (registry.yarnpkg.com) ✓
  - jsr (jsr.io) ✓
```

---

## 5.2 Security Script (`security_verify.sh`)

Runs the full security verification pipeline.

### Location
```
scripts/security_verify.sh
```

### Usage
```bash
./scripts/security_verify.sh
```

### Verification Levels

| Level | Verification | What it does |
|-------|--------------|--------------|
| 1 | Cargo Hardening | `fmt`, `clippy` (general and security lints), tests, `cargo audit`, `cargo deny`, and a per-crate `unsafe` policy |
| 2 | Semgrep | Custom security rules |
| 3 | Fuzzing | Every target in `fuzz/fuzz_targets/` for 15 s each, built with `--target <host triple>`; a crash is a FAIL |
| 4 | Sanitizers | AddressSanitizer and LeakSanitizer (a WARN when the nightly toolchain or `rust-src` is missing; `rustc` has no UBSan) |
| 5 | Security Tests | path_traversal, sandbox_escape, capability bypass, enforcement boundary, permissions ↔ JS engine, CLI ↔ `PermissionState`, package manager |
| 6 | Supply Chain | `Cargo.lock` present and `cargo vet --locked` |
| 7 | CodeQL | Workflow configured; Dependabot can read `Cargo.lock` |
| 8 | Coverage | `cargo-llvm-cov` through `scripts/coverage.sh` (WARN under 60% of lines) |
| 9 | Documentation | `cargo doc` warning count |
| 10 | Mutation testing | `cargo-mutants` on `vvva_permissions` (WARN when more than 5 mutants survive) |

### Reading the result

A `WARN` means a check did **not** verify what it names or needs a human look; it never counts as a pass. The script exits `0` unless a check `FAIL`ed, and the final summary reprints every warning. Set `STRICT=1` to make any warning fail the run.

- **`unsafe` policy.** A crate with no `unsafe` code must say so with `#![forbid(unsafe_code)]`, otherwise it is a FAIL. Crates that use `unsafe` (`cli`, `config`, `js`, `pm`) are reported with a count, as a WARN, to prompt a `// SAFETY:` review. This replaces an older `cargo geiger` grep that passed vacuously.
- **Fuzzing.** A target that crashes is a FAIL; one that times out or cannot be built is a WARN carrying the last line of its error (it used to hide stderr and report every target as "timeout o error").
- **Do not trust a `PASS` you did not read.** `security-reports/` (git-ignored, written locally) once held a `runlog.txt` saying PASS next to a `trivy.txt` listing six CRITICAL findings.

### Tool Installation
```bash
# Level 1
cargo install cargo-audit
cargo install cargo-deny
cargo install cargo-geiger

# Level 2
pip install semgrep  # or: npm install -g semgrep

# Level 3
cargo install cargo-fuzz
```

### Tool Installation (automatic)
The script attempts to install missing tools automatically.

### Expected Output
```
══════════════════════════════════════════════════════════
                    RESUMEN DE SEGURIDAD                   
══════════════════════════════════════════════════════════

Failures:  0
Warnings:  X

Advertencias (un WARN significa que ese chequeo NO se verificó o necesita revisión):
  - ...

✓ Sin fallos (X advertencias; STRICT=1 las trata como fallo)
```

### Vulnerability scan (`security_vuln_scan.sh`)

A second script runs the external scanners and writes a consolidated report to `security-reports/` (git-ignored):

```bash
scripts/security_vuln_scan.sh [--no-update] [--out DIR] [--skip audit,deny,...]
```

It covers `cargo audit`, `cargo deny`, `cargo geiger`, `cargo vet`, `cargo supply-chain` (publishers, informational), `cargo outdated`, `cargo udeps`, Semgrep, gitleaks, trivy, osv-scanner and a custom pattern scan, plus `fmt`, `clippy` and the test suite. Without `--no-update` it also installs or upgrades those tools globally. A tool that did not run is reported as a `WARN` ("NO se ejecutó"), never as a pass. trivy runs with `--exit-code 1`, skips the downloaded `tests/test262` suite, and reads `trivy-secret.yaml`, which allows only the fake token that the tests of `crates/pm/src/secrets.rs` use as sample input.

### Scheduled and on-demand checks

Two GitHub workflows cover what is too slow for every pull request:

- `.github/workflows/deep-checks.yml`: the **test262 baseline gate** (weekly and on demand; `NON_MODULE_KNOWN_FAILURES` and `INTL402_KNOWN_FAILURES` in `crates/test/tests/test262.rs`; more failures is a regression, fewer means lower the constant), and **N-API under AddressSanitizer** (weekly, on demand and on pull requests that touch `napi.rs`) with a canary that must trip `heap-buffer-overflow`, otherwise the sanitizer is not instrumenting.
- `.github/workflows/fuzz-nightly.yml` (daily): every fuzz target, see `docs/10-security/04-fuzzing.md`.

---

## 5.3 Recommended CI/CD Pipeline

### GitHub Actions
```yaml
name: Security & Integration Tests

on: [push, pull_request]

jobs:
  security:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable

      - name: Run security verification
        run: ./scripts/security_verify.sh

  integration:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable

      - name: Build
        run: cargo build --package vvva_cli

      - name: Run integration tests
        run: ./scripts/integration_tests.sh
```

---

## 5.4 Specific Security Tests

The project includes security tests in `crates/permissions/tests/security/`
(declared as the `security` test target in `crates/permissions/Cargo.toml`):

```bash
# Path traversal
cargo test -p vvva_permissions --test security path_traversal

# Sandbox escape
cargo test -p vvva_permissions --test security sandbox_escape

# Capability bypass
cargo test -p vvva_permissions --test security capability_bypass

# Resource-access enforcement boundaries
cargo test -p vvva_permissions --test security dos_prevention
```

> Note: `dos_prevention.rs` covers enforcement boundaries (fs/env/audit);
> memory/CPU limits belong to the JS runtime (`crates/js`) and are tested there.

---

*Scripts conforming to IEEE 829 test documentation standard.*
