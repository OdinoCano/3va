# Changelog

Todos los cambios notables en **3va** se documentan aquí.
Formato: [Keep a Changelog 1.0.0](https://keepachangelog.com/en/1.0.0/) · Versioning: [SemVer](https://semver.org/).

---

## [Unreleased]

### Añadido

- `3va dev` — servidor de desarrollo completo:
  - Flags `--port <N>` (default 3000), `--host <H>` (default 127.0.0.1), `--open`, `--public-dir <D>`.
  - HMR via Server-Sent Events en el endpoint `/__hmr`.
  - Script cliente HMR inyectado automáticamente antes de `</body>` en todos los HTML servidos.
  - Servicio de archivos estáticos con MIME types correctos (15 tipos soportados).
  - Fallback SPA: rutas desconocidas sirven `public/index.html`.
  - Rebuild automático con debounce de 300 ms al detectar cambios en archivos `.js`, `.ts`, `.jsx`, `.tsx`.
  - Página de desarrollo integrada cuando no existe `public/index.html`.
- `3va audit --secrets` — Fase 3 de auditoría: detección de secretos hardcodeados en dependencias (claves AWS, tokens GitHub, claves privadas PEM, tokens JWT, claves Stripe y otros patrones comunes) via `SecretsScanner`.
- `3va audit --json` — salida machine-readable con estructura `{ passed, phases: { malware, osv, secrets } }`; suprime completamente el output human-readable.
- `audit_packages_silent()` en `vvva_pm` — variante de auditoría sin output a consola, usada internamente en modo `--json`.
- `3va sandbox` — REPL interactivo completo:
  - Soporte multi-línea con detector de brackets balanceados (paréntesis, corchetes, llaves).
  - Comandos de sesión: `.help`, `.exit`, `.clear`, `.allow-read <path>`, `.allow-net <host>`, `.permissions`.
  - Formato de salida estilo Node.js: objetos como JSON, `undefined` explícito para declaraciones.
  - Detección de TTY: en pipes y entornos CI (stdin no-TTY), sale inmediatamente sin bloquear.
- `3va test --watch` — re-ejecuta la suite automáticamente al detectar cambios en archivos.
- `3va test --coverage` — informe de cobertura de líneas y ramas al finalizar la ejecución.
- `3va test --update-snapshots` / `-u` — sobreescribe snapshots existentes con los valores actuales.
- `3va bundle --split` — code splitting; `--minify` — minificación; `--source-map` — generación de mapa de fuentes.
- ESM completo: `EsmResolver` y `EsmLoader` en `vvva_js::esm`; soporte de `import`/`export` con rutas relativas, re-exportaciones y módulos TypeScript.
- Soporte completo de async/await y Promise chains mediante el loop de microtasks `execute_pending_job`.
- Watch mode del bundler (`start_watch_mode`) con watcher `notify` real (anteriormente era un stub sin implementación).
- Soporte de bloques `describe` y snapshots (`toMatchSnapshot`) en el test runner.
- `list_granted()` en `PermissionState` — expone la lista de capabilities concedidas en la sesión actual.
- Subcomando `3va update` con seguimiento de registry por paquete.
- Campo `registry` en `3va-lock.json` (en `packages` y `dependencies`) para registrar el origen de cada paquete instalado.
- Lógica de preservación de registries en el lockfile al regenerarlo: los registries de paquetes ya instalados no se pierden al instalar nuevos paquetes.
- Validación de `--allow-net` en `3va update`: el CLI inspecciona el lockfile, agrupa paquetes por registry y muestra el comando exacto a ejecutar si falta algún host autorizado.
- Soporte multi-registry en un mismo proyecto (e.g., `axios` desde `registry.npmjs.org` y `@std/path` desde `jsr.io`).
- Métodos `registry_for()`, `registries_needed()` y `set_registry()` en `Lockfile` (`crates/pm/src/lockfile.rs`).
- 11 tests de integración en `crates/test/tests/runner_integration.rs`.
- 12 tests unitarios en `crates/pm/src/auditor.rs`.
- 28 tests en `crates/js/tests/pipeline.rs` (ESM, async/await, TypeScript, permisos).
- Suite de integración `scripts/integration_tests.sh`: 58 tests en 12 fases (100% passing).

### Corregido

- `is_esm_source()` dejaba de escanear al encontrar la primera línea que no era un import; ahora escanea el archivo completo con tracking de comentarios de bloque.
- Permiso de snapshot fallaba cuando el test file estaba en `/tmp/` (TempDir); ahora se concede `FileRead`/`FileWrite` al directorio padre del test file.
- `audit --json` emitía output human-readable antes del JSON porque el malware scanner escribía directamente a stdout; resuelto mediante `audit_packages_silent()`.
- `run_audit_human` retornaba antes de alcanzar la Fase 3 si la Fase 1 (malware) o la Fase 2 (OSV) producían un error; ahora las tres fases son resilientes a fallos individuales y siempre se ejecutan de forma independiente.

---

## [0.1.0-dev] - 2026-05-19

> Versión de desarrollo activa. Aún no publicada como release estable.

### Añadido

#### Package Manager (`crates/pm`)
- `3va install <package>[@<version>] --allow-net=<registry-host>` — instalación segura desde npm, Yarn o JSR.
- `3va reinstall <package> --allow-net=<registry-host>` — reinstalación forzada.
- Derivación automática del registry a partir del host en `--allow-net` (no se necesita flag `--registry` separado).
- Soporte para tres registries integrados: `registry.npmjs.org`, `registry.yarnpkg.com`, `jsr.io`.
- Soporte para paquetes con scope (`@scope/name`, obligatorio en JSR).
- Verificación de existencia del paquete antes de instalar.
- Resolución de versión: si no se especifica, usa `dist-tags.latest`; si la versión solicitada no existe, muestra las 5 más cercanas por distancia semver.
- Sugerencias de versiones en formato `name@version`.
- Puerta de seguridad: cualquier intento de instalar sin `--allow-net` muestra un error explicativo y sugiere el comando correcto — ninguna llamada de red silenciosa.
- Detección de paquete ya instalado: evita reinstalación accidental y sugiere `reinstall`.
- Actualización de `package.json` y `3va-lock.json` tras cada instalación exitosa.
- Verificación de firmas vía `SignatureVerifier` (SHA-256/SHA-512).
- API JSR: endpoint `/api/scopes/{scope}/packages/{name}/versions`.
- Algoritmo de distancia semver: score = `major × 1_000_000 + minor × 1_000 + patch`.

#### CLI (`crates/cli`)
- Subcomandos: `run`, `install`, `reinstall`, `update`, `dev`, `bundle`, `test`, `audit`, `doctor`, `sandbox`.
- Flag global `--accessible` para modo accesible (sin colores ni animaciones, conforme EN 301 549).
- Permisos granulares en `run`: `--allow-read`, `--allow-write`, `--allow-net`, `--allow-env`, `--allow-child-process`.
- Prompt interactivo de permisos habilitado por defecto en `run`.

#### Motor JavaScript (`crates/js`)
- Integración con QuickJS vía `rquickjs`.
- Transpilación automática de TypeScript al ejecutar `.ts`.
- Sistema de módulos CommonJS-compatible.
- APIs globales: `console`, `fetch`, `fs` (restringidos por permisos), timers.

#### Bundler (`crates/bundler`)
- `3va bundle <input> --output <output>` — empaquetado de aplicaciones.
- Transpilación TypeScript en el proceso de bundle.

#### Test Runner (`crates/test`)
- `3va test [paths]` — ejecución de suites de prueba.
- Descubrimiento automático de archivos `*.test.ts`, `*.test.js`, `*.spec.*`.

#### Seguridad
- `SignatureVerifier`: cálculo y verificación de hashes SHA-256 y SHA-512 de archivos.
- `MalwareScanner`: análisis estático de dependencias.
- `AuditLogger`: registro de operaciones sensibles.
- Prompting interactivo para solicitar permisos en tiempo de ejecución.
- Post-install scripts deshabilitados por defecto.

### Cambiado
- El flag `--registry` fue eliminado del diseño. El registry se determina exclusivamente por el host autorizado en `--allow-net` — coherente con el modelo de capacidades de 3va.

### Arquitectura
- Workspace Cargo con crates: `vvva_core`, `vvva_cli`, `vvva_permissions`, `vvva_js`, `vvva_pm`, `vvva_bundler`, `vvva_test`.
- Rust edition 2024.
- Async runtime: Tokio.

---

## Formato de entradas

Cada versión sigue la estructura:

```
## [X.Y.Z] - YYYY-MM-DD

### Añadido     — funcionalidad nueva
### Cambiado    — cambios en funcionalidad existente
### Obsoleto    — funcionalidad que se eliminará en futuras versiones
### Eliminado   — funcionalidad eliminada
### Corregido   — corrección de errores
### Seguridad   — parches de vulnerabilidades (referenciar CVE si aplica)
```

---

*Conforme a Keep a Changelog 1.0.0 y SemVer 2.0.0.*
