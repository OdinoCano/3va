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

ESM support is implemented via two structs in `crates/js/src/esm.rs` that implement the `Resolver` and `Loader` traits for V8:

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
4. Deliver the source to V8 via the module evaluation API.

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

## 2.6 `require()` — Inline ESM→CJS Conversion

When a file loaded via `require()` contains static `import`/`export` syntax, the runtime automatically converts it to CommonJS inline before evaluation. This allows using ESM npm packages (including TypeScript source packages that ship with `"main": "src/index.ts"`) without a separate build step.

### 2.6.1 Detection

```javascript
// Triggers conversion if the source matches (multiline):
/^\s*(import\s|import\{|export\s|export\{|export\s*default)/m
```

### 2.6.2 Conversion Rules (`__esmToCjs`)

| ESM syntax | CJS equivalent |
|---|---|
| `import 'specifier'` | `require('specifier');` |
| `import def from 'x'` | `var def = (m=>m&&m.__esModule?m.default:m)(require('x'));` |
| `import * as ns from 'x'` | `var ns = require('x');` |
| `import { a, b } from 'x'` | `var {a, b} = require('x');` |
| `export default X` | `module.exports = X;` + deferred `module.exports.default = module.exports` |
| `export { a, b } from 'x'` | IIFE re-export via `require('x')` |
| `export * from 'x'` | copies all non-default keys from `require('x')` |
| `export const/let/var X = …` | declaration + `module.exports.X = X` |
| `export var X;` (no initializer) | declaration + deferred export (IIFE fills value first) |
| `export function/class X` | declaration + deferred `module.exports.X = X` |
| `export const { a, b } = X` | destructuring + `module.exports.a = a; module.exports.b = b;` |
| `export const [a, b] = X` | array destructuring + individual exports |
| `export {}` | no-op (OXC ESM marker) |
| `import(specifier)` | `__importAsync(specifier)` (see §2.6.4) |

**Deferred exports** are emitted after all declarations, ensuring TypeScript enum IIFEs, function hoisting, and class definitions complete before `module.exports.*` is populated.

### 2.6.3 Circular Dependency Handling

Before evaluating any module, the runtime pre-caches an empty `module.exports` object:

```javascript
globalThis.__requireCache[resolvedPath] = globalThis.module.exports; // empty {}
// ... eval ...
globalThis.__requireCache[resolvedPath] = result; // final value
```

Circular requires (A→B→A) receive the partially-filled exports object, matching Node.js behavior. Without this, mutual imports would trigger infinite recursion and a stack overflow.

### 2.6.4 Dynamic `import()` Polyfill

Dynamic `import(specifier)` expressions are replaced with `__importAsync(specifier)` at source-load time. The polyfill wraps synchronous `require()` in a resolved Promise and normalises the result as a module namespace object:

```javascript
// Source: const m = await import('./utils');
// Converted: const m = await __importAsync('./utils');

// Implementation (captures calling module's __dirname):
globalThis.__importAsync = function(specifier) {
    var dir = globalThis.__dirname;
    return new Promise(function(resolve, reject) {
        try {
            var mod = globalThis.require(specifier); // synchronous
            resolve(mod && mod.__esModule ? mod : { default: mod, ...mod });
        } catch(e) { reject(e); }
    });
};
```

---

## 2.7 Platform-Aware Extension Resolution

When `require()` cannot find a file exactly as specified, the resolver probes extensions in this priority order:

```
.web.js  →  .web.tsx  →  .web.ts  →  .web.mjs
.js      →  .tsx      →  .ts      →  .mjs      →  .cjs
```

For directory-style imports, index files follow the same order:
```
index.web.js  →  index.web.tsx  →  index.web.ts
index.js      →  index.tsx      →  index.ts
```

**Why `.web.*` first?** Expo and React Native packages ship platform-specific variants (`.native.ts`, `.web.ts`, `.ios.ts`). In the 3va server/CLI environment the web variant is the correct choice — it avoids imports of native modules like `react-native` that have no implementation outside a device. For example:

```
require('./ExpoFileSystem')
  → try ExpoFileSystem.web.ts   ← found: safe web stub
  → (would try ExpoFileSystem.ts, which needs native bridge)
```

This resolution also applies to multi-extension filenames like `setUpJsLogger.fx` → `setUpJsLogger.fx.web.ts`.

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
