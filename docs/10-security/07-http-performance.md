# 07 - HTTP PERFORMANCE AND CONNECTION PROTECTION

## 7.1 Overview

3va's HTTP server is built on top of the V8 event loop and Tokio. At high concurrency, the server applies connection-level rate limiting to protect the process from resource exhaustion. This document covers baseline benchmarks, the rate-limiting strategy, and the DoS protections built into the HTTP layer.

---

## 7.2 Baseline Benchmarks

Measured against a minimal `Hello, World` HTTP server. 100,000 requests at 1,000 concurrent connections:

| Runtime | Req/s | P50 | P99 | Success rate |
|---------|-------|-----|-----|-------------|
| Bun 1.3 | 20,758 | 4.4 ms | 16.0 ms | 100% |
| **3va 1.0** (debug build) | **1,572** | **61 ms** | **143 ms** | **100%** |

> The debug build penalty is significant. Release builds (`cargo build --release`) reduce latency by 3–5×. The throughput gap vs. Bun narrows considerably in release mode.

### Stress test: 2,000 concurrent connections, 1,000,000 requests

| Runtime | Success rate | Req/s | Notes |
|---------|-------------|-------|-------|
| Bun 1.3 | 100% | 21,650 | No connection limiting |
| Node.js 25 | 99.97% | 8,869 | 281 connection errors |
| **3va 1.0** (debug) | **70.4%** | **327** | Rate-limited by design |

At 2,000 concurrent connections, 3va drops excess connections rather than queuing them indefinitely. The 70.4% success rate reflects the rate limiter shedding load — not crashes, memory exhaustion, or errors in accepted connections.

---

## 7.3 Connection Rate Limiting

### Design intent

3va deliberately limits the number of simultaneously active HTTP connections. When the limit is reached, new incoming connections are dropped immediately at the accept loop rather than queued. This is a **fail-fast** strategy: callers get a TCP RST quickly instead of waiting for an overloaded server that may never respond.

### Behavior under load

```
Incoming connection
         │
         ▼
active_connections < MAX_ACTIVE?
         │
    ┌────┴────┐
   Yes        No
    │          │
    ▼          ▼
  Accept     Drop (TCP RST)
  + handle   + log (if --audit-level=all)
```

Dropped connections are not retried server-side. Clients that implement retry-with-backoff will recover automatically once load decreases.

### Production recommendation

For production deployments that need higher concurrency, place 3va behind a reverse proxy (nginx, Caddy, Envoy) that handles connection queuing and back-pressure externally. 3va's rate limiting then applies only to the connections the proxy forwards.

---

## 7.4 Slowloris Protection

A Slowloris attack keeps many connections open by sending HTTP request headers very slowly, exhausting the server's connection slots without sending complete requests.

3va's HTTP layer protects against this at the accept loop level: connections that do not deliver a complete HTTP request within the header read timeout are closed. This prevents slow senders from occupying slots indefinitely.

```
Connection accepted
         │
         ▼
Read headers (with timeout)
         │
    ┌────┴─────────┐
  Complete       Timeout
    │              │
    ▼              ▼
  Process        Close connection
  request        (Slowloris shed)
```

---

## 7.5 v2.0.0: Firewall Interno (`vvva_firewall`)

La v2.0.0 implementa el firewall completo como un crate separado. Sustituye el limitador de conexiones fijo anterior.

### Rate limiting adaptativo por IP

El token bucket per-IP (`rate_limit_rps` / `rate_limit_burst`) throttlea a cada origen de forma independiente. Las IPs legítimas no se ven afectadas por el abuso de otras. Tras `auto_block_threshold` violaciones, la IP se añade al blocklist automáticamente.

### Detección de RUDY

El cuerpo de la petición ahora está protegido por `body_timeout_ms`. Un cliente que envía el cuerpo byte a byte no puede bloquear un slot de conexión indefinidamente — la lectura de `read_exact` se cancela con timeout.

### Límites de cabeceras

`max_header_count` y `max_header_bytes` frenan los ataques de header flood antes de que el cuerpo sea leído.

> Documentación completa: [`08-firewall.md`](08-firewall.md)

---

## 7.6 Interpreting Benchmark Results

When running your own benchmarks against 3va, keep in mind:

- **Debug vs. release builds**: `./target/debug/3va` includes bounds checks, no inlining, and debug symbols. Always benchmark `./target/release/3va` for production comparisons.
- **Rate limiter interference**: tools like `wrk`, `hey`, or `k6` at high concurrency will hit the connection limit. Lower the concurrency level (`-c` flag) or raise the server limit if you want to measure raw throughput rather than limiting behavior.
- **Event loop contention**: the JS event loop and HTTP accept loop share the same Tokio runtime. CPU-bound JS code in request handlers reduces HTTP throughput directly.

---

*HTTP server implemented in `crates/js/src/builtins/http_server.rs`.*
*Rate limiting and connection management in the `__httpAcceptAsync` built-in.*
