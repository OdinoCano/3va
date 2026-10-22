# 08 - Plan: usar los streams de Node en lugar de los propios

Estado: **propuesto, prioritario** (2026-10-22). Es la primera tarea después de fusionar 2.13.0.

## 8.1 Por qué

`stream`, `events`, `buffer` y gran parte de `http` de 3va son reimplementaciones simplificadas en JavaScript
(`crates/js/src/builtins/modules.rs`), no el código de Node. La medición de la suite oficial de Node
(`docs/12-roadmap/04-compatibility.md`, 21.2 % en 2.13.0) muestra el efecto:

- de 250 tests `test-stream-*` pasan unos 20, y otros 23 terminan con "Mismatched function calls": el callback
  que Node llama no se llama, porque la semántica de `read()`, `readable`, `pipe`, `destroy` o `finished` es
  distinta;
- `http`, `fs`, `net` y `zlib` se apoyan en esos streams, así que cada diferencia se repite en ellos;
- cada arreglo suelto (en 2.13.0: `Writable` de una escritura a la vez, `decodeStrings`,
  `setDefaultEncoding`, `destroyed`, `StringDecoder` base64) sube pocos tests y puede romper otros: 12
  regresiones aparecieron de golpe al endurecer `destroy`.

Los grandes saltos de 2.13.0 no vinieron de detalles sino de brechas de base (`server.listen(callback)`,
`net.Server.address().port`, `unref()`, escrituras no bloqueantes). Quedan más de esas, y reimplementar a mano
la semántica de streams de Node no converge. El objetivo del proyecto (más del 95 % de la suite) pasa por usar
el mismo código que Node.

## 8.2 Qué se vendoriza

Del repositorio de Node, **a la misma versión que fija `tests/node/COMMIT`** (hoy `f14cd3aa7bf9`), licencia MIT:

| Origen en Node | Para qué |
|---|---|
| `lib/internal/streams/*` (readable, writable, duplex, transform, passthrough, pipeline, end-of-stream, destroy, compose, operators, utils, state, legacy, from, add-abort-signal, duplexify) | el núcleo |
| `lib/stream.js`, `lib/stream/promises.js` | la fachada pública |
| `lib/events.js` | `EventEmitter`, `once`, `on`, `getEventListeners`, `EventTarget` helpers |
| `lib/string_decoder.js` | `StringDecoder` |
| `lib/internal/errors.js`, `internal/validators.js`, `internal/util.js`, `internal/util/types.js` | códigos de error y validación exactos |
| `lib/internal/per_context/primordials.js` | los primordials que usa todo lo anterior |
| `lib/internal/abort_controller.js`, `internal/event_target.js` (si hace falta) | señales |

`buffer` **no** se vendoriza al principio: `Buffer` sigue siendo el nativo de 3va. Las piezas que Node espera
de `internalBinding(...)` se cubren con una capa pequeña propia (ver 8.4).

## 8.3 Cómo se trae y se verifica el código (cadena de suministro)

- El árbol de Node que hay en `.compatibility/node` (ignorado por git) está en **otro commit**
  (`45d6d31279b`) que el de los tests (`f14cd3aa7bf9`): no sirve como origen. El origen es el commit de
  `tests/node/COMMIT`.
- Un script `scripts/vendor-node-lib.sh` descarga los archivos de la lista, **a un commit fijo**, y comprueba el
  `sha256` de cada uno contra una tabla commiteada (`vendor/node-lib/SHA256SUMS`). Sin red en el build: los
  archivos se commitean en `vendor/node-lib/` con la cabecera `// Vendored from nodejs/node <commit> — MIT`.
- Se añade `vendor/node-lib/LICENSE` (la de Node) y se enlaza en `NOTICE`.
- Nada se edita dentro de `vendor/node-lib/`. Las diferencias necesarias viven en un parche
  (`vendor/node-lib/patches/*.patch`) que el script reaplica, de modo que actualizar la versión de Node es
  repetir el script y revisar el parche.
- Revisión de seguridad antes de fusionar: procedencia (hash contra el release firmado de Node), que no hay
  `eval`/`new Function`/acceso a ficheros o red en lo traído, y que no se debilita ningún límite del cortafuegos.

## 8.4 Cómo se carga

1. Los archivos se incluyen en el binario (`include_str!`) y se compilan una vez con el code cache de V8, como
   ya se hace con los bootstrap (`bootstrap_js`), para no empeorar el arranque.
2. Un cargador mínimo resuelve `require('internal/...')` solo para esa lista cerrada; el código de usuario
   nunca puede pedir `internal/*`.
3. Una capa de compatibilidad (`internalBinding('util')`, `types`, `errors`, `messaging` solo lo usado) se
   escribe a mano y es corta; todo lo que no se use se deja fuera.
4. `require('stream')`, `require('events')` y `require('string_decoder')` pasan a devolver los módulos de Node;
   `http`, `fs`, `net`, `zlib` y los streams de la web se adaptan para heredar de ellos.

## 8.5 Fases y puerta de cada una

Cada fase termina con su commit y **no avanza si falla su puerta**. Antes de cada push se corre la suite completa
de Node (`NODE_TESTS_BASELINE_OUT`, unos 4 min), no solo `cargo test`.

1. **`events` y `string_decoder`** (los más pequeños). Puerta: 0 fallos nuevos en la línea base.
2. **`stream` vendorizado detrás de un interruptor** (`THREEVA_NODE_STREAMS=1`), sin tocar `http` ni `fs`.
   Puerta: los tests `test-stream-*` suben de ~20 a más de 150 con el interruptor, y todo lo demás no empeora.
3. **`fs` y `net`** heredan de los streams de Node (`ReadStream`, `WriteStream`, `Socket`). Puerta: línea base
   sin regresiones; `fs` y `net` suben.
4. **`http`**: `IncomingMessage`, `OutgoingMessage` y `ServerResponse` sobre los streams de Node. Puerta: el
   diferencial (`scripts/differential.sh`) con `unexpected=0` y los 60 escenarios, y
   `scripts/perf-check.sh` dentro del 15 % (rendimiento HTTP y arranque).
5. **Quitar el interruptor y la implementación propia.** Puerta: los mismos criterios, más `scripts/test262.sh`
   sin fallos nuevos y el fuzz de 30 s.

## 8.6 Riesgos y límites

- **Arranque**: cargar más JavaScript cuesta. Se mide en cada fase; el presupuesto es el 15 % de
  `scripts/perf-check.sh`. Si no cabe, se difiere la carga de `stream/promises` y de los operadores.
- **Rendimiento HTTP**: las respuestas pasan por más capas; se mide con las 4 rondas alternadas de
  `perf-check.sh`. El camino caliente de `res.end(body)` se mantiene sin pasar por el stream completo si hace
  falta, como ya lo hace 3va.
- **Seguridad**: los límites del cortafuegos (slowloris, RUDY, tamaño de cabeceras) viven en el lado nativo y no
  se tocan; el cambio es solo de la capa JavaScript.
- **Compatibilidad hacia atrás**: el comportamiento pasa a ser el de Node. Cualquier diferencia que un usuario
  note se documenta en el CHANGELOG como corrección hacia Node, no como cambio de diseño.

## 8.7 Lo que no es este plan

No toca `buffer`, el motor de módulos ni los límites de seguridad. La lista de causas de cuelgue sueltas
(`keep-alive`, `pipeline`, `upgrade`, errores de parseo del cliente, 'connection' en el `accept`) sigue su curso
y se reevalúa después de la fase 4, porque parte de ellas desaparecerá con los streams correctos.
