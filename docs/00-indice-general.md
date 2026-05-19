# ÍNDICE GENERAL - Documentación Técnica 3va

## Volumen 1: Introducción y Visión del Proyecto

- [01-resumen-ejecutivo.md](01-intro/01-resumen-ejecutivo.md) - Resumen ejecutivo del proyecto
- [02-alcance.md](01-intro/02-alcance.md) - Alcance y objetivos
- [03-definiciones.md](01-intro/03-definiciones.md) - Definiciones y abreviaturas
- [04-referencias.md](01-intro/04-referencias.md) - Referencias normativas

## Volumen 2: Arquitectura del Sistema

- [01-arquitectura-general.md](02-arquitectura/01-arquitectura-general.md) - Arquitectura general del sistema
- [02-diseno-componentes.md](02-arquitectura/02-diseno-componentes.md) - Diseño de componentes
- [03-interfaces.md](02-arquitectura/03-interfaces.md) - Interfaces y comunicación
- [04-flujos-datos.md](02-arquitectura/04-flujos-datos.md) - Flujos de datos

## Volumen 3: Especificación del CLI

- [01-interfaz-linea-comandos.md](03-cli/01-interfaz-linea-comandos.md) - Interfaz de línea de comandos
- [02-comandos.md](03-cli/02-comandos.md) - Comandos disponibles
- [03-opciones.md](03-cli/03-opciones.md) - Opciones y flags
- [04-ejemplos.md](03-cli/04-ejemplos.md) - Ejemplos de uso

## Volumen 4: Core Runtime

- [01-event-loop.md](04-core/01-event-loop.md) - Event loop y scheduler
- [02-modulo-system.md](04-core/02-modulo-system.md) - Módulos del sistema
- [03-globals.md](04-core/03-globals.md) - Objetos globales
- [04-process.md](04-core/04-process.md) - Gestión de procesos

## Volumen 5: Motor JavaScript

- [01-quickjs-integration.md](05-js-engine/01-quickjs-integration.md) - Integración con QuickJS
- [02-modulo-loader.md](05-js-engine/02-modulo-loader.md) - Carga de módulos
- [03-polyfills.md](05-js-engine/03-polyfills.md) - Polyfills y shims
- [04-web-apis.md](05-js-engine/04-web-apis.md) - APIs web compatibles

## Volumen 6: Sistema de Permisos

- [01-capability-model.md](06-permissions/01-capability-model.md) - Modelo de capacidades
- [02-enforcement.md](06-permissions/02-enforcement.md) - Aplicación de políticas
- [03-sandboxing.md](06-permissions/03-sandboxing.md) - Sandboxing y aislamiento
- [04-audit.md](06-permissions/04-audit.md) - Auditoría y logging

## Volumen 7: Package Manager

- [01-especificacion-pm.md](07-pm/01-especificacion-pm.md) - Especificación del PM
- [02-resolucion.md](07-pm/02-resolucion.md) - Resolución de dependencias
- [03-sandboxing.md](07-pm/03-sandboxing.md) - Sandboxing de paquetes
- [04-lockfile.md](07-pm/04-lockfile.md) - Formato de lockfile

## Volumen 8: Bundler

- [01-especificacion-bundler.md](08-bundler/01-especificacion-bundler.md) - Especificación del bundler
- [02-transpilation.md](08-bundler/02-transpilation.md) - Transpilación TS/JSX
- [03-tree-shaking.md](08-bundler/03-tree-shaking.md) - Tree shaking
- [04-code-splitting.md](08-bundler/04-code-splitting.md) - Code splitting

## Volumen 9: Test Runner

- [01-especificacion-tests.md](09-testing/01-especificacion-tests.md) - Especificación de tests
- [02-matchers.md](09-testing/02-matchers.md) - Matchers y aserciones
- [03-snapshots.md](09-testing/03-snapshots.md) - Snapshots
- [04-watch-mode.md](09-testing/04-watch-mode.md) - Modo watch
- [05-scripts.md](09-testing/05-scripts.md) - Scripts de test y verificación

## Volumen 10: Funciones de Seguridad Avanzadas

- [01-static-analysis.md](10-security/01-static-analysis.md) - Análisis estático
- [02-malware-scanner.md](10-security/02-malware-scanner.md) - Scanner de malware
- [03-secrets-detection.md](10-security/03-secrets-detection.md) - Detección de secretos
- [04-fuzzing.md](10-security/04-fuzzing.md) - Fuzzing integrado
- [05-post-quantum.md](10-security/05-post-quantum.md) - Criptografía post-cuántica

## Volumen 11: APIs y Referencia

- [01-js-api.md](11-api/01-js-api.md) - API JavaScript pública
- [02-interna-api.md](11-api/02-interna-api.md) - API interna del runtime
- [03-error-codes.md](11-api/03-error-codes.md) - Códigos de error

## Volumen 12: Hoja de Ruta y LTS

- [01-roadmap.md](12-roadmap/01-roadmap.md) - Hoja de ruta de desarrollo
- [02-lts-criteria.md](12-roadmap/02-lts-criteria.md) - Criterios LTS
- [03-release-process.md](12-roadmap/03-release-process.md) - Proceso de release
- [04-compatibility.md](12-roadmap/04-compatibility.md) - Compatibilidad retroactiva

## Changelog

- [CHANGELOG.md](../CHANGELOG.md) — Historial de cambios por versión (Keep a Changelog 1.0.0)

---

**Identificador del documento:** 3VA-SPEC-2026-001
**Versión:** 1.1.0
**Fecha:** 2026-05-19
**Clasificación:** Público
**Estado:** Borrador

---

*Documento conforme a ISO/IEC/IEEE 29148 y estándares europeos de documentación técnica.*