# 01 - QUICKJS INTEGRATION

## 1.1 Overview

3va uses QuickJS as its JavaScript engine, integrated via the `rquickjs` Rust library. QuickJS is a lightweight and fast JavaScript implementation written by Fabrice Bellard, licensed under MIT.

## 1.2 QuickJS Selection

### 1.2.1 Technical Justification

| Feature | V8 (Node) | JavaScriptCore (Bun) | QuickJS (3va) |
|---------|-----------|----------------------|---------------|
| Binary size | ~30MB | ~15MB | ~1MB |
| Startup time | ~25ms | ~12ms | ~5ms |
| ES2024 support | Yes | Yes | Partial |
| WASM embeddable | Limited | Limited | Native |
| Thread isolates | Limited | Yes | Yes (limited) |
| License | BSD | Apple | MIT |

### 1.2.2 Advantages for 3va

1. **MIT License**: Compatible with commercial distribution
2. **Native WASM**: Easy compilation to WebAssembly
3. **Small source code**: Easy security audit
4. **No external dependencies**: Self-contained binary
5. **Fast startup**: Ideal for serverless/edge

### 1.2.3 Limitations and Mitigations

| Limitation | Mitigation |
|------------|------------|
| Less sophisticated GC | Optimized transpilation |
| No JIT | Pre-compilation of frequent modules |
| Partial ES2024 | Selective polyfills |

## 1.3 Integration with rquickjs

### 1.3.1 Integration Structure

```rust
// crates/js/src/lib.rs
use rquickjs::{AsyncRuntime, AsyncContext};
use vvva_permissions::PermissionState;
use vvva_core::Runtime;

pub struct JsEngine {
    runtime: AsyncRuntime,   // rquickjs async runtime
    context: AsyncContext,   // rquickjs async context
    runtime_core: Mutex<Runtime>, // vvva_core Runtime for TimerWheel + TaskQueue
    // permissions: Arc<PermissionState> — held internally
}

impl JsEngine {
    pub async fn new(permissions: Arc<PermissionState>) -> anyhow::Result<Self> {
        let runtime = AsyncRuntime::new()?;
        let runtime_core = Mutex::new(Runtime::new((*permissions).clone()));
        // ESM loader + resolver registered on the runtime
        runtime.set_loader(EsmResolver, EsmLoader { permissions: permissions.clone() }).await;
        let context = AsyncContext::full(&runtime).await?;
        // Builtins injected: console, timers, buffer, process, fetch, fs, websocket, modules
        // ...
        Ok(Self { runtime, context, runtime_core })
    }
}
```

### 1.3.2 Lifecycle

```
1. Create AsyncRuntime + AsyncContext
       │
       ▼
2. Register EsmResolver + EsmLoader
       │
       ▼
3. Inject builtins (console, timers, buffer, process, fetch, fs, modules)
       │
       ▼
4. Load globals and polyfills
       │
       ▼
5. Evaluate code / load modules
       │
       ▼
6. Collect resources
       │
       ▼
7. Destroy Context
       │
       ▼
8. Destroy Runtime
```

### 1.3.3 Execution Context

```rust
pub fn eval(&self, code: &str) -> anyhow::Result<Value> {
    self.context.with(|ctx| {
        // Evaluate code in context
        let result = ctx.eval(code)?;
        Ok(result)
    })
}

pub fn eval_module(&self, code: &str, path: &str) -> anyhow::Result<Value> {
    self.context.with(|ctx| {
        // Create module from code
        let module = ctx.compile(path, code)?;
        // Evaluate module (for side effects)
        module.evaluate()?;
        // Return exports
        module.get("default").unwrap_or(Value::undefined())
    })
}
```

---

## 1.4 Memory Limits

Heap and GC limits are configured in `JsEngine::new()` (`crates/js/src/lib.rs`):

```rust
runtime.set_memory_limit(256 * 1024 * 1024).await;   // 256 MB heap
runtime.set_gc_threshold(204 * 1024 * 1024).await;    // GC triggers at 80% (≈204 MB)
```

QuickJS throws `InternalError: out of memory` when the heap limit is reached. Stack size is unbounded by default; a configurable stack limit is planned.

## 1.5 Transpiler (Oxc)

Source: `crates/js/src/transpiler.rs`. Backed by the [Oxc](https://oxc.rs/) toolchain.

### 1.5.1 Entry points

| Function | Input | Output |
|----------|-------|--------|
| `transpile(src)` | TypeScript (no JSX) | JavaScript with types stripped |
| `transpile_jsx(src)` | TypeScript or JavaScript with JSX | JavaScript, JSX → `React.createElement` |
| `transpile_js(src)` | Plain JS that may contain JSX or Flow | JavaScript (with automatic fallbacks) |
| `looks_like_jsx(src) -> bool` | Any source | `true` if `<Tag` or `</Tag` is detected |

### 1.5.2 Automatic dispatch in `eval_file` and `require()`

| File extension | Function called |
|----------------|-----------------|
| `.tsx`, `.jsx` | `transpile_jsx()` |
| `.ts`, `.mts`, `.cts` | `transpile()` |
| `.js`, `.mjs`, `.cjs`, others | `transpile_js()` if `looks_like_jsx()` returns true; otherwise source is used as-is |

### 1.5.3 JSX transform

Uses the **Classic runtime** (`React.createElement`):

```javascript
// Input (.jsx or .js with JSX)
const el = <View style={{ flex: 1 }}><Text>hello</Text></View>;

// Output
const el = React.createElement(View, { style: { flex: 1 } },
  React.createElement(Text, null, "hello")
);
```

Fragments use `React.Fragment`:
```javascript
const el = <><div /><span /></>;
// → React.createElement(React.Fragment, null, React.createElement("div", null), React.createElement("span", null))
```

### 1.5.4 Flow type stripping (best-effort)

`transpile_js()` applies a two-pass Flow fallback when Oxc cannot parse a `.js` file:

1. **`strip_flow()`** — removes `@flow`/`@format` pragmas, `import type …`, `import typeof …` statements at character level (no regex).
2. **`strip_inline_flow_types()`** — removes `: Type` annotations from `const`/`let`/`var` declarations and function parameters by scanning at character level with brace-depth tracking.

This enables basic loading of Flow-annotated files from React Native packages. Complex Flow generics, `opaque type`, and `declare module` blocks are **not** handled — use a full Babel/Metro pipeline for React Native.

### 1.5.5 Limitations

| Feature | Status |
|---------|--------|
| TypeScript type stripping | ✅ Full |
| JSX Classic transform | ✅ Full |
| JSX Automatic transform (`react/jsx-runtime`) | ❌ Not configured (use Classic) |
| Flow basic annotations | ✅ Best-effort via `strip_flow` |
| Flow opaque types / declare module | ❌ Not supported |
| Decorators (`@Injectable`) | ⚠️ Partial (depends on Oxc version) |
| `import.meta` | ✅ Preserved as-is |

---

## 1.6 Planned — WASM Compilation Target

> **Status: FUTURE** — no implementation started.

QuickJS itself can be compiled to WASM, which would allow 3va to run inside a browser sandbox or Cloudflare Workers. This requires a separate build profile and JS binding layer. Planned for after the QuickJS+Tokio event loop integration stabilizes.

```rust
// FUTURE — architecture target only, not a real type yet
// pub struct WasmIsolate { ... }
// Each WASM instance gets its own QuickJS runtime
// allowing multiple isolated contexts in the same process.
```

---

*Integration via the `rquickjs` crate. Source: `crates/js/src/lib.rs`.*
