# 05 - SCRIPTS DE TEST Y VERIFICACIÓN

## 5.1 Script de Integración (`integration_tests.sh`)

Este script valida el funcionamiento completo de 3va con todos los registries soportados.

### Ubicación
```
scripts/integration_tests.sh
```

### Uso
```bash
./scripts/integration_tests.sh
```

### Fases de Test

| Fase | Descripción | Verifica |
|------|-------------|----------|
| FASE 1 | NPM Registry | lodash desde registry.npmjs.org |
| FASE 2 | Yarn Registry | axios desde registry.yarnpkg.com |
| FASE 3 | JSR Registry | @std/path desde jsr.io |
| FASE 4 | Import Verification | Coexistencia de paquetes |
| FASE 5 | Basic Execution | JS/TypeScript puro |
| FASE 6 | Diagnostics | doctor, help, version |
| FASE 7 | Bundle | basic, minify, split |
| FASE 8 | Test Runner | runner y --watch |
| FASE 9 | Update/Reinstall | comandos disponibles |
| FASE 10 | Sandbox Security | secure-by-default |

### Requisitos
- Binary compilado en `target/debug/3va`
- Network acceso a:
  - registry.npmjs.org
  - registry.yarnpkg.com
  - jsr.io

### Salida Esperada
```
╔════════════════════════════════════════════════════════════════╗
║                         RESUMEN FINAL                        ║
╚════════════════════════════════════════════════════════════════╝

  Total Tests:   25
  Passed:        25
  Failed:        0
  Success Rate: 100.0%

✓ TODOS LOS TESTS DE INTEGRACIÓN PASARON

Registries verificados:
  - npm (registry.npmjs.org) ✓
  - yarn (registry.yarnpkg.com) ✓
  - jsr (jsr.io) ✓
```

---

## 5.2 Script de Seguridad (`security_verify.sh`)

Ejecuta el pipeline completo de verificación de seguridad.

### Ubicación
```
scripts/security_verify.sh
```

### Uso
```bash
./scripts/security_verify.sh
```

### Niveles de Verificación

| Nivel | Verificación | Estado Requerido |
|-------|--------------|------------------|
| 1 | Cargo Hardening | fmt, clippy, test, audit, deny, geiger |
| 2 | Semgrep | Reglas custom de seguridad |
| 3 | Fuzzing | Parser y package manager |
| 4 | Sanitizers | ASAN, UBSAN |
| 5 | Security Tests | path_traversal, sandbox_escape, etc. |
| 6 | Supply Chain | Cargo.lock, cargo vet |
| 7 | CodeQL | GitHub Advanced Security |

### Instalación de Herramientas
```bash
# Nivel 1
cargo install cargo-audit
cargo install cargo-deny
cargo install cargo-geiger

# Nivel 2
pip install semgrep  # o: npm install -g semgrep

# Nivel 3
cargo install cargo-fuzz
```

### Instalación de Herramientas (automático)
El script intenta instalar las herramientas faltantes automáticamente.

### Salida Esperada
```
══════════════════════════════════════════════════════════
                    RESUMEN DE SEGURIDAD                   
══════════════════════════════════════════════════════════

Failures:  0
Warnings:  X

✓ Pipeline de seguridad PASSED
```

---

## 53. Pipeline CI/CD Recomendado

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

## 5.4 Tests de Seguridad Específicos

El proyecto incluye tests de seguridad en `tests/security/`:

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

*Scripts conformes a IEEE 829 test documentation standard.*