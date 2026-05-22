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

## 1.4 Planned — Memory Limits

> **Status: PENDING** — memory and stack limits are not yet configured in `JsEngine::new()`.

Planned configuration:

```rust
// PLANNED — not yet applied
runtime.set_memory_limit(256 * 1024 * 1024);  // 256 MB heap
runtime.set_max_stack_size(1 * 1024 * 1024);  // 1 MB stack
```

QuickJS will throw `InternalError: out of memory` if the heap is exhausted. When the limit API is integrated, 3va will catch this and return a structured error instead of panicking.

## 1.5 Planned — WASM Compilation Target

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
