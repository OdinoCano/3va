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
| PHASE 1 | NPM Registry | lodash from registry.npmjs.org |
| PHASE 2 | Yarn Registry | axios from registry.yarnpkg.com |
| PHASE 3 | JSR Registry | @std/path from jsr.io |
| PHASE 4 | Import Verification | Package coexistence |
| PHASE 5 | Basic Execution | Pure JS/TypeScript |
| PHASE 6 | Diagnostics | doctor, help, version |
| PHASE 7 | Bundle | basic, minify, split |
| PHASE 8 | Test Runner | runner and --watch |
| PHASE 9 | Update/Reinstall | available commands |
| PHASE 10 | Sandbox Security | secure-by-default |

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

  Total Tests:   25
  Passed:        25
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

| Level | Verification | Required Status |
|-------|--------------|------------------|
| 1 | Cargo Hardening | fmt, clippy, test, audit, deny, geiger |
| 2 | Semgrep | Custom security rules |
| 3 | Fuzzing | Parser and package manager |
| 4 | Sanitizers | ASAN, UBSAN |
| 5 | Security Tests | path_traversal, sandbox_escape, etc. |
| 6 | Supply Chain | Cargo.lock, cargo vet |
| 7 | CodeQL | GitHub Advanced Security |

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
                    SECURITY SUMMARY                   
══════════════════════════════════════════════════════════

Failures:  0
Warnings:  X

✓ Security pipeline PASSED
```

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

The project includes security tests in `tests/security/`:

```bash
# Path traversal
cargo test --test security path_traversal

# Sandbox escape
cargo test --test security sandbox_escape

# Capability bypass
cargo test --test security capability_bypass

# DOS prevention
cargo test --test security dos
```

---

*Scripts conforming to IEEE 829 test documentation standard.*
