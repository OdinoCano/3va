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
| **RUDY** (R-U-Dead-Yet) | Envía cuerpos POST extremadamente despacio para mantener conexiones abiertas | `body_timeout_ms`: deadline total de lectura del cuerpo + `min_body_rate_bps`: tasa mínima media de recepción (un cuerpo que llega a <50 B/s se aborta a los ~2 s, sin esperar al deadline) |
| **Header flood** | Envía cientos de cabeceras para agotar memoria y CPU | `max_header_count` + `max_header_bytes` |
| **DDoS por tasa** | IP individual dispara miles de requests por segundo | Token bucket per-IP con `rate_limit_rps` / `rate_limit_burst` |
| **Agotamiento de conexiones** | Abre miles de conexiones sin enviar datos | `max_connections_per_ip` + `max_connections_total` |
| **IPs persistentes** | IP que ya ha sido identificada como maliciosa reintenta | Blocklist adaptativo: cada auto-bloqueo suma un *strike*; la duración escala como `block_duration_secs × factor^(strikes-1)` hasta `max_block_duration_secs` |

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
    pub block_duration_secs: u64,     // bloqueo base: 300 s (5 min)
    pub block_escalation_factor: u32, // ×2 por reincidencia (adaptativo)
    pub max_block_duration_secs: u64, // tope: 3600 s (1 h)
    pub strike_decay_secs: u64,       // olvida el historial tras 3600 s de calma
    pub max_connections_per_ip: u32,  // 50 conexiones simultáneas por IP
    pub max_connections_total: u32,   // 10,000 conexiones totales
    pub header_timeout_ms: u64,       // 10 s para recibir cabeceras completas
    pub body_timeout_ms: u64,         // 30 s para recibir el cuerpo
    pub min_body_rate_bps: u32,       // 100 B/s mínimos de cuerpo (RUDY)
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

### Escalación adaptativa (strikes)

`auto_block_threshold` + `block_duration_secs` forman la capa base: violaciones repetidas → bloqueo automático con duración fija. Sobre eso, el firewall es **adaptativo**: cada auto-bloqueo registra un *strike* por IP y la duración del bloqueo escala con la reincidencia.

```
auto-block nº `n` → duración = min(block_duration_secs × factor^(n-1), max_block_duration_secs)
```

- Con los valores por defecto (base 300 s, factor ×2, tope 3600 s): 1ª ofensa → 300 s, 2ª → 600 s, 3ª → 1200 s, 4ª → 2400 s, 5ª → 3600 s (tope).
- El historial de strikes **no** se borra cuando expira el bloqueo: un reincidente que vuelve a atacar recibe el siguiente escalón. Solo se limpia cuando la IP lleva `strike_decay_secs` sin otro auto-bloqueo (por defecto 1 h de calma), momento en que la duración vuelve a `block_duration_secs`.
- La escalación penaliza el *castigo* (duración del bloqueo), no el bucket de tasa per-IP: los `rate_limit_rps`/`rate_limit_burst` de cada IP permanecen fijos.

### Rate limiting adaptativo (baseline EWMA)

La escalación anterior penaliza reincidencia; este modo adaptativo resuelve el problema inverso: **picos legítimos**. Con umbrales fijos, una IP cuyo tráfico legítimo crece gradualmente (un cliente pesado, un proxy corporativo) cruza `rate_limit_rps` y empieza a acumular violaciones aunque no haya nada malicioso. Con `adaptive_rate_limit: true`, cada request observado alimenta un baseline por IP y el umbral efectivo sube con él.

```
Ventana de observación: 1 s (ADAPTIVE_WINDOW_SECS)
Baseline:  ewma = α × count_ventana + (1 − α) × ewma_anterior
           α = ewma_alpha_pct / 100          (default 20 → α = 0.20)
Umbral:    rps_efectivo = max(rate_limit_rps, ceil(ewma × ADAPTIVE_HEADROOM))
           con tope rps_efectivo ≤ rate_limit_rps × ADAPTIVE_MAX_RATE_MULTIPLIER
           (ADAPTIVE_HEADROOM = 1.5, ADAPTIVE_MAX_RATE_MULTIPLIER = 4.0 — constantes en crates/firewall/src/lib.rs)
```

- El bucket consume contra `rps_efectivo` (la tasa de refill del token bucket se ajusta en cada check), así que la IP con baseline alto obtiene más tokens/segundo sin tocar su burst.
- Si la IP vuelve a tráfico bajo, el EWMA decae hacia abajo en las ventanas siguientes y el umbral converge de vuelta a `rate_limit_rps`.
- Un atacante que arranca desde cero sigue limitado por el umbral estático: sin historial observado, `ewma = 0` → umbral = estático.
- Knobs de configuración (`FirewallConfig`): `adaptive_rate_limit: bool` (default `false`) y `ewma_alpha_pct: u32` 0–100 (default `20`; mayor = se adapta más rápido, menor = suaviza más).

Verificado por los tests `ewma_update_tracks_samples_with_configurable_smoothing`, `effective_rps_rises_with_baseline_and_stays_capped` y `growing_legitimate_traffic_raises_limit_without_violations` en `crates/firewall/src/lib.rs`.

### RUDY: `min_body_rate_bps`

Además del deadline total (`body_timeout_ms`), el lector del cuerpo calcula la tasa media de recepción pasados 2 s de gracia: si `bytes_recibidos / tiempo < min_body_rate_bps`, la conexión se cierra inmediatamente. Esto neutraliza el RUDY real (1 byte/s) sin esperar a que expire el deadline de 30 s — el caso límite que el timeout por sí solo cubre mal.

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

    // Proxies de confianza para X-Forwarded-For (IPs o CIDRs; vacío = ignorar header)
    trustedProxies: [],

    // Número de violaciones antes de bloquear la IP automáticamente
    autoBlockThreshold: 10,

    // Duración base del bloqueo en segundos (300 = 5 minutos)
    blockDurationSecs: 300,

    // Multiplicador de la duración por reincidencia (2 = se duplica cada ofensa)
    blockEscalationFactor: 2,

    // Tope de la duración escalada (3600 = 1 hora)
    maxBlockDurationSecs: 3600,

    // Segundos de calma tras los que se olvida el historial de strikes
    strikeDecaySecs: 3600,

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

    // Tasa mínima de recepción del cuerpo en B/s (RUDY). 0 = desactivada.
    minBodyRateBps: 100,

    // Rate limiting adaptativo: sube el umbral por IP según su tráfico
    // legítimo observado (EWMA); ver §"Rate limiting adaptativo"
    adaptiveRateLimit: false,

    // Factor de suavizado del EWMA en % (0–100; mayor = adapta más rápido)
    ewmaAlphaPct: 20,

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

### Detrás de un reverse proxy (`trustedProxies` + `X-Forwarded-For`)

Si 3va corre detrás de nginx/Caddy (o cualquier proxy de confianza), el peer address es la IP del proxy, no la del cliente. Con `trustedProxies` configurado, cuando la conexión llega **directamente** desde un proxy de confianza, `remoteAddress` reporta la IP real extraída de `X-Forwarded-For` y el rate-limiting contabiliza contra esa IP:

```typescript
export default {
  firewall: {
    // IPs o CIDRs autorizados a setear X-Forwarded-For
    trustedProxies: ["127.0.0.1", "10.0.0.0/8"],
  },
};
```

Reglas de resolución (implementadas en `resolve_forwarded_for`, `crates/firewall/src/lib.rs`):

- **Peer no confiable → header ignorado.** Un cliente que conecta directo y manda su propio `X-Forwarded-For` no puede falsificar `remoteAddress` (anti-spoofing). Solo se mira el header si el peer inmediato matchea `trustedProxies`.
- Se recorre la lista **de derecha a izquierda**, saltando los hops que son proxies de confianza; la primera dirección no-confiable es el cliente.
- Una entrada malformada corta el recorrido.
- Si todas las entradas son proxies de confianza, gana la más a la izquierda (el hop más profundo de la cadena).
- Sin header o sin resolución → se usa el peer address tal cual.

Verificado por los tests `xff_ignored_when_peer_not_trusted`, `xff_resolves_client_behind_trusted_proxy`, `firewall_client_ip_uses_xff_only_through_trusted_proxies` (`crates/firewall`) y los e2e `trusted_proxy_forwards_client_ip_to_remote_address`, `untrusted_xff_header_is_ignored` (`crates/js/tests/http_server.rs`).

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
| `adaptive_block_duration_escalates_and_caps` | La duración escala 10→20→40→80 s y nunca supera el tope |
| `adaptive_escalation_factor_one_disables_escalation` | `factor: 1` = duración fija |
| `adaptive_max_below_base_never_shortens_first_block` | Un tope mal configurado < base no acorta el 1er bloqueo |
| `adaptive_auto_block_escalates_across_offenses` | La 2ª ofensa devuelve `remaining_ms` ×3 y suma un strike |
| `adaptive_strikes_persist_within_decay_window` | `cleanup()` no borra el historial dentro de la ventana |
| `adaptive_strikes_clear_after_decay_window` | `cleanup()` resetea el historial tras `strike_decay_secs` |

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
| `firewall_rudy_slow_body_rejected_and_recovers` | Atacante real (cuerpo a ~1 B/s): la conexión se cierra en ~2 s por `min_body_rate_bps`, el body nunca llega a JS y el servidor se recupera |
| `firewall_adaptive_escalation_repeat_offender` | Reincidente: 1ª bloqueo 1 s, tras reincidir el 2ª bloqueo dura 2 s (escalado) y solo se sirve de nuevo al expirar |

> Los tests de ataque simulado (RUDY y escalación adaptativa) ejercitan el servidor HTTP real con un cliente TCP de verdad — no son unit tests de la función aislada. El RUDY usa un deadline de cuerpo de 120 s para demostrar que es la *tasa mínima* quien rechaza, no el timeout total.

---

## 8.9 Limitaciones Conocidas

- **No soporta IPv6 NAT64** — una IP IPv6 puede representar múltiples clientes reales en redes con traducción de direcciones. El rate-limit se aplica por dirección IP tal como aparece en el socket.
- **Sin persistencia** — el blocklist, los buckets y los strikes viven en memoria. Un reinicio del proceso vacía todas las restricciones.
- **Sin modo de solo-observación** — `enabled: false` desactiva todas las protecciones. No existe un modo "log-only" en v2.0.0.
- **Sin integración con reverse proxy** — resuelto: `trustedProxies` + `X-Forwarded-For` soportados, ver §8.5 "Detrás de un reverse proxy". Queda pendiente el protocolo `Forwarded` (RFC 7239) y proxies en múltiples hops no listados.
- **La adaptividad ajusta el castigo, no la tasa** — los `rate_limit_rps`/`rate_limit_burst` de cada IP son fijos. La escalación (strikes) solo alarga los bloqueos de reincidentes; el modo `adaptiveRateLimit` sí ajusta el umbral de tasa por IP según su baseline EWMA (ver §"Rate limiting adaptativo"), pero no aprende patrones más complejos.
- **El historial de strikes se olvida tras `strike_decay_secs`** — un atacante que espera la ventana completa de calma (1 h por defecto) vuelve a empezar con la duración base. No hay memoria de ofensas antiguas más allá de esa ventana.

---

*Implementación: `crates/firewall/src/lib.rs`*
*Integración HTTP: `crates/js/src/builtins/http_server.rs`*
*Schema de config: `crates/config/src/schema.rs` → `FirewallConfig`*
