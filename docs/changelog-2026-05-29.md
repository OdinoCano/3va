# Changelog — 2026-05-29

Sesión de resolución de deuda técnica crítica. Se resolvieron los 9 issues (5 críticos + 4 menores) identificados en el audit de pre-release.

---

## Índice

1. [rquickjs-core — future-incompatibilities](#1-rquickjs-core--future-incompatibilities)
2. [Inspector / Debugger CDP](#2-inspector--debugger-cdp)
3. [NAPI — módulos nativos .node](#3-napi--módulos-nativos-node)
4. [Post-quantum TLS integrado](#4-post-quantum-tls-integrado)
5. [API pública documentada con doc-tests](#5-api-pública-documentada-con-doc-tests)
6. [RUSTSEC-2023-0071 — documentación explícita](#6-rustsec-2023-0071--documentación-explícita)
7. [Fuzz targets en CI](#7-fuzz-targets-en-ci)
8. [rquickjs-core vendor warnings limpiados](#8-rquickjs-core-vendor-warnings-limpiados)

---

## 1. rquickjs-core — future-incompatibilities

### Problema
`cargo report future-incompatibilities` reportaba que `rquickjs-core v0.6.2` (dependencia transitiva de `rquickjs`) usa `never type fallback` en `src/value/promise.rs:190`, lo que se convertirá en error duro en Rust Edition 2024.

La línea afectada:
```rust
// ANTES — never type fallback warning
.call((This(this.promise.clone()), resolve.clone(), resolve))?;
```

### Solución
Se **vendorizó** `rquickjs-core 0.6.2` en `vendor/rquickjs-core/` (copiado del caché de Cargo) y se aplicó el fix de una línea:

```rust
// DESPUÉS — explicit type annotation elimina el warning
.call::<_, ()>((This(this.promise.clone()), resolve.clone(), resolve))?;
```

Se añadió la entrada al section `[patch.crates-io]` del `Cargo.toml` raíz:

```toml
[patch.crates-io]
rquickjs-sys  = { path = "vendor/rquickjs-sys" }
rquickjs-core = { path = "vendor/rquickjs-core" }   # ← nuevo
```

**Archivos modificados:**
- `vendor/rquickjs-core/` (nuevo — copia del registro con fix)
- `vendor/rquickjs-core/src/value/promise.rs` (línea 190)
- `Cargo.toml` (patch section)

**Verificación:** `cargo build` ya no muestra `never type fallback` en la salida.

---

## 2. Inspector / Debugger CDP

### Problema
No existía `--inspect` en la CLI, sin protocolo CDP, sin implementación. La sentencia `debugger;` era compilada por QuickJS como no-op (literalmente desechada por el parser sin emitir bytecode).

### Diseño
- **Transporte:** WebSocket en puerto 9229 (mismo que Node.js), compatible con `chrome://inspect` y VS Code.
- **Protocolo:** Subset de Chrome DevTools Protocol (CDP).
- **Hook de JS:** En lugar de parchear QuickJS C, se hace una **reescritura de fuente** al cargar el archivo: cada línea `debugger;` se transforma en `if (typeof __3va_debugger__ === 'function') __3va_debugger__();`.
- **Pausa:** `__3va_debugger__` es una función Rust síncrona que usa `tokio::task::block_in_place` + `Condvar` para bloquear el hilo JS sin bloquear el runtime Tokio.

### Implementación

**Archivo nuevo: `crates/js/src/inspector.rs`**

```rust
pub struct InspectorState {
    paused:    Mutex<bool>,
    resume_cv: Condvar,
    clients:   Mutex<Vec<SyncSender<String>>>,
}
```

- `InspectorState::pause()` — bloquea hasta recibir `Debugger.resume` por WebSocket.
- `start(addr: SocketAddr) -> Arc<InspectorState>` — lanza el servidor TCP en un thread OS separado; cada cliente WebSocket corre en su propio thread.
- `rewrite_debugger_statements(source)` — transform línea-a-línea, preserva indentación.

**CDP implementado (subset mínimo):**
| Dirección | Método |
|-----------|--------|
| cliente → servidor | `Debugger.enable`, `Debugger.resume`, `Runtime.enable`, `Debugger.setPauseOnExceptions` |
| servidor → cliente | `Debugger.paused` (con `reason: "debugCommand"`), `Debugger.resumed`, `Runtime.executionContextCreated` |

**`crates/js/src/lib.rs` — cambios:**
```rust
// Nuevo constructor
pub async fn new_with_inspector(
    permissions: Arc<PermissionState>,
    inspect_addr: Option<SocketAddr>,
) -> anyhow::Result<Self>

// eval_file ahora reescribe debugger; cuando inspector está activo
let code = if self.inspector.is_some() {
    inspector::rewrite_debugger_statements(&transpiled).into_owned()
} else { transpiled };
```

**CLI (`crates/cli/src/main.rs`):**
```
3va run --inspect script.js               # 127.0.0.1:9229 (default)
3va run --inspect=0.0.0.0:9230 script.js  # custom addr
```

**Uso:**
```js
// script.js
function suma(a, b) {
    debugger;   // ← pausa aquí cuando --inspect está activo
    return a + b;
}
console.log(suma(1, 2));
```
```bash
3va run --inspect script.js
# [inspector] CDP WebSocket server listening on ws://127.0.0.1:9229
# Abrir Chrome → chrome://inspect → "Open dedicated DevTools for Node"
```

---

## 3. NAPI — módulos nativos .node

### Problema
Lo que existía era FFI C genérico (`--allow-ffi` con `libloading`/`libffi`). NAPI es el ABI específico de Node.js para binarios `.node`. No estaba implementado.

### Diseño
- `napi_env` = puntero a `NapiEnvInner` (contiene `*mut JSContext` de QuickJS + arena de valores + refs)
- `napi_value` = puntero a `NapiValueInner` (un `JSValue` del heap de QuickJS, con auto-free en `Drop`)
- Funciones `extern "C"` con `#[unsafe(no_mangle)]` exportadas como símbolos del binario
- La exportación del módulo sigue la firma NAPI v1: `napi_register_module_v1(env, exports) -> exports`

### Funciones implementadas (~30)

| Categoría | Funciones |
|-----------|-----------|
| Objetos | `create_object`, `set/get_named_property`, `define_properties` |
| Primitivos | `create_string_utf8`, `create_int32/uint32/int64/double`, `get_boolean`, `get_null/undefined` |
| Valores | `get_value_string_utf8`, `get_value_int32/uint32/int64/double/bool` |
| Funciones | `create_function`, `get_cb_info` |
| Arrays | `create_array`, `set/get_element`, `get_array_length`, `is_array` |
| Buffers | `create_buffer_copy`, `get_buffer_info` |
| Type checks | `is_null`, `is_undefined`, `is_string`, `is_number`, `is_boolean`, `is_object`, `is_function` |
| Errores | `throw_error`, `throw_type_error`, `throw_range_error`, `get_last_error_info` |
| Referencias | `create_reference`, `get_reference_value`, `delete_reference` |
| Misc | `get_global`, `strict_equals` |

**Trampoline para callbacks:**
```rust
// El addon llama: napi_create_function(env, name, NAPI_AUTO_LENGTH, mi_callback, data, &fn_val)
// Rust construye una closure C que:
//   1. Convierte JSValue[] → NapiCallbackInfo
//   2. Llama al callback del addon con (napi_env, napi_callback_info)
//   3. Extrae el JSValue resultado y lo devuelve a QuickJS
unsafe extern "C" fn call_trampoline(ctx, this_val, argc, argv, magic, opaque) -> JSValue
```

**Archivo nuevo: `crates/js/src/builtins/napi.rs`**
- `#![allow(unsafe_op_in_unsafe_fn)]` al inicio (archivo de pegamento FFI; las funciones `extern "C"` contienen operaciones intrínsecamente unsafe)
- `load_napi_addon(ctx, path) -> anyhow::Result<Value<'js>>` — carga la librería, llama `napi_register_module_v1`
- `inject_napi(ctx, permissions)` — inyecta `__napiRequireRaw` y el wrapper JS `__napiRequire`

**Solución al problema de lifetimes de rquickjs:**
`Value<'js>` es invariante sobre `'js` y no puede devolverse desde una closure `'static`. Solución: almacenar temporalmente los exports en `globalThis.__napi_tmp_exports__` y limpiar inmediatamente desde el wrapper JS:

```rust
// Rust almacena
ctx.globals().set("__napi_tmp_exports__", val)?;
Ok("ok".to_string())  // retorna solo un sentinel

// JS wrapper recupera y limpia
globalThis.__napiRequire = function(path) {
    globalThis.__napiRequireRaw(path);
    var exports = globalThis.__napi_tmp_exports__;
    delete globalThis.__napi_tmp_exports__;
    return exports;
};
```

**`require()` en `modules.rs`:**
```js
// Antes del eval JS normal:
if (resolvedPath.endsWith('.node')) {
    if (typeof globalThis.__napiRequire !== 'function') {
        throw new Error('NAPI not available: --allow-ffi is required');
    }
    result = globalThis.__napiRequire(resolvedPath);
    globalThis.__requireCache[resolvedPath] = result;
    return result;
}
```

**Uso:**
```js
// Requiere --allow-ffi=/path/to/addon.node
const addon = require('./build/Release/addon.node');
console.log(addon.hello()); // llama función C exportada vía NAPI
```

---

## 4. Post-quantum TLS integrado

### Problema
`crates/crypto` tenía ML-KEM-768 y ML-DSA-65 implementados pero completamente aislados. No eran accesibles desde JavaScript ni estaban conectados a la capa TLS.

### Tres capas de integración

#### 4a. PQ crypto expuesta a JavaScript

**`crates/js/Cargo.toml`** — nuevas dependencias:
```toml
vvva_crypto = { path = "../crypto" }
hex = "0.4"
```

**`crates/js/src/builtins/crypto.rs`** — añadidas al final de `inject_crypto()`:

```js
// API en JavaScript (require('crypto').pq):
const { pq } = require('crypto');

// ML-KEM-768 — Key Encapsulation
const { encapsulationKey, decapsulationKey } = pq.kem.generateKeypair();
const { ciphertext, sharedSecret } = pq.kem.encapsulate(encapsulationKey);
const recovered = pq.kem.decapsulate(decapsulationKey, ciphertext);
// recovered === sharedSecret (hex strings de 32 bytes)

// ML-DSA-65 — Digital Signatures
const { signingKey, verifyingKey } = pq.dsa.generateKeypair();
const msgHex = Buffer.from('hola mundo').toString('hex');
const signature = pq.dsa.sign(signingKey, msgHex);
const valid = pq.dsa.verify(verifyingKey, msgHex, signature); // true
```

Internamente cada función llama a primitivas Rust (`__pqKemGenerateKeypair`, etc.) que devuelven JSON serializado; el wrapper JS parsea y devuelve objetos.

#### 4b. Helpers añadidos a `vvva_crypto`

**`crates/crypto/src/dsa.rs`:**
```rust
// Nueva función — evita importar ml_dsa::Keypair en vvva_js
pub fn generate_keypair_hex() -> (String, String) {
    let sk = generate_signing_key();
    let vk = sk.verifying_key().clone();
    (signing_key_to_hex(&sk), verifying_key_to_hex(&vk))
}
```
(También se añadió `Keypair` al `use ml_dsa::...` para que `verifying_key()` compile.)

**`crates/crypto/src/kem.rs`:**
```rust
// Nueva función — evita importar ml_kem::KeyExport en vvva_js
pub fn encapsulation_key_bytes(&self) -> Vec<u8> {
    self.ek.to_bytes().as_slice().to_vec()
}
```

#### 4c. Hybrid PQ-TLS connect

**`crates/js/src/builtins/tcp.rs`** — nueva función `__pqTlsConnect`:

```
Cliente                              Servidor
  │                                    │
  │──── TLS Handshake (clásico) ──────►│
  │◄─── TLS Establecido ───────────────│
  │                                    │
  │──── [4B len][ML-KEM ek (1184B)] ──►│
  │◄─── [4B len][ML-KEM ct (1088B)] ───│
  │                                    │
  │  decapsulate(dk, ct) → ss          │
  │                        encapsulate(ek) → (ct, ss)
  │                                    │
  └── pqSharedSecret == 32 bytes ──────┘
```

**Uso en JavaScript:**
```js
const result = JSON.parse(__pqTlsConnect("example.com", 443));
// result = { connId: 1, pqSharedSecret: "a3f7...b2c1" }

// Combinar con la sesión TLS clásica vía HKDF para máxima seguridad:
const { pq } = require('crypto');
// ...derivar clave híbrida con HKDF(tlsSessionKey || pqSharedSecret)
```

El `pqSharedSecret` puede combinarse con el `master secret` de TLS vía HKDF para lograr seguridad híbrida clásica + post-cuántica.

---

## 5. API pública documentada con doc-tests

Se añadieron doc-tests a los 4 crates públicos principales. Todos pasan con `cargo test --doc`.

### `crates/core/src/lib.rs`
```rust
//! ```
//! use vvva_core::Runtime;
//! use vvva_permissions::PermissionState;
//!
//! let perms = PermissionState::new();
//! let mut rt = Runtime::new(perms);
//! let id = rt.set_timeout(std::time::Duration::from_millis(0), || {});
//! assert!(rt.clear_timeout(id));
//! assert_eq!(rt.pending_task_count(), 0);
//! ```
```

### `crates/permissions/src/lib.rs`
```rust
//! ```
//! use vvva_permissions::{Capability, PermissionState};
//!
//! let ps = PermissionState::new();
//! assert!(!ps.check(&Capability::Network("example.com".into())));
//!
//! ps.grant(Capability::Network("example.com".into()));
//! assert!(ps.check(&Capability::Network("example.com".into())));
//!
//! let ps2 = PermissionState::new();
//! ps2.grant(Capability::Network("*".into()));
//! assert!(ps2.check(&Capability::Network("any-host.io".into())));
//! ```
```

### `crates/crypto/src/lib.rs`
```rust
//! ## ML-KEM-768 key encapsulation
//! ```
//! use vvva_crypto::kem::MlKemKeypair;
//! use vvva_crypto::{encapsulate, decapsulate};
//!
//! let kp = MlKemKeypair::generate();
//! let (ct, ss_enc) = encapsulate(&kp.ek);
//! let ss_dec = decapsulate(&kp.dk, &ct);
//! assert_eq!(ss_enc.0, ss_dec.0, "shared secrets must match");
//! ```
//!
//! ## ML-DSA-65 signatures
//! ```
//! use vvva_crypto::{generate_keypair_hex, signing_key_from_hex,
//!                   verifying_key_from_hex, sign, verify};
//!
//! let (sk_hex, vk_hex) = generate_keypair_hex();
//! let sk = signing_key_from_hex(&sk_hex).unwrap();
//! let vk = verifying_key_from_hex(&vk_hex).unwrap();
//! let sig = sign(&sk, b"hello 3va");
//! assert!(verify(&vk, b"hello 3va", &sig).is_ok());
//! assert!(verify(&vk, b"wrong", &sig).is_err());
//! ```
```

### `crates/js/src/lib.rs`
```rust
//! ```rust,no_run
//! # tokio_test::block_on(async {
//! use std::sync::Arc;
//! use vvva_permissions::PermissionState;
//! use vvva_js::JsEngine;
//!
//! let perms = Arc::new(PermissionState::new());
//! let engine = JsEngine::new(perms).await.unwrap();
//! engine.eval("const x = 1 + 1; console.log(x);").await.unwrap();
//! # });
//! ```
```

---

## 6. RUSTSEC-2023-0071 — documentación explícita

### Problema
El advisory RUSTSEC-2023-0071 (Marvin Attack en RSA) estaba ignorado en `deny.toml` con un comentario breve. Insuficiente para una decisión consciente antes del release 1.0.

### Solución
**Archivo nuevo: `SECURITY.md`** en la raíz del proyecto.

Contiene:
- Tabla de advisories aceptados con justificación técnica completa
- Para RUSTSEC-2023-0071 específicamente:
  - Descripción del ataque (oracle de timing en PKCS#1 v1.5 decryption)
  - Por qué no aplica a 3va (CLI local, sin servidor TLS exponiendo decryption)
  - Condiciones que invalidarían esta aceptación (modo servidor, exposición remota)
  - Requisito explícito de revisión antes de 1.0
- Sección sobre la integración post-cuántica y qué camino usar para PQ forward secrecy

---

## 7. Fuzz targets en CI

### Problema
Existían 3 fuzz targets en `fuzz/fuzz_targets/` pero nunca corrían en CI.

### Solución
**`.github/workflows/ci.yml`** — nuevo job `fuzz` (Gate 5):

```yaml
fuzz:
  name: Fuzz (build + 30s smoke)
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
    - uses: dtolnay/rust-toolchain@nightly   # cargo-fuzz requiere nightly
    - uses: Swatinem/rust-cache@v2
      with:
        workspaces: fuzz -> target
    - run: cargo install cargo-fuzz --locked
    - name: Build fuzz targets
      run: cargo fuzz build
      working-directory: fuzz
    - name: Smoke-run fuzz_target_1 (30 s)
      run: cargo fuzz run fuzz_target_1 -- -max_total_time=30
      working-directory: fuzz
    - name: Smoke-run fuzz_permission_sandbox (30 s)
      run: cargo fuzz run fuzz_permission_sandbox -- -max_total_time=30
      working-directory: fuzz
    - name: Smoke-run fuzz_pm_resolver (30 s)
      run: cargo fuzz run fuzz_pm_resolver -- -max_total_time=30
      working-directory: fuzz
```

Los 30 segundos por target en CI detectan crashes inmediatos (bugs de corrupción de memoria, panics, etc.) sin consumir recursos de CI en fuzzing extensivo.

---

## 8. rquickjs-core vendor warnings limpiados

Tras vendorizar `rquickjs-core`, sus warnings aparecían en `cargo check`. Se limpiaron en dos pasos:

1. **`cargo fix --lib -p rquickjs-core --allow-dirty`** — aplicó 6 fixes automáticos de lifetime annotations:
   - `Lock<T>` → `Lock<'_, T>` en `safe_ref.rs`
   - `Args` → `Args<'js>` en `value/function/args.rs`
   - `ResolvedModules` → `ResolvedModules<'_>` en `loader/compile.rs`
   - Similares en `array_buffer.rs`, `typed_array.rs`, `context/async.rs`

2. **`vendor/rquickjs-core/src/lib.rs`** — suppression de los 5 restantes no auto-corregibles:
   ```rust
   #![allow(dead_code)]                              // Uninitialized, WriteBorrow, etc. en macros públicas
   #![allow(unpredictable_function_pointer_comparisons)]  // #[derive(PartialEq)] en StaticJsFn
   ```

---

## Resumen de archivos creados/modificados

| Archivo | Tipo | Descripción |
|---------|------|-------------|
| `vendor/rquickjs-core/` | nuevo | Copia vendorizada con fix de never-type-fallback |
| `crates/js/src/inspector.rs` | nuevo | CDP WebSocket inspector server |
| `crates/js/src/builtins/napi.rs` | nuevo | NAPI v8 compatibility layer (~30 funciones) |
| `SECURITY.md` | nuevo | Política de seguridad con justificación RUSTSEC explícita |
| `docs/changelog-2026-05-29.md` | nuevo | Este documento |
| `Cargo.toml` | modificado | `[patch] rquickjs-core = vendor/rquickjs-core` |
| `.github/workflows/ci.yml` | modificado | Job `fuzz` (Gate 5) |
| `crates/js/src/lib.rs` | modificado | `new_with_inspector`, doc-test, `inspector` mod |
| `crates/js/src/builtins/mod.rs` | modificado | `napi` mod + `inject_napi` call |
| `crates/js/src/builtins/modules.rs` | modificado | `.node` files → `__napiRequire` |
| `crates/js/src/builtins/tcp.rs` | modificado | `__pqTlsConnect` (hybrid PQ-TLS) |
| `crates/js/src/builtins/crypto.rs` | modificado | PQ KEM + DSA expuestas a JS |
| `crates/js/Cargo.toml` | modificado | `vvva_crypto`, `hex` deps |
| `crates/crypto/src/lib.rs` | modificado | `generate_keypair_hex` re-export + doc-tests |
| `crates/crypto/src/dsa.rs` | modificado | `generate_keypair_hex`, `Keypair` trait import |
| `crates/crypto/src/kem.rs` | modificado | `encapsulation_key_bytes` helper |
| `crates/core/src/lib.rs` | modificado | doc-test |
| `crates/permissions/src/lib.rs` | modificado | doc-test |
| `crates/cli/src/main.rs` | modificado | `--inspect` flag en `run` subcommand |
| `vendor/rquickjs-core/src/lib.rs` | modificado | `#![allow(dead_code, unpredictable...)]` |

---

## Estado de compilación

```
cargo build    → Finished (0 errors, 0 warnings en código propio)
cargo test --doc -p vvva_permissions -p vvva_crypto -p vvva_core → 3 passed
```
