# 08 — FIREWALL INTERNO HTTP

## 8.1 Visión General

`vvva_firewall` es el crate de seguridad de red de 3va. Protege el servidor HTTP integrado contra ataques volumétricos y de agotamiento de recursos sin requerir configuración externa. El firewall opera **en el bucle de aceptación de conexiones** (antes de que cualquier byte llegue al código JavaScript), lo que garantiza que el runtime nunca procese solicitudes que superan los límites de seguridad.

### Arquitectura de capas

```
Internet
    │
    ▼
TCP accept (Tokio)
    │
    ├── check_connection(ip)  ←── blocklist + límites de conexiones
    │       │
    │   [Allow] ──► on_connect(ip)
    │   [Deny]  ──► reject_stream(503/403) → continue (siguiente conexión)
    │
    ▼
parse_request(stream, timeouts, header_limits)
    │
    ├── Timeout de cabeceras  ←── Slowloris
    ├── Timeout de cuerpo     ←── RUDY
    ├── max_header_count      ←── Header flood
    └── max_header_bytes      ←── Header size bomb
            │
         [Error] ──► on_disconnect(ip) → continue
         [OK]    ──► check_request(ip)
                         │
                     [Allow]        ──► retornar a JS con remoteAddress
                     [RateLimited]  ──► 429 → on_disconnect → continue
                     [Blocked]      ──► 403 → on_disconnect → continue
```

### Principio de operación

El bucle de aceptación en `__httpAcceptAsync` es un `loop {}` de Rust. Las conexiones rechazadas (firewall, timeout, flood) no emergen a JavaScript — el loop descarta la conexión y acepta la siguiente sin retornar al event loop de V8. Esto evita que el código JS tenga que manejar errores de infraestructura y mantiene el servidor respondiendo incluso bajo ataque.

---

## 8.2 Ataques Mitigados

| Ataque | Descripción | Mecanismo de defensa |
|--------|-------------|---------------------|
| **Slowloris** | Abre conexiones enviando cabeceras muy lentamente, una línea por segundo, agotando los slots de conexión | `header_timeout_ms`: cada `read_line` tiene un deadline independiente |
| **RUDY** (R-U-Dead-Yet) | Envía cuerpos POST extremadamente despacio para mantener conexiones abiertas | `body_timeout_ms`: `read_exact` del cuerpo tiene su propio deadline |
| **Header flood** | Envía cientos de cabeceras para agotar memoria y CPU | `max_header_count` + `max_header_bytes` |
| **DDoS por tasa** | IP individual dispara miles de requests por segundo | Token bucket per-IP con `rate_limit_rps` / `rate_limit_burst` |
| **Agotamiento de conexiones** | Abre miles de conexiones sin enviar datos | `max_connections_per_ip` + `max_connections_total` |
| **IPs persistentes** | IP que ya ha sido identificada como maliciosa reintenta | Blocklist con TTL configurable (`block_duration_secs`) |

---

## 8.3 Componentes del Crate

### `FirewallConfig`

Todos los campos tienen valores predeterminados seguros. Basta con `FirewallConfig::default()` para activar protecciones básicas.

```rust
pub struct FirewallConfig {
    pub enabled: bool,                // true por defecto
    pub rate_limit_rps: u32,          // 100 req/s por IP
    pub rate_limit_burst: u32,        // burst de 200 req antes de throttle
    pub auto_block_threshold: u32,    // bloqueo auto tras 10 violaciones
    pub block_duration_secs: u64,     // IPs bloqueadas 300 s (5 min)
    pub max_connections_per_ip: u32,  // 50 conexiones simultáneas por IP
    pub max_connections_total: u32,   // 10,000 conexiones totales
    pub header_timeout_ms: u64,       // 10 s para recibir cabeceras completas
    pub body_timeout_ms: u64,         // 30 s para recibir el cuerpo
    pub max_header_count: u32,        // máx 100 cabeceras por request
    pub max_header_bytes: u32,        // máx 16 KiB de cabeceras combinadas
    pub max_body_bytes: u32,          // 0 = usar límite interno de 100 MiB
}
```

### `TokenBucket`

Algoritmo de token bucket por IP. Cada IP tiene su propio bucket que se rellena a razón de `rate_limit_rps` tokens por segundo. El `burst` es la capacidad máxima del bucket — permite ráfagas legítimas.

```
Tokens disponibles (inicia en `burst`)
         ↑
         │  se rellenan a `rps` tokens/segundo
         │
consume() → ¿tokens >= 1?
    Sí → tokens -= 1, request permitido
    No → violations++, request denegado
              ↓
    violations >= auto_block_threshold?
        Sí → block_ip(ip, block_duration, RateLimitViolation)
```

Los tokens se rellenan *lazily* al llamar `consume()` basándose en el tiempo transcurrido (`Instant::elapsed()`). No hay hilo de fondo para el bucket — se calcula en el momento del check.

### `FirewallDecision`

```rust
pub enum FirewallDecision {
    Allow,
    RateLimited { retry_after_ms: u64 },    // HTTP 429
    Blocked { reason: BlockReason, remaining_ms: u64 }, // HTTP 403
    ConnectionLimitReached,                  // HTTP 503
}
```

### `Firewall`

La estructura principal. Thread-safe vía `Mutex<HashMap<...>>`. Diseñada para compartirse como `Arc<Firewall>` entre el `JsEngine` y el servidor HTTP.

```rust
let fw = Firewall::new(FirewallConfig::default());
let engine = JsEngine::new_with_firewall(permissions, fw).await?;
```

---

## 8.4 Configuración en `3va.config.ts`

```typescript
export default {
  firewall: {
    // Activar/desactivar el firewall completo
    enabled: true,

    // Token bucket: tasa sostenida máxima de requests por IP
    rateLimitRps: 100,

    // Capacidad de ráfaga antes de que se active el rate limiting
    rateLimitBurst: 200,

    // Número de violaciones antes de bloquear la IP automáticamente
    autoBlockThreshold: 10,

    // Duración del bloqueo en segundos (300 = 5 minutos)
    blockDurationSecs: 300,

    // Conexiones simultáneas máximas por IP
    maxConnectionsPerIp: 50,

    // Conexiones simultáneas totales (todas las IPs)
    maxConnectionsTotal: 10_000,

    // Tiempo máximo para recibir la línea de petición + cabeceras (ms)
    // Protege contra Slowloris
    headerTimeoutMs: 10_000,

    // Tiempo máximo para recibir el cuerpo completo tras las cabeceras (ms)
    // Protege contra RUDY
    bodyTimeoutMs: 30_000,

    // Número máximo de cabeceras HTTP por petición
    maxHeaderCount: 100,

    // Tamaño máximo combinado de todas las cabeceras (bytes)
    maxHeaderBytes: 16_384,

    // Tamaño máximo del cuerpo (0 = límite interno de 100 MiB)
    maxBodyBytes: 0,
  }
}
```

### Perfiles de configuración recomendados

**API pública de alta disponibilidad** — tráfico masivo, necesita ráfagas amplias:
```typescript
firewall: {
  rateLimitRps: 500,
  rateLimitBurst: 1000,
  autoBlockThreshold: 20,
  maxConnectionsPerIp: 200,
  maxConnectionsTotal: 50_000,
}
```

**API interna o de uso empresarial** — tráfico controlado, seguridad estricta:
```typescript
firewall: {
  rateLimitRps: 50,
  rateLimitBurst: 100,
  autoBlockThreshold: 5,
  blockDurationSecs: 3_600,  // bloqueo 1 hora
  maxConnectionsPerIp: 20,
  headerTimeoutMs: 5_000,
  bodyTimeoutMs: 15_000,
}
```

**Desarrollo local** — sin restricciones de tasa:
```typescript
firewall: {
  enabled: false,
}
```

---

## 8.5 `remoteAddress` en los Requests

Cuando el firewall está activo, cada request de HTTP que llega a JavaScript incluye la IP del cliente en `req.socket.remoteAddress`:

```typescript
const http = require('http');

http.createServer((req, res) => {
  console.log('Petición de:', req.socket.remoteAddress);
  res.end('ok');
}).listen(3000);
```

Este campo se propaga independientemente del estado del firewall — se resuelve desde el peer address del socket TCP en el momento del `accept`.

---

## 8.6 Tarea de Limpieza en Background

El crate expone `spawn_cleanup_task` para evitar crecimiento ilimitado de memoria en el blocklist y los buckets de rate-limiting:

```rust
// Llamado automáticamente desde __httpListen al crear el primer servidor.
vvva_firewall::spawn_cleanup_task(firewall.clone(), Duration::from_secs(60));
```

La tarea se lanza automáticamente cuando el primer servidor HTTP es creado (`__httpListen` con `id == 0`). Ejecuta `firewall.cleanup()` cada 60 segundos:

- **Blocklist**: elimina entradas cuyo `expires` ya pasó.
- **Token buckets**: elimina buckets de IPs que llevan más de 5 minutos sin actividad.

---

## 8.7 Integración con `vvva_permissions`

El firewall **no reemplaza** el sistema de permisos (`vvva_permissions`). Ambos operan en capas distintas:

| Sistema | Capa | Pregunta |
|---------|------|----------|
| `vvva_permissions` | Capacidades del proceso | ¿Puede este *proceso* escuchar en esta dirección? |
| `vvva_firewall` | Tráfico de red en tiempo real | ¿Puede esta *IP* hacer esta petición ahora? |

El permiso de red se comprueba en `__httpListen` (al crear el servidor). El firewall actúa en `__httpAcceptAsync` (por cada conexión entrante).

---

## 8.8 Tests

### Unit tests (`vvva_firewall`)

```
cargo test -p vvva_firewall
```

| Test | Qué verifica |
|------|-------------|
| `allow_within_burst` | Las primeras N peticiones (dentro del burst) son permitidas |
| `rate_limited_after_burst` | La petición N+1 devuelve `RateLimited` |
| `auto_block_after_threshold` | Tras `threshold` violaciones la IP queda bloqueada |
| `manual_block_and_unblock` | `block_ip` / `unblock_ip` funcionan correctamente |
| `connection_tracking` | `on_connect` / `on_disconnect` mantienen contadores correctos |
| `total_connection_cap` | Se rechaza la conexión N+1 cuando se alcanza `max_connections_total` |
| `cleanup_removes_expired_blocks` | `cleanup()` elimina entradas de blocklist expiradas |
| `check_connection_allows_fresh_ip` | Una IP nueva sin historial es permitida |
| `disabled_firewall_allows_everything` | `enabled: false` ignora blocklist y rate limits |
| `decision_http_status_codes` | `http_status()` devuelve 200 / 429 / 403 / 503 según la decisión |
| `decision_messages` | `message()` devuelve el texto HTTP correcto |
| `auto_block_reason_is_rate_limit_violation` | Auto-block usa `BlockReason::RateLimitViolation` |
| `block_remaining_ms_is_positive` | `remaining_ms` es > 0 inmediatamente tras el bloqueo |
| `connection_count_stays_consistent_after_disconnect` | El contador no queda en negativo tras disconnects |
| `disconnect_below_zero_does_not_panic` | `on_disconnect` sin `on_connect` no produce panic ni underflow |

### Integration tests (`vvva_js`)

```
cargo test -p vvva_js --test http_server
```

| Test | Qué verifica |
|------|-------------|
| `request_exposes_remote_address` | `req.socket.remoteAddress` contiene la IP del cliente |
| `firewall_rate_limits_after_burst_exhausted` | La 3ª petición rápida recibe HTTP 429 |
| `firewall_auto_blocks_after_threshold` | La 5ª petición recibe HTTP 403 (IP auto-bloqueada) |
| `firewall_rejects_header_flood_and_continues` | Header flood es descartado y el servidor sigue aceptando |
| `firewall_slowloris_timeout_and_recovery` | Conexión lenta es cerrada por timeout; el servidor responde normalmente a la siguiente |

---

## 8.9 Limitaciones Conocidas

- **No soporta IPv6 NAT64** — una IP IPv6 puede representar múltiples clientes reales en redes con traducción de direcciones. El rate-limit se aplica por dirección IP tal como aparece en el socket.
- **Sin persistencia** — el blocklist y los buckets viven en memoria. Un reinicio del proceso vacía todas las restricciones.
- **Sin modo de solo-observación** — `enabled: false` desactiva todas las protecciones. No existe un modo "log-only" en v2.0.0.
- **Sin integración con reverse proxy** — si 3va está detrás de nginx/Caddy, `remoteAddress` será la IP del proxy, no la del cliente final. Soporte para `X-Forwarded-For` es trabajo futuro.

---

*Implementación: `crates/firewall/src/lib.rs`*
*Integración HTTP: `crates/js/src/builtins/http_server.rs`*
*Schema de config: `crates/config/src/schema.rs` → `FirewallConfig`*
