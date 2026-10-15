# 06 — CLIENTE `http`/`https`, `fetch`, `net` y `stream`: matriz Node vs 3va

Este documento describe los fallos del **lado cliente** (lo que una app usa
para llamar a APIs externas) que se corrigieron en **2.12.6**, con la matriz de
verificación Node 24 vs 3va y lo que queda pendiente.

El lado **servidor** (cuerpo de request binario y en streaming) se corrigió en
2.12.5 (PR #64). Al revisar ese PR se comprobó que el lado cliente, presente ya
en 2.12.3, tenía fallos propios y que uno de ellos afectaba a todo lo que use
`http.request`: axios, got, los SDK de AWS/Stripe, etc.

La regla de verificación es siempre la misma: **servidor en un proceso Node y
cliente en 3va en otro proceso** (en el mismo proceso hay comportamientos que no
se dan en producción). Un test de regresión en `crates/js/tests/client_http_matrix.rs`
levanta un servidor real (hilo/tokio) y ejerce el cliente de 3va contra él.

## Los 7 fallos reproducidos

Servidor eco (Node, proceso aparte) que responde el hex del cuerpo recibido.

| Caso (cliente en 3va) | Node 24 | 3va 2.12.3–2.12.5 | 2.12.6 |
|---|---|---|---|
| `http.request` POST + `req.end("hi")` | `6869` | cuerpo vacío | `6869` |
| `http.request` POST con `Buffer`/`Uint8Array` binario | `00ff6869c8` | cuerpo vacío | `00ff6869c8` |
| `fetch(url, { body: Buffer.from([0,255,104,105,200]) })` | `00ff6869c8` | `00efbfbd6869efbfbd` | `00ff6869c8` |
| `fetch(..., { body: new Uint8Array([...]) })` | `00ff6869c8` | `"0,255,104,105,200"` | `00ff6869c8` |
| `net.connect` → `c.end(data)` y el servidor responde tras su `'end'` | recibe respuesta | `TIMEOUT` / respuesta perdida | recibe respuesta |
| `net`: `write("hi")` y `end()` 100 ms después | `6869` | `close` sin datos | `6869` |
| `new PassThrough()`; `.on('data')`/`.on('end')`; `.end(chunk)` | `data` + `end` | nada; salida 0 | `data` + `end` |

### Causas

- **Cliente `http` sin framing de cuerpo.** `ClientRequest` escribía la línea de
  request y las cabeceras del usuario, pero nunca añadía `Content-Length` ni
  `Transfer-Encoding: chunked`. Un servidor Node, al no ver framing, lee **cero**
  bytes de cuerpo: los bytes que el cliente escribía quedaban fuera del request.
  Tampoco enviaba el cuerpo escrito con `write()` después del `'connect'`
  (streaming/`pipe`), ni de-framaba la respuesta (`chunked`, `Content-Length`,
  read-to-EOF), y `flushHeaders()`/`abort()`/`timeout`/`agent` keep-alive eran
  inertes.
- **`fetch` serializaba el cuerpo como string.** `String(options.body)` convertía
  un `Buffer` en su texto UTF-8 con pérdida y un `Uint8Array` en
  `"0,255,104,105,200"`; el nativo además recibía siempre un string y usaba
  `send_string`.
- **`net` no hacía half-close.** `Socket.end()` destruía la conexión
  (`__tcpClose` con `shutdown(Both)`) en lugar de cerrar sólo el lado de
  escritura, así que la respuesta que el servidor enviaba tras el FIN del
  cliente nunca se leía.
- **`end(chunk)` marcaba el stream como terminado antes de escribir.** En
  `Writable`/`Transform`, `end(chunk)` hacía `state.ended = true` y luego
  `this.write(chunk)`, que rechazaba el chunk con `write after end`: se perdían
  el chunk y el flush final (`PassThrough.end(buf)`, `socket.end(data)`).
- **`'drain'` se emitía de forma síncrona** dentro de `write()`, antes de que el
  llamante pudiera adjuntar su listener, con lo que la espera por `'drain'` se
  quedaba colgada (no sólo en `pipeline`).

## Matriz ampliada (cliente en 3va, servidor Node en otro proceso)

| Caso | Node 24 | 3va 2.12.6 |
|---|---|---|
| `http` GET/POST/PUT/PATCH/DELETE (con y sin cuerpo) | ok | ok |
| `http` cuerpo string, `Buffer`, `Uint8Array`, vacío | ok | ok |
| `http` varios `write()` + `end()` | `616200ff6364` | `616200ff6364` |
| `http` `pipe(fs.createReadStream(1 MB))` + `Content-Length` (SHA-256 en servidor) | ok | ok |
| `http` streaming sin `Content-Length` → chunked | ok | ok |
| `http` `setHeader` / `flushHeaders` | `68656c6c6f` | `68656c6c6f` |
| `http` respuesta binaria | `00ff6869c8` | `00ff6869c8` |
| `http` respuesta chunked (100 000 B) de-framada | ok | ok |
| `http` `abort()` (emite `'close'`, proceso sigue vivo) | ok | ok |
| `http` `agent` keep-alive: 5 peticiones, 4 reutilizan conexión | reused 4 | reused 4 |
| `http` `pipe` de 200 MB con RSS acotado | pendiente de medición formal | ver *Pendientes* |
| `fetch` `body` string / `Buffer` / `Uint8Array` / `ArrayBuffer` / `DataView` | ok | ok |
| `fetch` `body` `Blob` | ok | ok |
| `fetch` `body` `URLSearchParams` (content-type automático) | ok | ok |
| `fetch` `body` `FormData` (multipart con archivo binario) | ok | ok |
| `fetch` `body` `ReadableStream` (`duplex: 'half'`) | ok | **pendiente** |
| `net` half-close `allowHalfOpen` true/false, `end(data)` | ok | ok |
| `net` `setEncoding('utf8')` | ok | ok |
| `net` backpressure: `write()` false + `'drain'` | ok (ver nota) | false + `'drain'` |
| `net` `pipe` de archivo 1 MB (SHA-256 en servidor) | ok | ok |
| `net` ECONNREFUSED con `code` | `ECONNREFUSED` | `ECONNREFUSED` |
| `stream` `Transform` con `transform` option | ok | ok |
| `stream` `Duplex` / `PassThrough` con `end(chunk)` | ok | ok |
| `stream` `objectMode` con `end(chunk)` | ok | ok |
| `stream` `pipeline` de varias etapas | ok | ok |
| `stream` `pipeline` con error a mitad (propaga y cierra) | `boom` | `boom` |
| `stream` `finished()` (una sola vez) | ok | ok |
| `stream` `Readable.from` → `pipe` | ok | ok |
| Event loop: trabajo pendiente (socket/stream) mantiene vivo el proceso | ok | ok |
| Promesa rechazada sin manejar → salida ≠ 0 | exit 1 | exit 1 |

Nota sobre `net` backpressure: Node devuelve `true` para un único `write()` de
200 000 B porque su `_write` async deja el buffer del stream a 0 al aceptarlo
libuv; 3va devuelve `false` (cuenta los bytes antes de escribir) y emite
`'drain'` en el siguiente turno. Ambos entregan el cuerpo completo; el valor de
retorno es una divergencia menor documentada.

## Librerías reales (npm)

| Librería | Caso | Node 24 | 3va 2.12.6 |
|---|---|---|---|
| axios (adaptador `http`) | JSON, binario, 1 MB con SHA-256, respuesta binaria | ok | ok |
| axios + `form-data` | multipart con archivo binario | ok | ok |
| `form-data` + `http.request` | multipart con archivo binario | ok | ok |
| got | JSON, binario, respuesta binaria | ok | **pendiente** (require de ESM) |
| node-fetch | JSON, binario, respuesta binaria | ok | **pendiente** (`target.sort is not a function`) |
| undici (`request`) | JSON, binario | ok | **pendiente** (`zlib`/`stream` en `decompress.js`) |
| `@aws-sdk/client-s3` / SDK Stripe | contra mock | — | no probado |

`axios` usaba su **adaptador `fetch`** en 3va porque `process` no exponía
`Symbol.toStringTag`: `Object.prototype.toString.call(process)` devolvía
`[object Object]` en lugar de `[object process]`, `utils.kindOf(process)` no era
`"process"`, axios decidía que el adaptador `http` "no está soportado" y caía al
de `fetch`. Corregido en 2.12.6 (`process.rs`), con lo que axios usa su
adaptador `http` y pasa la matriz completa.

## Pendientes (con reproducción mínima)

- **`fetch` con `body` `ReadableStream` (`duplex: 'half'`)**: 3va rechaza con
  `TypeError: fetch: a ReadableStream request body is not supported by 3va yet`.
  `fetch` sigue leyendo el cuerpo completo antes de enviarlo.
  ```js
  const rs = new ReadableStream({ start(c) { c.enqueue(new Uint8Array([1,2,3])); c.close(); } });
  await fetch(url, { method: 'POST', body: rs, duplex: 'half' });
  ```
- **got**: `require('got')`/`import('got')` falla con
  `Unexpected token 'export'` al cargar `@sindresorhus/is` (dependencia ESM) —
  es un hueco del cargador de módulos, no del cliente `http`.
- **node-fetch**: `util.types.isAnyArrayBuffer` se añadió, pero la librería falla
  después con `target.sort is not a function` (iteración de `Headers`/`Map`).
- **undici**: falla en `lib/interceptor/decompress.js`
  (`Cannot read properties of undefined (reading 'bind')`) — falta de `zlib`
  (`createBrotliDecompress`/`createZstdDecompress`) y/o `stream`.
- **`pipe` de 200 MB**: la transferencia de 1 MB con SHA-256 se verificó; falta
  la medición formal de RSS para 200 MB.
- **SDK de AWS/Stripe contra mock**: no probado en esta entrega.

## Cambios de comportamiento que el cliente debe conocer

- `Buffer.isBuffer()` (ya en 2.12.5) es `true` sólo para `Buffer` reales.
- `process` ahora expone `Symbol.toStringTag === 'process'`, como Node. Es lo que
  hace que axios/got/etc. reconozcan el entorno Node.
- `net.Socket#end()` hace half-close: el socket sigue **legible** después de
  `end()` hasta que el peer cierra. Un `'close'` prematuro ya no se emite al
  terminar la escritura.
- `Writable`/`Transform`/`Duplex.end(chunk)` escribe el chunk antes de marcar el
  stream como terminado (comportamiento Node correcto).
- `Stream.prototype.pipe` (el `Stream` básico del que heredan `combined-stream`
  y `form-data`) es un `pipe` real, no un no-op.
