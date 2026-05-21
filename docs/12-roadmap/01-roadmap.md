# 01 - HOJA DE RUTA DE DESARROLLO

## 1.1 Visión

3va busca ser el runtime de JavaScript/TypeScript más seguro, superando a Bun en características de ciberseguridad y modelo de permisos.

---

## 1.2 Estado Actual (v0.1.0-dev · 2026-05-21)

### Implementado y funcional

| Módulo | Estado | Notas |
|--------|--------|-------|
| CLI con permisos granulares | ✅ | `run`, `install`, `reinstall`, `update`, `bundle`, `test`, `audit`, `doctor`, `sandbox`, `dev` — completos |
| Modo accesible (`--accessible`) | ✅ | Conforme EN 301 549 |
| Motor JS (QuickJS) | ✅ | Transpilación TS automática |
| Módulos CommonJS + ESM | ✅ | `EsmResolver` + `EsmLoader`; import/export estático y dinámico |
| async/await y cadenas Promise | ✅ | Bucle de microtareas completo |
| Sistema de permisos (deny-by-default) | ✅ | `FileRead`, `FileWrite`, `Network`, `EnvAccess`, `SpawnProcess`, `FFI` |
| Prompt interactivo de permisos | ✅ | `PermissionState`; habilitado por defecto en `run` |
| Package Manager — `install` | ✅ | npm, Yarn, JSR; versión específica; sugerencias cercanas |
| Package Manager — `reinstall` | ✅ | Forzado |
| Package Manager — `update` | ✅ | Registry-aware; multi-registry; validación `--allow-net` |
| Lockfile con campo `registry` | ✅ | Trazabilidad de origen por paquete; resolución semver |
| Verificación de firmas (SHA-256/SHA-512) | ✅ | `SignatureVerifier` |
| Malware scanner | ✅ | Análisis estático de `node_modules` |
| Scanner de secretos | ✅ | `SecretsScanner`; 16 patrones (AWS, GitHub, GitLab, Stripe, Slack, SendGrid, Twilio, claves privadas, JWT, tokens npm, contraseñas, API keys, cadenas de conexión DB) |
| Auditoría OSV | ✅ | 3 fases (malware + CVE + secretos); caché 24 h; flags `--deny`/`--json`/`--secrets`/`--update-cache` |
| Bundler | ✅ | Tree shaking, code splitting (`--split`), minificación (`--minify`), source maps (`--source-map`), watch mode con notificador real |
| Test runner | ✅ | `describe`/`test`/`expect`; matchers completos; snapshots (`toMatchSnapshot` + `--update-snapshots`); `--watch`; `--coverage`; E/S de archivos de snapshot |
| Sandbox REPL | ✅ | Multi-línea; `.help`/`.exit`/`.clear`/`.allow-read`/`.allow-net`/`.permissions`; detección TTY |
| Servidor de desarrollo (`dev`) | ✅ | `--port`/`--host`/`--open`/`--public-dir`; HMR vía SSE (`/__hmr`); inyección de cliente HMR; archivos estáticos; SPA fallback; rebuild con debounce 300 ms |
| Audit logger | ✅ | Registro de operaciones sensibles |
| Crate `vvva_crypto` | ✅ | Utilidades de preparación post-cuántica (crate independiente) |
| Suite de tests | ✅ | 58 tests de integración (12 fases, 100 % passing); 287 tests unitarios |

---

## 1.3 Fases de Desarrollo

### Fase 1: Foundation (Q2 2026) — ✅ COMPLETADO

| Elemento | Estado |
|----------|--------|
| CLI completo con permisos | ✅ |
| Core runtime (event loop Tokio) | ✅ |
| Motor JS QuickJS integrado | ✅ |
| Transpilación TypeScript | ✅ |
| Módulos CommonJS + ESM | ✅ |
| async/await y cadenas Promise | ✅ |
| Modo accesible EN 301 549 | ✅ |

### Fase 2: Package Manager (Q3 2026) — ✅ COMPLETADO ANTES DE PLAZO

| Elemento | Estado |
|----------|--------|
| PM básico funcional (install/reinstall/update) | ✅ |
| Multi-registry (npm, Yarn, JSR) | ✅ |
| Lockfile con campo `registry` y resolución semver | ✅ |
| Verificación de firmas (SHA-256/SHA-512) | ✅ |
| Scanner de malware (análisis estático) | ✅ |
| Scanner de secretos (16 patrones) | ✅ |
| Auditoría OSV 3 fases + caché 24 h | ✅ |
| Audit logger | ✅ |
| Post-install scripts deshabilitados | ✅ |

### Fase 3: Herramientas (Q4 2026) — ✅ COMPLETADO ANTES DE PLAZO

| Elemento | Estado |
|----------|--------|
| Bundler (tree shaking, code splitting, minificación, source maps) | ✅ Completado |
| Watch mode en bundler (notificador real) | ✅ Completado |
| Test runner (matchers, snapshots, coverage, watch) | ✅ Completado |
| Sandbox REPL con detección TTY | ✅ Completado |
| Servidor de desarrollo con HMR | ✅ Completado |
| Inspector / debugger / breakpoints | 🔲 Pendiente |

### Fase 4: LTS (2027)

| Elemento | Estado |
|----------|--------|
| Inspector / debugger / breakpoints | 🔲 |
| Carga de módulos WebAssembly (WASM) | 🔲 |
| Perfilado de rendimiento / flamegraph | 🔲 |
| Soporte de módulos nativos (NAPI) | 🔲 |
| Criptografía post-cuántica integrada en TLS | 🔲 |
| Estabilización API pública | 🔲 |
| Release 1.0 LTS | 🔲 |

---

## 1.4 Milestones

| Versión | Fecha objetivo | Features | Estado |
|---------|----------------|----------|--------|
| 0.1.0 | Jun 2026 | CLI + Core + JS (ESM/CJS/async) + PM + Bundler + Test runner + Dev server + Seguridad (malware + secretos + OSV) | ✅ Feature-complete; en estabilización |
| 0.2.0 | Sep 2026 | Inspector/debugger + WASM + perfilado de rendimiento | 🔲 |
| 0.3.0 | Nov 2026 | Soporte NAPI + post-quantum TLS + benchmarks públicos | 🔲 |
| 1.0.0 | Mar 2027 | LTS + API estable + post-quantum crypto completamente integrada | 🔲 |

---

## 1.5 Ventajas vs Competencia

| Feature | Node.js | Deno | Bun | **3va** |
|---------|---------|------|-----|---------|
| Permisos granulares | No | Sí | No | **Sí** |
| Red denegada por defecto en PM | No | Sí | No | **Sí** |
| Multi-registry con trazabilidad de origen | No | No | No | **Sí** |
| Post-install scripts deshabilitados | No | No | No | **Sí** |
| Análisis de malware integrado | No | No | No | **Sí** |
| Verificación de firmas obligatoria | No | No | No | **Sí** |
| Detección de secretos integrada | No | No | No | **Sí** |
| Auditoría OSV 3 fases con caché | No | Parcial | No | **Sí** |
| Servidor de desarrollo con HMR | No | Sí | Sí | **Sí** |
| Modo accesible (EN 301 549) | No | No | No | **Sí** |
| Criptografía post-cuántica (crate listo) | No | No | No | **Sí** |

---

*Roadmap sujeto a cambios según feedback y prioridades del proyecto.*
