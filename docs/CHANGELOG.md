# Changelog

Todos los cambios notables en **3va** se documentan aquí.
Formato: [Keep a Changelog 1.0.0](https://keepachangelog.com/en/1.0.0/) · Versioning: [SemVer](https://semver.org/).

---

## [Unreleased]

### Añadido
- Subcomando `3va update` con seguimiento de registry por paquete.
- Campo `registry` en `3va-lock.json` (en `packages` y `dependencies`) para registrar el origen de cada paquete instalado.
- Lógica de preservación de registries en el lockfile al regenerarlo: los registries de paquetes ya instalados no se pierden cuando se instala un paquete nuevo.
- Validación de `--allow-net` en `3va update`: el CLI inspecciona el lockfile, agrupa paquetes por registry y muestra el comando exacto que el usuario debe ejecutar si falta algún host autorizado.
- Soporte multi-registry en un mismo proyecto (e.g., `axios` desde `registry.npmjs.org` y `@std/path` desde `jsr.io`).
- Métodos `registry_for()`, `registries_needed()` y `set_registry()` en `Lockfile` (`crates/pm/src/lockfile.rs`).

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
- Puerta de seguridad: cualquier intento de instalar sin `--allow-net` muestra un error explicativo y sugiere los comandos correctos — ninguna llamada de red silenciosa.
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
