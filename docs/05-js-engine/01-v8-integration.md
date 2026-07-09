# 01 - V8 INTEGRATION

## 1.1 Overview

3va uses V8 as its JavaScript engine, integrated via the official `v8` Rust crate. V8 is Google's high-performance JavaScript engine, used in Chrome and Node.js, supporting full ES2023 and beyond.

## 1.2 V8 Selection

### 1.2.1 Technical Justification

| Feature | V8 (3va) | JavaScriptCore (Bun) | QuickJS |
|---------|----------|----------------------|---------|
| Binary size | ~20MB | ~15MB | ~1MB |
| Startup time | ~15ms | ~12ms | ~5ms |
| ES2024 support | Yes | Yes | Partial |
| WASM embeddable | Yes | Limited | Native |
| JIT compilation | Yes | Yes | No |
| License | BSD | Apple | MIT |

### 1.2.2 Advantages for 3va

1. **Full JIT compilation**: High performance for compute-intensive workloads
2. **ES2023+ complete support**: No partial feature limitations
3. **WASM support**: Native WebAssembly execution
4. **Mature ecosystem**: Well-tested in production at scale
5. **CDP inspector**: Native DevTools protocol support

## 1.3 Integration with v8

### 1.3.1 Integration Structure

```rust
// crates/js/src/lib.rs
use v8::{HandleScope, Isolate, FunctionCallbackArguments, ReturnValue};
use vvva_permissions::PermissionState;
use vvva_core::Runtime;

pub struct JsEngine {
    isolate: v8::OwnedIsolate,
    context: Option<v8::Global<v8::Context>>,
    runtime_core: Mutex<Runtime>,
    timer_manager: Arc<TimerManager>,
    // ...
}

impl JsEngine {
    pub async fn new(permissions: Arc<PermissionState>) -> anyhow::Result<Self> {
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();

        let mut isolate = Isolate::new(Default::default());
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);

        let runtime_core = Mutex::new(Runtime::new((*permissions).clone()));
        // Builtins injected: console, timers, buffer, process, fetch, fs, websocket, modules
        // ...
        Ok(Self { isolate, context: None, runtime_core, timer_manager })
    }
}
```

### 1.3.2 Lifecycle

```
1. Create V8 platform + initialize
       │
       ▼
2. Create Isolate with memory limits
       │
       ▼
3. Create Context with globalThis
       │
       ▼
4. Inject builtins (console, timers, buffer, process, fetch, fs, modules)
       │
       ▼
5. Load globals and polyfills
       │
       ▼
6. Evaluate code / load modules
       │
       ▼
7. Run event loop (microtasks + timers)
       │
       ▼
8. Collect resources (isolate disposal)
```

### 1.3.3 Execution Context

```rust
pub async fn eval(&mut self, code: &str) -> anyhow::Result<()> {
    let context_global = self.context.clone().expect("engine not initialized");
    let scope = std::pin::pin!(v8::HandleScope::new(&mut *self.isolate));
    let mut scope = scope.init();
    let context = v8::Local::new(&mut scope, &context_global);
    let scope = v8::ContextScope::new(&mut scope, context);

    let source = v8::String::new(&scope, &code).unwrap();
    let script = v8::Script::compile(&scope, source.into(), None)
        .ok_or_else(|| anyhow::anyhow!("compile error"))?;
    let _result = script.run(&scope)
        .ok_or_else(|| anyhow::anyhow!("execution error"))?;
    Ok(())
}
```

---

## 1.4 Memory Limits

Heap limits are configured via V8's isolate parameters:

```rust
let mut isolate = Isolate::new(Default::default());
// V8 heap statistics available via:
// isolate.get_heap_statistics()
```

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

## 1.6 Inspector / Debugger

V8 supports the Chrome DevTools Protocol (CDP) for debugging:

```rust
// crates/js/src/inspector.rs
pub fn start(addr: SocketAddr) -> Option<Arc<InspectorState>> {
    // Starts a WebSocket server accepting CDP connections
    // from Chrome DevTools or compatible debuggers
}
```

---

*Integration via the `v8` crate. Source: `crates/js/src/lib.rs`.*
