# 10 - ROADMAP Y SEGUIMIENTO DE TRABAJO

## 10.1 Estado Actual (Completado)
Se ha finalizado exitosamente la "Fase de Integración" (Wiring Phase). Los siguientes componentes están conectados y funcionales:

### ✅ CLI Conectado
- `3va run` → Lee archivos y ejecuta con `JsEngine::eval()`
- `3va bundle` → Invoca a `vvva_bundler::bundle_file()`
- `3va test` → Invoca a `vvva_test::run_tests()`
- `3va install` → Invoca a `vvva_pm::install_package()`

### ✅ Motor JS Integrdo
- **Built-ins injectados**: `console`, `timers`, `buffer`, `process`, `fetch`, `fs`
- **Console expandido**: log, warn, error, info, debug
- **Permisos conectados**: El motor recibe `PermissionState` y lo usa internamente

### ✅ Sistema de Permisos
- `AuditLogger` implementado en `permissions/src/audit.rs`
- APIs de `fs` y `fetch` con verificación de permisos
- Enforcers integrados en los builtins

### ✅ Seguridad de Paquetes (NIS2/eIDAS)
- `MalwareScanner` implementado en `pm/src/malware_scanner.rs`
- `SignatureVerifier` implementado en `pm/src/signature_verifier.rs`
- Detección de: fork bombs, recursive deletes, curl|wget|sh, crypto mining, backdoors

### ✅ TimerWheel
- Integración con `setTimeout`/`setInterval` en `js/src/builtins/timers.rs`
- TimerId expuesto públicamente para uso externo

---

## 10.2 Fase de Estabilización (Completado)

### ✅ Transpilador TypeScript
- `js/src/transpiler.rs` — stripper de tipos en puro Rust (sin deps extra)
- Elimina: `interface`, `type`, `declare`, `import type`, `export type`
- Elimina anotaciones inline: `const x: string` → `const x`
- Elimina `as TypeName`, modificadores de acceso, `!` non-null, genéricos
- Integrado en `JsEngine::eval_file()` — transpilación automática para `.ts`/`.tsx`
- 16 tests de transpilación, todos passing

### ✅ Runtime Async (Event Loop)
- `TimerManager` reimplementado en `js/src/builtins/timers.rs` con `thread_local!`
- Nativos Rust: `__nativeSetTimeout`, `__nativeSetInterval`, `__nativeClearTimer`
- JS wrappers reales: `setTimeout`/`setInterval` registran callbacks y llaman nativos
- `__fireTimer(id)` ejecuta el callback cuando el timer expira
- `JsEngine::run_event_loop()` — drena todos los timers pendientes después de `eval_file()`
- CLI: `3va run` ahora ejecuta el event loop automáticamente tras la ejecución

### ✅ Módulo System (CommonJS require)
- `js/src/builtins/modules.rs` — `require()` respaldado por Rust
- Cache de módulos vía `global.__requireCache`
- Wrapper CJS: `(function(exports, module, __filename, __dirname) { ... })`
- Resolución de paths relativos con extensiones `.js` y `.ts`
- Verificación de permisos (`Capability::FileRead`) antes de leer archivos
- Transpilación automática de `.ts` en `require()`

### ✅ Lockfile y Cache
- `install_package()` lee/crea `package.json`, resuelve deps y genera `3va-lock.json`
- Formato compatible con npm lockfile v3 con extensiones de seguridad 3va

---

## 10.3 Siguientes Pasos

1. **ESM import/export** — usar `rquickjs::Module` para carga nativa de módulos ES
2. **Async/await real** — integrar Promises de QuickJS con Tokio para I/O no bloqueante
3. **REPL / Sandbox interactivo** — implementar `3va sandbox`
4. **Dev server** — implementar `3va dev` con hot-reload
