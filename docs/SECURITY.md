# 3VA Security Documentation

## Philosophy

Security is prioritized over compatibility. We assume:
- QuickJS will have bugs
- crates will have CVEs
- sandbox escapes will exist
- parser bugs will be found
- DOS will be attempted
- bypasses will occur

The question is not "if" but "how well contained".

---

## Pipeline de Seguridad

### Nivel 1 - Cargo Hardening (Obligatorio)

```bash
./scripts/security_verify.sh
```

Ejecuta automáticamente:
- `cargo fmt --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-features`
- `cargo audit --deny-warnings`
- `cargo deny check`
- `cargo geiger` (detección de unsafe)

### Instalación de herramientas de Nivel 1

```bash
# Instalar cargo-audit
cargo install cargo-audit

# Instalar cargo-deny
cargo install cargo-deny

# Instalar cargo-geiger
cargo install cargo-geiger
```

---

### Nivel 2 - Semgrep (Seguridad Seria)

Reglas custom en `.semgrep/rules/`:

#### Detección de código inseguro
- `no-unwrap-in-security-critical`: Detecta `unwrap()` en código sensible
- `no-expect-in-security-critical`: Detecta `expect()` en código sensible
- `no-panic-in-security-critical`: Detecta `panic!` en código sensible

#### Sistema de archivos
- `no-std-fs-without-validation`: Detecta uso de `std::fs` sin validación
- `no-tokio-process-without-validation`: Detecta command execution sin sanitizar

#### Reglas específicas 3VA
- **Capability bypass**: Detecta accesos sin `--allow-read`, `--allow-net`, `--allow-env`
- **Sandbox bypass**: Detecta symlink escapes, path traversal
- **Host validation**: Valida URLs antes de network calls
- **Deserialización**: Detecta parsing sin validación de esquema
- **JS↔Rust bridge**: Detecta conversiones de tipo inseguras

### Instalación de Semgrep

```bash
# Python
pip install semgrep

# O npm
npm install -g semgrep

# Ejecutar
semgrep --config .semgrep/rules/ .
```

---

### Nivel 3 - Fuzzing

Superficies fuzzables:
- CLI parser (`cargo run ...`)
- Package manager (tarballs, package.json, lockfiles)
- Capability parser (`--allow-read`, `--allow-net`, `--deny-env`)
- JS↔Rust bridge (type conversions, host functions)

### Instalación de fuzzing

```bash
# Instalar cargo-fuzz
cargo install cargo-fuzz

# Inicializar fuzzing
cargo fuzz init

# Fuzz targets están en fuzz/fuzz_targets/
```

---

### Nivel 4 - Sanitizers

**MUY IMPORTANTE**: QuickJS usa C internamente, necesitas ASAN.

```bash
# Instalar Rust nightly
rustup install nightly

# AddressSanitizer
RUSTFLAGS="-Z sanitizer=address" cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu

# UndefinedBehaviorSanitizer
RUSTFLAGS="-Z sanitizer=undefined" cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu

# ThreadSanitizer (si usas Tokio intensamente)
RUSTFLAGS="-Z sanitizer=thread" cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu
```

---

### Nivel 5 - Tests de Seguridad

Ejecutar tests específicos:

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

### Nivel 6 - Supply Chain

**OBLIGATORIO**: Cargo.lock debe estar en el repositorio.

```bash
# Verificar Cargo.lock existe
ls Cargo.lock

# Instalar cargo-vet (recomendado)
cargo install cargo-vet

# Auditar dependencias
cargo vet
```

**Protecciones críticas**:
- Zip Slip: Validar paths en tarballs
- Symlink escape: No seguir symlinks sin validación
- Compression bombs: Detectar ratios >1000:1
- Unicode normalization: Normalizar paths

---

### Nivel 7 - GitHub Advanced Security

**MUY RECOMENDADO**: Configurar GitHub Advanced Security.

En GitHub:
1. Settings → Security → Code scanning
2. Enable CodeQL
3. Configurar `.github/workflows/codeql.yml`

---

## Configuración de GitHub Actions

### Pipeline mínimo

Crear `.github/workflows/security.yml`:

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

      - name: Clippy
        run: cargo clippy --all-targets --all-features -- -D warnings

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

## Puntos Críticos de Seguridad

### 1. Boundary Rust ↔ QuickJS (MÁS RIESGOSO)
- Cada API expuesta al JS es superficie de ataque
- Validar TODAS las conversiones de tipo
- No confiar en valores JS sin validar

### 2. Package Manager
- Supply chain es la amenaza principal
- Bloquear lifecycle scripts: `preinstall`, `install`, `postinstall`, `prepare`, `prepublish`
- Verificar integridad de tarballs

### 3. Capability Enforcement
- Cualquier bypass destruye el modelo de seguridad
- Validar capabilities ANTES de cada operación

### 4. Path Canonicalization
- MUCHOS runtimes fallan aquí
- Usar `canonicalize()` y validar que empieza con base

### 5. Host API Exposure
- Cada función expuesta al JS es un vector potencial

---

## AI Security Review

Para análisis automático de PRs, configurar:

```bash
# Con Semgrep
semgrep --config .semgrep/rules/ --json --output=semgrep.json .

# Generar reporte
jq '.results | length' semgrep.json
```

---

## Release Gate

Nunca publicar si falla:
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

## Contacto de Seguridad

Para reportar vulnerabilidades: [edgarcano.166@gmail.com]