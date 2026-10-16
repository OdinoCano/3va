# 07 - Estándar de verificación de compatibilidad con Node

Este documento fija cómo se justifica que 3va es compatible con Node.js y qué hay que comprobar antes de
afirmarlo en un PR, en el CHANGELOG o en la documentación. Es la norma del proyecto para cualquier cambio
en la capa de API de Node.

## 7.1 Por qué existe

Entre 2.12.0 y 2.12.6 un usuario que migraba una app Express de producción encontró, release tras
release, fallos que habíamos dado por verificados:

- el `await` de nivel superior devolvía la Promise;
- `express.json()` no llenaba `req.body` (Express 4 y 5, por motivos distintos);
- los cuerpos binarios de las peticiones se corrompían y los grandes agotaban la memoria;
- `http.request` enviaba el POST sin cuerpo (lo que usan axios, got, los SDK de AWS/Stripe);
- `fetch` mandaba los cuerpos binarios como texto;
- `net` perdía la respuesta en el half-close y `PassThrough.end(chunk)` no emitía nada.

Casi todos venían de antes de 2.12.0. No eran regresiones: la verificación no los podía ver.

| Lo que teníamos | Qué mide | Por qué no los detectó |
|---|---|---|
| test262 (`06-test262.md`) | El lenguaje ECMAScript | Esa capa es V8 y funciona. Los fallos estaban en la API de Node, que 3va reimplementa y test262 no toca |
| Tests propios (`crates/js/tests/`) | Casos escritos por nosotros | Camino feliz, casi siempre cliente y servidor en el mismo proceso |
| `scripts/compat-check.sh` | Que `require('express')` carga | No ejecuta la librería |
| `scripts/compat-scan.sh` | Que el código se parsea | No lo ejecuta |

Además había métodos que aparentaban funcionar sin hacer nada (`function() { return this; }`), como
`Stream.prototype.pipe`. Un test de "¿existe el método?" los da por buenos.

## 7.2 Las capas y cómo se verifica cada una

Preparación del entorno: `./scripts/dev-setup.sh --all` instala los hooks, avisa de las herramientas que
faltan (node 24, npm, docker, hyperfine 1.x, oha…) y descarga test262 (`tests/test262/`) y la suite de Node
(`tests/node/`). Por separado: `--with-test262` o `--with-node-tests`. Ver también `CONTRIBUTING.md`.

| Capa | Herramienta | Estado |
|---|---|---|
| Lenguaje (ECMAScript) | test262, gate de trinquete en CI | Activo (`06-test262.md`) |
| API de Node (`http`, `stream`, `Buffer`, `fs`, `net`, `zlib`, `crypto`…) | Suite oficial de Node (`test/parallel` de nodejs/node) con línea base y trinquete | **Pendiente** (§7.4, paso 2) |
| Comportamiento observable frente a Node | Pruebas diferenciales: el mismo script con `node` y con `3va`, comparando la salida | **Pendiente** de convertir en suite de CI (§7.4, paso 1) |
| Librerías reales | Las suites de tests de las propias librerías (`npm test`) ejecutadas con 3va | **Pendiente** (§7.4, paso 4) |
| Uso real de un cliente | Escenario completo con el binario de release en la imagen de producción | **Pendiente** (§7.4, paso 5) |

## 7.3 Reglas para todo PR que toque la API de Node

1. **Prueba diferencial obligatoria.** Cada fallo o función nueva se reproduce con un script mínimo que se
   ejecuta con `node` (24) y con `3va`. El PR muestra la salida de ambos, antes y después.
2. **Cliente y servidor en procesos distintos** cuando intervienen red, sockets o HTTP. En un solo proceso
   hay comportamientos que no se dan en producción.
3. **La matriz, no el caso.** Un arreglo en una API se prueba en la matriz de esa API, no solo en el caso
   reportado. Por ejemplo, un cambio en el cuerpo de la petición cubre Express 4 y 5, con y sin
   middleware asíncrono, JSON, vacío, binario (SHA-256), multipart, chunked y malformado.
4. **Bytes, no texto.** Los datos binarios se comparan por hash. Nunca se pasa un cuerpo por
   `String::from_utf8_lossy` ni por `String(uint8array)`.
5. **Sin métodos falsos.** No se añade un método que no haga lo que dice. Si algo no está soportado, lanza
   un error claro (`ERR_NOT_SUPPORTED`) o queda documentado como no-op intencionado con su motivo.
6. **Un test de regresión que falle sin el arreglo**, en `crates/js/tests/`.
7. **Ninguna afirmación sin la prueba.** El CHANGELOG y la documentación solo dicen "funciona" o
   "compatible" sobre lo que se probó en la versión publicada, y el PR enlaza o incluye esa prueba.
8. **Dependencias.** Nada de `cargo update` general: solo la dependencia que el cambio necesite o la que
   exija un advisory. `cargo vet --locked` y `cargo deny check` en verde. No usar
   `cargo vet regenerate imports` sin revisar el diff de `supply-chain/config.toml`, porque borra
   exenciones con notas de revisión manual.

9. **Tests que crean procesos.** Siempre dentro de `systemd-run --user --scope -p MemoryMax=8G
   -p TasksMax=1024`, una sola tanda a la vez. Para matar el grupo de procesos de un test usa
   `kill -KILL -- -<pgid>` (con `--`) o `libc::kill(-(pgid as i32), SIGKILL)`, nunca con `pgid <= 1`:
   en procps-ng 4.0.4, `kill -KILL -<pgid>` sin `--` se convierte en `kill(-1, SIGKILL)` y mata todos los
   procesos del usuario (tumbó la sesión de escritorio dos veces el 2026-10-16).

## 7.4 Hoja de ruta (en este orden)

1. **Suite diferencial en CI.** Convertir en una suite fija las matrices de 2.12.3 a 2.12.6 (Express 4 y
   5, Fastify, Koa, axios, multer, `pg`, `http`/`fetch`/`net` como cliente y servidor, binarios por
   SHA-256), con `node` y `3va` en CI y fallo si la salida difiere. Es lo que encontró todos los fallos
   anteriores, y el primer paso porque reutiliza lo que ya existe.
2. **Suite oficial de Node.** Ejecutar `test/parallel` de nodejs/node (versión fijada), empezando por
   `test-buffer-*`, `test-stream-*`, `test-http-*`, `test-net-*`, `test-fs-*`, `test-zlib-*`. Línea base
   de fallos y gate de trinquete igual que test262: si sube es una regresión, si baja se actualiza la
   constante. Es lo que usan Bun y Deno para medir su compatibilidad.
3. **Inventario de métodos falsos.** Revisar cada `function() { return this; }` y cada stub de
   `crates/js/src/builtins/`: implementarlo, hacerlo fallar con un error claro o documentarlo.
4. **Suites de las librerías.** `npm test` de express, body-parser, axios, `pg`, multer, etc. con 3va, con
   resultados guardados por versión.
5. **Escenario de cliente como gate de release.** El binario de release en `debian:bookworm-slim` con una
   app Express + `pg` + axios + subida de archivos detrás de un proxy. Si falla, no hay release.
6. **Tabla de compatibilidad medida.** Los porcentajes por módulo de `12-roadmap/04-compatibility.md`, del
   README, de la web y de la wiki se generan desde los pasos 2 y 4, no se estiman a mano.

Hasta completar los pasos 1 y 2, cualquier porcentaje de compatibilidad con Node que publiquemos es una
estimación y debe decirlo.

## 7.5 Antes de cada release

- [ ] Las pruebas diferenciales del área tocada, con su salida en el PR.
- [ ] `cargo test -p vvva_js -p vvva_firewall -p vvva_cli --no-fail-fast` completo.
- [ ] test262 sin fallos nuevos frente a la lista de la versión anterior (`scripts/test262.sh`).
- [ ] `scripts/perf-check.sh` frente a la versión anterior, en rondas alternadas (es ruidoso).
- [ ] `scripts/compat-check.sh` y `scripts/compat-scan.sh`.
- [ ] `cargo vet --locked` y `cargo deny check` en verde; solo cambian las dependencias justificadas.
- [ ] El Security Pipeline en verde sobre el commit que se va a etiquetar (el workflow de release lo
      exige).
- [ ] El binario publicado se prueba en `debian:bookworm-slim` (glibc 2.36) con el flujo del cliente.
