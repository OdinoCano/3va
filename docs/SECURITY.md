# 3VA Security Documentation

## Philosophy

Security is prioritized over compatibility. We assume:
- V8 will have bugs
- crates will have CVEs
- sandbox escapes will exist
- parser bugs will be found
- DOS will be attempted
- bypasses will occur

The question is not "if" but "how well contained".

---

## Security Pipeline

### Level 1 - Cargo Hardening (Mandatory)

```bash
./scripts/security_verify.sh
```

Automatically executes:
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-features`
- `cargo audit --deny-warnings`
- `cargo deny check`
- `cargo geiger` (unsafe detection)

### Clippy — Security Lints

The Clippy step runs in two phases:

**Phase 1 — Warnings as errors (all targets):**
```bash
cargo clippy --all-targets --all-features -- -D warnings
```

**Phase 2 — Security-specific lints:**
```bash
cargo clippy --all-targets --all-features -- \
  -D clippy::unwrap_used \
  -D clippy::expect_used \
  -D clippy::panic \
  -D clippy::indexing_slicing \
  -D clippy::integer_arithmetic \
  -D clippy::todo \
  -D clippy::unimplemented \
  -W clippy::unreachable \
  -W clippy::wildcard_enum_match_arm
```

| Lint | Level | Reason |
|------|-------|--------|
| `unwrap_used` | error | Silent panic in production |
| `expect_used` | error | Panic with message, equally dangerous |
| `panic` | error | Aborts the runtime without cleanup |
| `indexing_slicing` | error | Panic due to out-of-bounds index |
| `integer_arithmetic` | error | Unchecked overflow/underflow |
| `todo` | error | Incomplete code in production |
| `unimplemented` | error | Same as `todo` |
| `unreachable` | warning | May indicate incorrect logic |
| `wildcard_enum_match_arm` | warning | Non-exhaustive enum match |

To silence a specific lint with documented justification:
```rust
#[allow(clippy::unwrap_used)] // SAFETY: field initialized in new(), invariant guaranteed
let val = self.inner.unwrap();
```

### Level 1 Tool Installation

```bash
# Install cargo-audit
cargo install cargo-audit

# Install cargo-deny
cargo install cargo-deny

# Install cargo-geiger
cargo install cargo-geiger
```

---

### Level 2 - Semgrep (Serious Security)

Custom rules in `.semgrep/rules/`:

#### Unsafe Code Detection
- `no-unwrap-in-security-critical`: Detects `unwrap()` in sensitive code
- `no-expect-in-security-critical`: Detects `expect()` in sensitive code
- `no-panic-in-security-critical`: Detects `panic!` in sensitive code

#### File System
- `no-std-fs-without-validation`: Detects `std::fs` usage without validation
- `no-tokio-process-without-validation`: Detects command execution without sanitization

#### 3VA-specific rules
- **Capability bypass**: Detects accesses without `--allow-read`, `--allow-net`, `--allow-env`
- **Sandbox bypass**: Detects symlink escapes, path traversal
- **Host validation**: Validates URLs before network calls
- **Deserialization**: Detects parsing without schema validation
- **JS↔Rust bridge**: Detects unsafe type conversions

### Semgrep Installation

```bash
# Via Python
pip install semgrep

# Or via npm
npm install -g semgrep

# Run
semgrep --config .semgrep/rules/ .
```

---

### Level 3 - Fuzzing

3va ships three coverage-guided libFuzzer targets. All targets enforce hard invariants with `assert!` — any violation is a bug.

| Target | Surface | Key invariants checked |
|--------|---------|------------------------|
| `fuzz_target_1` | Bundler `CodeGenerator` (IIFE + ESM) | No panics on arbitrary JS source |
| `fuzz_permission_sandbox` | `PermissionState`, `VirtualFs`, `VirtualNetwork` | Path-traversal containment; `deny_all` beats any grant; deny beats grant |
| `fuzz_pm_resolver` | `Semver`, `SemverRange`, `DependencyGraph`, `Resolver` | No panics; parse/resolve determinism |

```bash
# Requires nightly toolchain
rustup install nightly
cargo install cargo-fuzz

# Run a target (Ctrl-C to stop; corpus saved automatically)
cargo fuzz run fuzz_target_1
cargo fuzz run fuzz_permission_sandbox
cargo fuzz run fuzz_pm_resolver

# Replay existing corpus without fuzzing
cargo fuzz run fuzz_permission_sandbox -- -runs=0
```

Full documentation: `docs/10-security/04-fuzzing.md`.

---

### Level 4 - Sanitizers

**VERY IMPORTANT**: V8 is written in C++, memory safety is enforced but ASAN is still recommended for development.

```bash
# Instalar Rust nightly
rustup install nightly

# AddressSanitizer
RUSTFLAGS="-Z sanitizer=address" cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu

# UndefinedBehaviorSanitizer
RUSTFLAGS="-Z sanitizer=undefined" cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu

# ThreadSanitizer (if you use Tokio heavily)
RUSTFLAGS="-Z sanitizer=thread" cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu
```

---

### Level 5 - Security Tests

Run specific tests:

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

### Level 6 - Supply Chain

**MANDATORY**: Cargo.lock must be in the repository.

```bash
# Verify Cargo.lock exists
ls Cargo.lock

# Install cargo-vet (recommended)
cargo install cargo-vet

# Audit dependencies
cargo vet
```

**Critical protections**:
- Zip Slip: Validate paths in tarballs
- Symlink escape: Do not follow symlinks without validation
- Compression bombs: Detect ratios >1000:1
- Unicode normalization: Normalize paths

---

### Level 7 - GitHub Advanced Security

**HIGHLY RECOMMENDED**: Configure GitHub Advanced Security.

On GitHub:
1. Settings → Security → Code scanning
2. Enable CodeQL
3. Configure `.github/workflows/codeql.yml`

---

## GitHub Actions Configuration

### Minimum Pipeline

Create `.github/workflows/security.yml`:

```yaml
name: Security Pipeline

on:
  push:
  pull_request:

jobs:
  security:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable

      - name: Format
        run: cargo fmt --check

      - name: Clippy (general)
        run: cargo clippy --all-targets --all-features -- -D warnings

      - name: Clippy (security lints)
        run: |
          cargo clippy --all-targets --all-features -- \
            -D clippy::unwrap_used \
            -D clippy::expect_used \
            -D clippy::panic \
            -D clippy::indexing_slicing \
            -D clippy::integer_arithmetic \
            -D clippy::todo \
            -D clippy::unimplemented \
            -W clippy::unreachable \
            -W clippy::wildcard_enum_match_arm

      - name: Tests
        run: cargo test --all-features

      - name: Audit
        run: cargo install cargo-audit && cargo audit

      - name: Deny
        run: cargo install cargo-deny && cargo deny check

      - name: Geiger
        run: cargo install cargo-geiger && cargo geiger
```

---

## Critical Security Points

### 1. Rust ↔ V8 Boundary (MOST RISKY)
- Each API exposed to JS is an attack surface
- Validate ALL type conversions
- Do not trust unvalidated JS values

### 2. Package Manager
- Supply chain is the main threat
- Block lifecycle scripts: `preinstall`, `install`, `postinstall`, `prepare`, `prepublish`
- Verify tarball integrity

### 3. Capability Enforcement
- Any bypass destroys the security model
- Validate capabilities BEFORE each operation

### 4. Path Canonicalization
- MANY runtimes fail here
- Use `canonicalize()` and validate it starts with the base

### 5. Host API Exposure
- Each function exposed to JS is a potential vector

---

## AI Security Review

For automatic PR analysis, configure:

```bash
# With Semgrep
semgrep --config .semgrep/rules/ --json --output=semgrep.json .

# Generate report
jq '.results | length' semgrep.json
```

---

## Release Gate

Never publish if any of these fail:
- [ ] cargo fmt
- [ ] cargo clippy
- [ ] cargo audit
- [ ] cargo deny
- [ ] cargo geiger
- [ ] semgrep
- [ ] CodeQL
- [ ] fuzzing
- [ ] ASAN/UBSAN
- [ ] security tests

---

---

## Accepted Risk Register

Advisories ignorados en `deny.toml` con justificación documentada.
Cada entrada incluye el path de código afectado y las condiciones que invalidarían la aceptación.

### RUSTSEC-2023-0071 — Marvin Attack (RSA timing side-channel)

| Campo        | Valor                                          |
|--------------|------------------------------------------------|
| Advisory     | RUSTSEC-2023-0071                              |
| Crate        | `rsa`                                          |
| CVE          | CVE-2023-49092                                 |
| CVSS         | 5.9 (Medium)                                   |
| Estado       | **Aceptado — path vulnerable no alcanzable en 3va** |

**Qué es:** El path de `RsaPrivateKey::decrypt()` con PKCS#1 v1.5 del crate `rsa 0.9`
es vulnerable al Marvin Attack: un oracle de timing que permite a un atacante medir la
latencia de operaciones de decryption para recuperar texto plano sin la clave privada.

**Por qué no aplica a 3va:** 3va usa `rsa` **únicamente** para:
- `crypto.generateKeyPair('rsa')` → `RsaPrivateKey::new()` (generación, no vulnerable)
- `crypto.createSign('RSA-SHA*')` → `pkcs1v15::SigningKey::sign()` (firma, no vulnerable)
- `crypto.createVerify('RSA-SHA*')` → `pkcs1v15::VerifyingKey::verify()` (verificación, no vulnerable)

`RsaPrivateKey::decrypt()` **nunca se llama** en el código de 3va. El path vulnerable
es inalcanzable. No existe ni la API JS `crypto.privateDecrypt()` expuesta al usuario.

El ataque además requiere:
1. Un servicio en red que ejecute decryption RSA-PKCS1 y devuelva respuestas medibles.
2. Capacidad de enviar miles de ciphertexts fabricados midiendo latencia sub-milisegundo.

Ninguna condición se cumple — la condición 1 es estructuralmente imposible en 3va.

**Estado del fix upstream:** `rsa 0.10.0-rc.18` contiene el fix pero es una RC que
requiere `digest 0.11` y `rand_core 0.10`. El ecosistema RustCrypto no tiene aún
versiones estables de `sha2`/`sha1` para `digest 0.11`. Migrar cuando `rsa 0.10`
estable se publique.

**Invalidadores — si alguno ocurre, esta aceptación debe revocarse:**
- 3va expone `crypto.privateDecrypt()` u otro path de decryption RSA-PKCS1 por red.
- `rsa 0.10` estable se publica — en ese momento migrar de inmediato.

**Referencias:**
- https://rustsec.org/advisories/RUSTSEC-2023-0071.html
- https://people.redhat.com/~hkario/marvin/

---

### RUSTSEC-2023-0051 & RUSTSEC-2024-0370 (wasmtime deps transitivas)

Aceptados como dependencias transitivas de `wasmtime` sin path de explotación
activo para el uso de 3va. Se revisan en cada upgrade mayor de `wasmtime`.

### RUSTSEC-2025-0057 — fxhash sin mantenimiento

`fxhash` es un crate sin mantenimiento importado transitivamente por `wasmtime`.
Sin CVE activo. Se elimina si `wasmtime` lo descarta.

---

## Post-Quantum Cryptography

3va implementa ML-KEM-768 (NIST FIPS 203) y ML-DSA-65 (NIST FIPS 204) en
el crate `vvva_crypto`. Accesibles desde JS via `require('crypto').pq`.

Las conexiones via `__pqTlsConnect` realizan un intercambio **híbrido**
TLS-clásico + ML-KEM-768, proporcionando confidencialidad post-cuántica
contra adversarios cuánticos futuros.

> **Nota:** El path TLS por defecto (`native-tls`) **no** incluye PQ key
> exchange. Las aplicaciones que requieran PQ forward secrecy deben usar
> `__pqTlsConnect` explícitamente.

---

## Security Contact

To report vulnerabilities: edgarcano.166@gmail.com
Do NOT open a public GitHub issue for security bugs.