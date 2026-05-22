# 02 - MODULE LOADING

## 2.1 Supported Module Systems

`vvva_js` supports two module systems simultaneously:

| System | Syntax | Support |
|--------|--------|---------|
| ESM | `import` / `export` | ✅ Complete |
| CommonJS | `require()` | ✅ Complete |

Detection is automatic: if the file contains static `import` or `export` statements on any line (ignoring block comments), it is loaded as an ESM module. Otherwise, it is evaluated as a CommonJS script.

---

## 2.2 ESM — ECMAScript Modules

### 2.2.1 Implementation

ESM support is implemented via two structs in `crates/js/src/esm.rs` that implement the `Resolver` and `Loader` traits from `rquickjs`:

```rust
// Resolves the module name to an absolute canonical path
pub struct EsmResolver;

// Loads module content from disk applying permissions
pub struct EsmLoader {
    pub permissions: PermissionState,
}
```

They are registered in the `Runtime` when creating the engine:

```rust
runtime.set_loader(EsmResolver, EsmLoader { permissions: permissions.clone() });
```

### 2.2.2 Resolution Algorithm (`EsmResolver`)

1. If `name` starts with `./` or `../`: resolve relative to the base module directory (`base`).
2. Try extensions in order: unchanged → `.js` → `.ts` → `.jsx` → `.tsx`.
3. Canonicalize the resulting path (`fs::canonicalize`) to remove symlinks and `..`.
4. Return the canonical path as the module's unique identifier.

```
resolve("./utils", "/app/src/index.ts")
  → try /app/src/utils
  → try /app/src/utils.js     ← doesn't exist
  → try /app/src/utils.ts     ← exists → canonicalize → "/app/src/utils.ts"
```

### 2.2.3 Loading Algorithm (`EsmLoader`)

1. Check `Capability::FileRead(path)` in `PermissionState`. If denied → permission error.
2. Read the file with `fs::read_to_string`.
3. If the extension is `.ts` or `.tsx`: transpile with the built-in TypeScript transpiler.
4. Deliver the source to QuickJS via `Module::declare(ctx, name, source)`.

### 2.2.4 Supported Syntax

```javascript
// Named export
export const PI = 3.14159;
export function add(a, b) { return a + b; }

// Default export
export default class Calculator { ... }

// Named import
import { add, PI } from './math.js';

// Default import
import Calculator from './Calculator.ts';

// Re-export
export { add } from './math.js';
export * from './utils.js';

// TypeScript module imported from JavaScript
import { format } from './formatter.ts';
```

### 2.2.5 Async/await and Promises

Promises and async code work thanks to the microtask loop executed at the end of each file:

```rust
loop {
    match self.runtime.execute_pending_job() {
        Ok(true)  => continue,   // more jobs pending
        Ok(false) => break,      // empty queue
        Err(e)    => return Err(anyhow::anyhow!("JS job error: {:?}", e)),
    }
}
```

This enables:

```javascript
// async/await
async function fetchData() {
    const result = await someAsyncOperation();
    return result;
}

// Promise chains
Promise.resolve(42)
    .then(n => n * 2)
    .then(n => console.log(n)); // prints 84
```

---

## 2.3 CommonJS

Files that do not contain static `import`/`export` are evaluated as plain scripts. `require()` is not available in ESM mode and vice versa.

---

## 2.4 TypeScript Transpilation

The TypeScript transpiler (`crates/js/src/transpiler.rs`) is a pure Rust implementation that removes type annotations without needing `tsc` or Node.js:

- Removes type declarations: `type`, `interface`, `enum`
- Removes annotations on parameters, return types, and variables
- Handles generics, decorators, and visibility modifiers (`public`, `private`, `readonly`)
- Applies automatically to `.ts` and `.tsx` files in both `run` and `import`

---

## 2.5 ESM Module Detection

The function `is_esm_source(source: &str) -> bool` determines whether a file should be treated as ESM:

- Scans **all** lines of the file (does not stop at the first non-import line).
- Ignores lines within block comments `/* ... */`.
- Considers ESM if it finds any line starting with `import `, `export `, `export default` or `export {`.

---

## 2.6 Permissions and Modules

Each `import` goes through permission verification before reading the file:

```
import { x } from './lib.ts'
  → EsmLoader::load("/app/lib.ts")
    → PermissionState::check(FileRead("/app/lib.ts"))
      → Denied if there is no --allow-read=/app or broader
```

This ensures that a module cannot read files outside authorized paths even if the JS code tries to import them.

---

*ESM implemented in `crates/js/src/esm.rs`. Transpiler in `crates/js/src/transpiler.rs`. Tests in `crates/js/tests/pipeline.rs` (28 tests).*
