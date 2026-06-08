# Changelog — 2026-06-05

## Firewall interno HTTP (`vvva_firewall`) + ESM→CJS transparente

---

### Nuevas características

#### `vvva_firewall` — crate de firewall de red

Nuevo crate `vvva_firewall` (v2.0.0) con protección completa del servidor HTTP integrado:

| Protección | Mecanismo |
|------------|-----------|
| **Slowloris** | Timeout por línea de cabecera (`header_timeout_ms`) |
| **RUDY** | Timeout de lectura del cuerpo (`body_timeout_ms`) |
| **Header flood** | Límite de número (`max_header_count`) y tamaño (`max_header_bytes`) de cabeceras |
| **Rate-based DDoS** | Token bucket por IP: `rate_limit_rps` / `rate_limit_burst` |
| **Auto-bloqueo** | IPs con `>= auto_block_threshold` violaciones se bloquean automáticamente |
| **Límites de conexión** | `max_connections_per_ip` + `max_connections_total` |

El firewall opera **antes** de que cualquier conexión llegue al event loop de JavaScript. Las conexiones rechazadas son descartadas en Rust sin coste para el código JS.

Tipos públicos: `Firewall`, `FirewallConfig`, `FirewallDecision`, `BlockReason`.  
Función de utilidad: `spawn_cleanup_task(firewall, interval)` — limpia blocklist y buckets cada N segundos (lanzada automáticamente al primer `__httpListen`).

#### `remoteAddress` en todas las peticiones HTTP

`req.socket.remoteAddress` ahora está siempre disponible en los handlers de `http.createServer()`. Se resuelve desde el peer address del socket TCP en el momento del `accept`, independientemente de si el firewall está activo.

#### ESM→CJS transparente en entry points

Los archivos de entrada con `import`/`export` de ES modules ahora se transpilan automáticamente a CommonJS antes de ejecutarse. El desarrollador puede escribir:

```typescript
// src/index.ts
import { AppRegistry } from 'react-native';
AppRegistry.registerComponent('App', () => App);
```

Y ejecutar `3va run src/index.ts` sin ninguna configuración adicional. Internamente OXC aplica `Module::CommonJS` transform, convirtiendo los `import` en llamadas `require()` que tienen acceso completo a los polyfills del runtime.

#### `FirewallConfig` en `3va.config.ts`

Nueva sección `firewall` en el schema de configuración del proyecto:

```typescript
export default {
  firewall: {
    enabled: true,
    rateLimitRps: 100,
    rateLimitBurst: 200,
    headerTimeoutMs: 10_000,
    bodyTimeoutMs: 30_000,
    // ... ver docs/10-security/08-firewall.md
  }
}
```

Todos los campos tienen valores predeterminados seguros. Añadir `firewall: {}` al config es suficiente para activar todas las protecciones con los valores recomendados.

---

### Cambios en APIs existentes

- `JsEngine::new_with_firewall(permissions, firewall)` — nuevo constructor público.
- `JsEngine::new_with_firewall_and_inspector(permissions, firewall, inspect_addr)` — nuevo constructor.
- `inject_http_server` ahora acepta `Option<Arc<Firewall>>` como tercer argumento.
- `inject_all` ahora acepta `firewall: Option<Arc<Firewall>>` como cuarto argumento.

---

### Tests añadidos

**`vvva_firewall` unit tests** (15 total, 8 nuevos):

- `check_connection_allows_fresh_ip`
- `disabled_firewall_allows_everything`
- `decision_http_status_codes`
- `decision_messages`
- `auto_block_reason_is_rate_limit_violation`
- `block_remaining_ms_is_positive`
- `connection_count_stays_consistent_after_disconnect`
- `disconnect_below_zero_does_not_panic`

**`vvva_js` integration tests** (12 total, 5 nuevos):

- `request_exposes_remote_address`
- `firewall_rate_limits_after_burst_exhausted`
- `firewall_auto_blocks_after_threshold`
- `firewall_rejects_header_flood_and_continues`
- `firewall_slowloris_timeout_and_recovery`

---

### Documentación

- **Nueva**: [`docs/10-security/08-firewall.md`](10-security/08-firewall.md) — referencia completa del firewall.
- **Actualizada**: [`docs/10-security/07-http-performance.md`](10-security/07-http-performance.md) — sección "v2 roadmap" reemplazada por descripción de la implementación real.
- **Actualizada**: [`docs/13-v2/05-config-file.md`](../docs/13-v2/05-config-file.md) — schema de `firewall` añadido al ejemplo de `3va.config.ts`.
- **Actualizado**: [`docs/00-indice-general.md`](00-indice-general.md) — entrada `08-firewall.md` añadida al Volumen 10.

---

### Archivos modificados

| Archivo | Cambio |
|---------|--------|
| `crates/firewall/` | **Nuevo crate** `vvva_firewall` |
| `crates/js/src/builtins/http_server.rs` | Reescritura completa: timeouts, límites, integración firewall, remoteAddress |
| `crates/js/src/builtins/mod.rs` | `inject_all` acepta `Option<Arc<Firewall>>` |
| `crates/js/src/lib.rs` | Nuevos constructores `new_with_firewall*`; ESM→CJS en `eval_file` |
| `crates/js/src/transpiler.rs` | Nueva fn `transpile_to_cjs`; refactor `try_transpile_inner` |
| `crates/js/src/esm.rs` | Fix: `.tsx` usaba `transpile()` en vez de `transpile_jsx()` |
| `crates/js/Cargo.toml` | `vvva_firewall` como dependencia y dev-dependencia |
| `crates/cli/src/main.rs` | Carga `FirewallConfig` del proyecto y crea `Firewall` en el comando `run` |
| `crates/cli/Cargo.toml` | `vvva_firewall` como dependencia |
| `crates/config/src/schema.rs` | `FirewallConfig` añadido a `ProjectConfig` |
| `crates/js/tests/http_server.rs` | 5 nuevos tests de integración del firewall |
