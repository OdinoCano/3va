# 09 — CUMPLIMIENTO NIST SSDF (SP 800-218)

Esta página relaciona las prácticas del Secure Software Development Framework
(NIST SP 800-218 v1.1) con la evidencia concreta que existe en este repositorio.
Sirve para evaluaciones de riesgo de proveedores, para el formulario de
autoatestación de CISA y como base técnica para el Cyber Resilience Act de la UE.

Estados: ✅ cubierto · ⚠️ parcial · ❌ pendiente.

## PO — Preparar la organización

| Práctica | Estado | Evidencia |
|----------|--------|-----------|
| PO.1 Requisitos de seguridad definidos | ✅ | [SECURITY.md](../../SECURITY.md) (alcance, garantías), modelo de permisos en [06-permissions](../06-permissions/) |
| PO.2 Roles y responsabilidades | ✅ | [.github/CODEOWNERS](../../.github/CODEOWNERS): revisión obligatoria en permisos, builtins, CI y dependencias |
| PO.3 Cadena de herramientas segura | ✅ | Toolchain fijado, builds con `--locked`, acciones de GitHub versionadas |
| PO.4 Criterios de verificación | ✅ | Gates de CI: fmt, clippy `-D warnings`, tests, `cargo-deny`, fuzz smoke, gitleaks |
| PO.5 Entornos seguros | ⚠️ | Secretos de publicación aislados en el environment `crates-io`. Pendiente: fijar las acciones por SHA en lugar de por tag |

## PS — Proteger el software

| Práctica | Estado | Evidencia |
|----------|--------|-----------|
| PS.1 Proteger el código contra cambios no autorizados | ⚠️ | CODEOWNERS y revisión por PR. Pendiente: exigir commits firmados en la protección de rama |
| PS.2 Mecanismo de verificación de integridad | ✅ | Firmas cosign sin clave y checksums SHA-256 en cada release ([release.yml](../../.github/workflows/release.yml)) |
| PS.3 Archivar y proteger cada release | ✅ | Procedencia SLSA L3 (generador genérico) y SBOM CycloneDX 1.5 firmado por release |

## PW — Producir software bien asegurado

| Práctica | Estado | Evidencia |
|----------|--------|-----------|
| PW.1 Diseño para la seguridad | ✅ | Permisos deny-by-default, sin scripts post-install, grants por dependencia |
| PW.2 Revisión del diseño | ✅ | Revisiones de seguridad documentadas (p. ej. "PM Phase C security review") |
| PW.4 Reutilizar software bien asegurado | ✅ | `cargo-deny` (advisories, licencias, bans), `cargo-audit`, Dependabot y `cargo vet` como gate de CI, con auditorías importadas de Mozilla, Google y Bytecode Alliance ([supply-chain/](../../supply-chain/)) |
| PW.5 Prácticas de codificación segura | ✅ | Reglas Semgrep, clippy, inventario de `unsafe` con cargo-geiger |
| PW.6 Configuración segura del compilador | ✅ | Perfil release con `--locked`. Sanitizers ASan/UBSan en CI |
| PW.7 Revisión y análisis del código | ✅ | CodeQL, Semgrep, revisión por PR |
| PW.8 Pruebas ejecutables | ✅ | Tests de sandbox/capacidades, fuzzing con `cargo-fuzz`, test262 |
| PW.9 Configuración segura por defecto | ✅ | Todas las capacidades bloqueadas por defecto |
| PW.4.x Criptografía validada | ✅ | Build FIPS 140-3 (`--features fips`) sobre AWS-LC; algoritmos no aprobados rechazados ([10-fips.md](10-fips.md)) |

## RV — Responder a vulnerabilidades

| Práctica | Estado | Evidencia |
|----------|--------|-----------|
| RV.1 Identificar vulnerabilidades continuamente | ✅ | `security-audit.yml` programado, Dependabot, OpenSSF Scorecard ([scorecard.yml](../../.github/workflows/scorecard.yml)) |
| RV.2 Evaluar, priorizar y remediar | ✅ | SLA en [SECURITY.md](../../SECURITY.md#response-times-sla): parche en 8 días para Critical/High, calculado a partir del historial medido |
| RV.3 Analizar causas raíz | ⚠️ | Se registran los riesgos aceptados en [docs/SECURITY.md](../SECURITY.md). Pendiente: postmortem por cada advisory propio |

## Pendientes fuera del repositorio

- **Pentest o auditoría externa** del sandbox de permisos y de `vvva_crypto`.
- **Certificado CMVP de AWS-LC FIPS 4.0.** El build `--features fips` ya enruta toda la criptografía y TLS del runtime por el módulo AWS-LC FIPS 4.0, que está *en proceso* ante NIST. Ver [10-fips.md](10-fips.md).
- **Badge de OpenSSF Best Practices.** Requiere registrarse en bestpractices.dev.
- **Contrato comercial** (soporte, responsabilidad) con CANOTECH Solutions.
