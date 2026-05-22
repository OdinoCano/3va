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
use rquickjs::{Context, Runtime, Module, Value};
use vvva_permissions::PermissionState;

pub struct JsEngine {
    runtime: Runtime,
    context: Context,
    module_loader: ModuleLoader,
    polyfills: PolyfillRegistry,
}

impl JsEngine {
    pub fn new(permissions: &PermissionState) -> anyhow::Result<Self> {
        // 1. Create QuickJS runtime
        let runtime = Runtime::new();

        // 2. Set memory limit (default: 256MB)
        runtime.set_memory_limit(256 * 1024 * 1024);

        // 3. Create context
        let context = Context::full(&runtime)?;

        // 4. Initialize modules and polyfills
        let module_loader = ModuleLoader::new(permissions);
        let polyfills = PolyfillRegistry::new();

        Ok(Self {
            runtime,
            context,
            module_loader,
            polyfills,
        })
    }
}
```

### 1.3.2 Lifecycle

```
1. Create Runtime
       │
       ▼
2. Configure limits (memory, stack)
       │
       ▼
3. Create Context
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

## 1.4 Memory Management

### 1.4.1 Memory Limits

```rust
// Memory limits configuration
pub struct MemoryLimits {
    pub heap_max: usize,      // 256MB default
    pub stack_limit: usize,   // 1MB default
    pub memory_warning: usize, // 80% of maximum
}

impl Default for MemoryLimits {
    fn default() -> Self {
        Self {
            heap_max: 256 * 1024 * 1024,
            stack_limit: 1024 * 1024,
            memory_warning: 256 * 1024 * 1024 / 10 * 8,
        }
    }
}
```

### 1.4.2 Exceeded Memory Handling

```rust
// Callback when memory is exceeded
runtime.set_memory_limit_callback(|| {
    // Options:
    // 1. Force GC
    // 2. Throw error
    // 3. Terminate process
    rquickjs::Error::new_error("Memory limit exceeded")
});
```

## 1.5 thread_isolate (WASM)

### 1.5.1 WebAssembly Isolation

```rust
// For future WASM-first support
pub struct WasmIsolate {
    runtime: Runtime,
    isolate: Isolates,
}

impl WasmIsolate {
    pub fn new() -> Self {
        // QuickJS can be compiled to WASM
        // allowing multiple isolates in the same process
    }

    pub fn spawn(&self) -> WasmInstance {
        // Create new isolated instance
    }
}
```

## 1.6 Options Configuration

### 1.6.1 Runtime Options

```rust
let runtime = Runtime::new()
    .set_memory_limit(512 * 1024 * 1024)  // 512MB
    .set_max_stack_size(2 * 1024 * 1024) // 2MB
    .set_unhandled_promise_rejection_mode(
        UnhandledPromiseRejection::Throw
    )
    .set_optimizer(false)  // Disable optimizer for debugging
    .set_strict(true);    // Strict mode by default
```

---

*Integration conforming to rquickjs and QuickJS documentation.*
