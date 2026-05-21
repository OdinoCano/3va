# 02 - CARGA DE MÓDULOS

## 2.1 Sistemas de Módulos Soportados

`vvva_js` soporta dos sistemas de módulos de forma simultánea:

| Sistema | Sintaxis | Soporte |
|---------|----------|---------|
| ESM | `import` / `export` | ✅ Completo |
| CommonJS | `require()` | ✅ Completo |

La detección es automática: si el archivo contiene sentencias `import` o `export` estáticas en cualquier línea (ignorando comentarios de bloque), se carga como módulo ESM. En caso contrario, se evalúa como script CommonJS.

---

## 2.2 ESM — Módulos ECMAScript

### 2.2.1 Implementación

El soporte ESM se implementa mediante dos structs en `crates/js/src/esm.rs` que implementan los traits `Resolver` y `Loader` de `rquickjs`:

```rust
// Resuelve el nombre del módulo a una ruta canónica absoluta
pub struct EsmResolver;

// Carga el contenido del módulo desde disco aplicando permisos
pub struct EsmLoader {
    pub permissions: PermissionState,
}
```

Se registran en el `Runtime` al crear el motor:

```rust
runtime.set_loader(EsmResolver, EsmLoader { permissions: permissions.clone() });
```

### 2.2.2 Algoritmo de resolución (`EsmResolver`)

1. Si `name` comienza con `./` o `../`: resolver relativo al directorio del módulo base (`base`).
2. Probar extensiones en orden: sin cambio → `.js` → `.ts` → `.jsx` → `.tsx`.
3. Canonicalizar la ruta resultante (`fs::canonicalize`) para eliminar symlinks y `..`.
4. Devolver la ruta canónica como identificador único del módulo.

```
resolve("./utils", "/app/src/index.ts")
  → prueba /app/src/utils
  → prueba /app/src/utils.js     ← no existe
  → prueba /app/src/utils.ts     ← existe → canonicalizar → "/app/src/utils.ts"
```

### 2.2.3 Algoritmo de carga (`EsmLoader`)

1. Verificar `Capability::FileRead(path)` en `PermissionState`. Si se deniega → error de permiso.
2. Leer el archivo con `fs::read_to_string`.
3. Si la extensión es `.ts` o `.tsx`: transpilar con el transpilador TypeScript integrado.
4. Entregar el source a QuickJS via `Module::declare(ctx, name, source)`.

### 2.2.4 Sintaxis soportada

```javascript
// Exportación nombrada
export const PI = 3.14159;
export function add(a, b) { return a + b; }

// Exportación por defecto
export default class Calculator { ... }

// Importación nombrada
import { add, PI } from './math.js';

// Importación por defecto
import Calculator from './Calculator.ts';

// Re-exportación
export { add } from './math.js';
export * from './utils.js';

// Módulo TypeScript importado desde JavaScript
import { format } from './formatter.ts';
```

### 2.2.5 Async/await y Promises

Las Promises y el código asíncrono funcionan gracias al bucle de microtareas ejecutado al finalizar cada archivo:

```rust
loop {
    match self.runtime.execute_pending_job() {
        Ok(true)  => continue,   // hay más jobs pendientes
        Ok(false) => break,      // cola vacía
        Err(e)    => return Err(anyhow::anyhow!("JS job error: {:?}", e)),
    }
}
```

Esto permite:

```javascript
// async/await
async function fetchData() {
    const result = await someAsyncOperation();
    return result;
}

// Cadenas Promise
Promise.resolve(42)
    .then(n => n * 2)
    .then(n => console.log(n)); // imprime 84
```

---

## 2.3 CommonJS

Los archivos que no contienen `import`/`export` estáticos se evalúan como scripts planos. `require()` no está disponible en modo ESM y viceversa.

---

## 2.4 Transpilación TypeScript

El transpilador TypeScript (`crates/js/src/transpiler.rs`) es una implementación pura en Rust que elimina anotaciones de tipo sin necesidad de `tsc` ni Node.js:

- Elimina declaraciones de tipo: `type`, `interface`, `enum`
- Elimina anotaciones en parámetros, retornos y variables
- Maneja genéricos, decoradores y modificadores de visibilidad (`public`, `private`, `readonly`)
- Se aplica automáticamente a archivos `.ts` y `.tsx` tanto en `run` como en `import`

---

## 2.5 Detección de módulo ESM

La función `is_esm_source(source: &str) -> bool` determina si un archivo debe tratarse como ESM:

- Escanea **todas** las líneas del archivo (no se detiene en la primera línea no-import).
- Ignora líneas dentro de comentarios de bloque `/* ... */`.
- Considera ESM si encuentra alguna línea que comience con `import `, `export `, `export default` o `export {`.

---

## 2.6 Permisos y Módulos

Cada `import` pasa por la verificación de permisos antes de leer el archivo:

```
import { x } from './lib.ts'
  → EsmLoader::load("/app/lib.ts")
    → PermissionState::check(FileRead("/app/lib.ts"))
      → Denegado si no hay --allow-read=/app o superior
```

Esto garantiza que un módulo no pueda leer archivos fuera de las rutas autorizadas incluso si el código JS intenta importarlos.

---

*ESM implementado en `crates/js/src/esm.rs`. Transpilador en `crates/js/src/transpiler.rs`. Tests en `crates/js/tests/pipeline.rs` (28 tests).*
