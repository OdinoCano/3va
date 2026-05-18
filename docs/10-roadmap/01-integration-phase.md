# 10 - ROADMAP Y SEGUIMIENTO DE TRABAJO

## 10.1 Estado Actual (Completado)
Se ha finalizado exitosamente la "Fase de Aislamiento Estructural". Los siguientes crates y módulos fundamentales ya están programados con sus lógicas core terminadas:
- **`vvva_core`**: Event Loop, Cola de Tareas (`TaskQueue`) y Rueda de Timers (`TimerWheel`).
- **`vvva_js`**: Built-ins base (`console`, `buffer`, `process`, `timers`).
- **`vvva_permissions`**: Sistema de capacidades, sandboxing de rutas (`VirtualFs`), y barreras protectoras (`Enforcement`).
- **`vvva_pm`**: Gestor de paquetes (`fetcher`, `lockfile`, `resolver`).
- **`vvva_bundler`**: Empaquetador con Tree-shaking.
- **`vvva_test`**: API de Testing nativa compatible con IEEE 829.

---

## 10.2 Siguientes Pasos: Fase de Integración y Conexión (WIRING PHASE)

Ahora que poseemos todos los bloques de construcción individuales, el próximo gran paso es **conectarlos entre sí y con el usuario final (CLI)**. A continuación se listan las tareas a desarrollar:

### 10.2.1 Integración del Motor JS (`vvva_js` + `vvva_core`)
- **Problema actual:** Los *builtins* de `vvva_js` existen en archivos, pero el motor QuickJS (en `js/src/lib.rs`) no se los inyecta al contexto global al inicializarse.
- **Acción:** Integrar el `TimerWheel` de `core` con `setTimeout` de `js`. Cuando JS llame a `setTimeout`, debe registrarse en la rueda asíncrona de Rust. Exponer el objeto `console` globalmente.
- **Acción:** Integrar el *Transpilador TS* (posible intercepción de código) antes de pasarlo a `eval`.

### 10.2.2 Integración de Permisos (`vvva_permissions` + `vvva_js`)
- **Problema actual:** Los Enforcers (`FsEnforcer`, `NetEnforcer`) están construidos, pero no bloquean nada porque ninguna operación los está invocando.
- **Acción:** Construir las APIs de `fs` y `fetch` en Rust, inyectarlas en QuickJS, y asegurarse de que internamente llamen a los Enforcers y generen un `throw_permission_error()` si el acceso es denegado.
- **Acción:** Implementar el `AuditLogger` (documentado en `06-permissions/04-audit.md`) para registrar cuándo un Enforcer deniega una petición.

### 10.2.3 Conexión del CLI (`vvva_cli`)
- **Problema actual:** El CLI actual (`main.rs`) parsea argumentos pero solo arranca un "stub" del Runtime. Los comandos como `bundle` o `test` están vacíos.
- **Acción:** Conectar `3va test` para que arranque el framework de `vvva_test` e imprima los resultados.
- **Acción:** Conectar `3va bundle` para que recoja el archivo de entrada e invoque al generador de `vvva_bundler`.
- **Acción:** Conectar `3va install` para invocar al `PackageFetcher` y generar un `.3va-lock`.

### 10.2.4 Auditoría de Paquetes (`vvva_pm`)
- **Problema actual:** Faltan las dos patas de seguridad fundamentales estipuladas en NIS2/eIDAS para los paquetes que se descargan.
- **Acción:** Programar el `MalwareScanner` (análisis del AST del paquete buscando ofuscaciones o comandos shell destructivos).
- **Acción:** Programar el `SignatureVerifier` (verificación de huella criptográfica del tarball).

## 10.3 Resumen de Prioridad
Para cualquier IA o desarrollador asumiendo la continuidad de este proyecto, se recomienda estricto orden de ataque:

1. **Prioridad 1:** Conectar el CLI (`main.rs`) a los crates nuevos (`bundler`, `test`, `pm`).
2. **Prioridad 2:** Inyectar los *Builtins* (Console, Timers, FS) en QuickJS.
3. **Prioridad 3:** Terminar las comprobaciones criptográficas en el Package Manager.
