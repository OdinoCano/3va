# 07 - Inventario de métodos falsos (stubs)

Métodos que existen pero no hacen lo que dicen. Casi todos son
`function() { return this; }` en `crates/js/src/builtins/`: el test "¿existe el
método?" los da por buenos, pero una app que los usa de verdad no nota nada — o
nota algo distinto que en Node. Es el paso 3 de la hoja de ruta de
[`docs/09-testing/07-compatibilidad-node.md`](../09-testing/07-compatibilidad-node.md)
(§7.4).

Medido con `rg 'function\(\) \{ return this; \}' crates/js/src/builtins/`:
**53 coincidencias**, de las cuales 46 son no-ops reales (las otras 7 son
correctas, ver §7.5). Este documento es el inventario y la decisión; el código
lleva el comentario `// ponytail:` correspondiente cuando es un no-op
intencionado.

Decisiones posibles, por orden de preferencia:

1. **Implementar** si lo usan librerías comunes (comprobado con la suite
   diferencial y `scripts/library-suites.sh`).
2. **Lanzar** un `Error` con `code: 'ERR_NOT_SUPPORTED'` y un mensaje que diga
   qué falta.
3. **No-op intencionado**, documentado con su motivo y con un comentario
   `// ponytail:` en el código.

Convertir un no-op silencioso en un error rompe apps que hoy funcionan, así que
los cambios a "lanzar" exigen pasar la suite diferencial, `scripts/compat-check.sh`
y las suites de librerías antes de aterrizar.

## 7.1 `dgram.Socket` — opciones de socket

`crates/js/src/builtins/dgram.rs:486-493`.

| API | Qué hace Node | Qué hacía 3va | Decisión |
|---|---|---|---|
| `setBroadcast(flag)` | `setsockopt(SO_BROADCAST)`; lanza si falla | `return this` | No-op documentado (falta `setsockopt` sobre el fd del socket UDP; lo usan apps de descubrimiento) |
| `setTTL(ttl)` | `setsockopt(IP_TTL)`; valida 1..255 | `return this` | No-op documentado |
| `setMulticastTTL(ttl)` | `setsockopt(IP_MULTICAST_TTL)` | `return this` | No-op documentado |
| `setMulticastLoopback(flag)` | `setsockopt(IP_MULTICAST_LOOP)` | `return this` | No-op documentado |
| `addMembership(group, iface)` | `setsockopt(IP_ADD_MEMBERSHIP)` | `return this` | No-op documentado (mDNS/SSDP no funcionan) |
| `dropMembership(group, iface)` | `setsockopt(IP_DROP_MEMBERSHIP)` | `return this` | No-op documentado |
| `ref()` / `unref()` | Marcan el handle frente al event loop | `return this` | No-op documentado (el bucle de 3va no libera el proceso por un socket) |

## 7.2 Sockets TCP (`net`, `http`, `tls`, `http2`)

| API | Qué hace Node | Qué hacía 3va | Decisión |
|---|---|---|---|
| `net.Socket.setKeepAlive(enable, delay)` (`modules.rs:3815`, `3970`) | `setsockopt(SO_KEEPALIVE)` | `return this` | No-op documentado (pista de rendimiento, no cambia la semántica) |
| `net.Socket.setNoDelay(enable)` (`3965`, `ClientRequest.setNoDelay` `4893`) | `setsockopt(TCP_NODELAY)` | `return this` | No-op documentado |
| `ClientRequest.setSocketKeepAlive(...)` (`4894`) | Igual, sobre el socket de la petición | `return this` | No-op documentado |
| `Socket.ref()` / `unref()` (`3818-3819`), `Server.ref/unref` (`4050-4051`), `UpgradeSocket.ref/unref` (`3974-3975`) | Referencian/desreferencian el handle | `return this` | No-op documentado |
| `http.Server.ref/unref` (`4725-4726`), `http2.Http2Server` (`1485-1486`), `Http2ClientSession` (`1561-1562`) | Igual | `return this` | No-op documentado |

## 7.3 Timeouts y destrucción de mensajes HTTP

| API | Qué hace Node | Qué hacía 3va | Decisión |
|---|---|---|---|
| `IncomingMessage.setTimeout(ms, cb)` (`modules.rs:4412`) | Arma un timeout sobre el socket | `return this` | No-op documentado (el timeout efectivo lo aplica el firewall/servidor) |
| `OutgoingMessage.setTimeout` (`4556`), `ClientRequest.setTimeout` (`4688`, que sí registra el callback) | Idem | `return this` | No-op documentado para `4556`; el de `ClientRequest` registra el callback |
| `ServerResponse.setTimeout(ms, cb)` (`4556`) y `http.Server.setTimeout` (`4636`) | Idem | `return this` | No-op documentado |
| `ServerResponse.destroy()` (`4557`) | Destruye la respuesta y cierra el socket | `return this` | **Pendiente de implementar** (hoy un `destroy()` no cierra nada) |
| `ServerResponse.addTrailers(headers)` | Añade una sección de trailers al final del cuerpo | No existía | No-op documentado: la respuesta se escribe entera de una vez (con `Content-Length`), así que no hay sección de trailers donde ponerlos. Los valores quedan en `res._trailers` |
| `ServerResponse.flushHeaders()` | Escribe la cabeza sin el cuerpo | No existía | No-op documentado: la cabeza y el cuerpo se escriben juntos en `end()` (no hay escritura parcial de la respuesta) |

## 7.4 Streams

| API | Qué hace Node | Qué hacía 3va | Decisión |
|---|---|---|---|
| `Writable.setDefaultEncoding(enc)` (`modules.rs:2538`) | Fija la codificación de `write(string)` | `return this` | **Pendiente de implementar** (una sola asignación; puede cambiar el comportamiento) |
| `zlib` stream `pause()` / `resume()` (`zlib.rs:778-779`), `setEncoding` (`784`) | Control de flujo / decodificación | `return this` | No-op documentado |
| `child_process` stdio `resume/pause/removeListener/removeAllListeners` (`child_process.rs:1510-1571`) | Control de flujo del stream | `return this` | No-op documentado (objeto de stream sintético) |

## 7.5 Correctas (no son stubs)

No son "métodos falsos" y no cambian:

- `web_globals.rs:479,484,489`: `[Symbol.iterator]: function() { return this; }`
  en un objeto iterador — es el protocolo correcto.
- `child_process.rs` (`removeListener`/`removeAllListeners`): devolver `this` es
  correcto cuando no hay listeners que quitar.
- `AsyncResource.emitDestroy` (`modules.rs:5620`): en 3va no hay hooks `destroy`
  que disparar; el valor de retorno es un no-op documentado.
- `diagnostics_channel` `enable/disable` (`modules.rs:5645`) y `ssh` `on/stdin.write`
  (`ssh.rs:1233`): objetos auxiliares sin estado que gestionar; no-op documentado.

## 7.6 Qué queda

- Implementar `ServerResponse.destroy()` y `Writable.setDefaultEncoding()`.
- Evaluar `dgram` (setsockopt) y `IncomingMessage`/`Server` timeouts si una
  librería real los necesita — con la suite diferencial y `library-suites.sh`
  como juez, no por completitud.
- El resto son no-ops intencionados con su `// ponytail:` en el código.
