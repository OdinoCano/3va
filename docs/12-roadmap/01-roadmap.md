# 01 - HOJA DE RUTA DE DESARROLLO

## 1.1 Visión

3va busca ser el runtime de JavaScript/TypeScript más seguro, superando a Bun en características de ciberseguridad y modelo de permisos.

---

## 1.2 Estado Actual (v0.1.0-dev · 2026-05-19)

### Implementado y funcional

| Módulo | Estado | Notas |
|--------|--------|-------|
| CLI con permisos granulares | ✅ | `run`, `install`, `reinstall`, `update`, `bundle`, `test`, `audit`, `doctor`, `sandbox` |
| Modo accesible (`--accessible`) | ✅ | Conforme EN 301 549 |
| Motor JS (QuickJS) | ✅ | Transpilación TS automática |
| Módulos CommonJS + ESM | ✅ | Carga y resolución de módulos |
| Sistema de permisos (capabilities) | ✅ | `--allow-net`, `--allow-read`, `--allow-write`, `--allow-env`, `--allow-child-process` |
| Prompt interactivo de permisos | ✅ | Habilitado por defecto en `run` |
| Package Manager — `install` | ✅ | npm, Yarn, JSR; versión específica; sugerencias cercanas |
| Package Manager — `reinstall` | ✅ | Forzado |
| Package Manager — `update` | ✅ | Registry-aware; multi-registry; validación `--allow-net` |
| Lockfile con campo `registry` | ✅ | Trazabilidad de origen por paquete |
| Bundler | ✅ | `3va bundle`; transpila TS |
| Test runner | ✅ | Descubrimiento automático; `*.test.ts`, `*.spec.ts` |
| Verificación de firmas (SHA-256/SHA-512) | ✅ | `SignatureVerifier` |
| Malware scanner | ✅ | Análisis estático de dependencias |
| Audit logger | ✅ | Registro de operaciones sensibles |

---

## 1.3 Fases de Desarrollo

### Fase 1: Foundation (Q2 2026) — ✅ COMPLETADO

| Elemento | Estado |
|----------|--------|
| CLI completo con permisos | ✅ |
| Core runtime (event loop Tokio) | ✅ |
| Motor JS QuickJS integrado | ✅ |
| Transpilación TypeScript | ✅ |
| Módulos CommonJS-compatible | ✅ |
| Modo accesible EN 301 549 | ✅ |

### Fase 2: Package Manager (Q3 2026) — ✅ COMPLETADO ANTES DE PLAZO

| Elemento | Estado |
|----------|--------|
| PM básico funcional (install/reinstall) | ✅ |
| Multi-registry (npm, Yarn, JSR, custom) | ✅ |
| Lockfile v3 con campo `registry` | ✅ |
| `update` con seguimiento de origen | ✅ |
| Verificación de firmas | ✅ |
| Scanner de malware | ✅ |
| Audit logger | ✅ |
| Post-install scripts deshabilitados | ✅ |

### Fase 3: Herramientas (Q4 2026) — Parcialmente completado

| Elemento | Estado |
|----------|--------|
| Bundler básico | ✅ Completado |
| Test runner | ✅ Completado |
| Análisis estático | 🔄 Especificado, pendiente integración completa |
| Watch mode (`dev`) | 🔲 Pendiente |
| Inspector / debugger | 🔲 Pendiente |

### Fase 4: LTS (2027)

| Elemento | Estado |
|----------|--------|
| Estabilización API pública | 🔲 |
| Performance tuning | 🔲 |
| Criptografía post-cuántica | 🔲 |
| Release 1.0 LTS | 🔲 |

---

## 1.4 Milestones

| Versión | Fecha objetivo | Features | Estado |
|---------|----------------|----------|--------|
| 0.1.0 | Jun 2026 | CLI + Core + JS + PM + Bundler + Tests | 🔄 En progreso |
| 0.2.0 | Sep 2026 | Estabilización PM + watch mode + debugger | 🔲 |
| 0.3.0 | Nov 2026 | Seguridad avanzada (análisis estático completo) | 🔲 |
| 1.0.0 | Mar 2027 | LTS + post-quantum crypto | 🔲 |

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
| Modo accesible (EN 301 549) | No | No | No | **Sí** |
| Criptografía post-cuántica (roadmap) | No | No | No | **Sí** |

---

*Roadmap sujeto a cambios según feedback y prioridades del proyecto.*
