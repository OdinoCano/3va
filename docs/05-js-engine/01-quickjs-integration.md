# 01 - INTEGRACIÓN CON QUICKJS

## 1.1 Descripción General

3va utiliza QuickJS como motor JavaScript, integrado mediante la librería `rquickjs` de Rust. QuickJS es una implementación ligera y rápida de JavaScript escrita por Fabrice Bellard, con licencia MIT.

## 1.2 Selección de QuickJS

### 1.2.1 Justificación Técnica

| Característica | V8 (Node) | JavaScriptCore (Bun) | QuickJS (3va) |
|----------------|-----------|----------------------|---------------|
| Tamaño binario | ~30MB | ~15MB | ~1MB |
| Tiempo de inicio | ~25ms | ~12ms | ~5ms |
| Soporte ES2024 | Sí | Sí | Parcial |
| WASM embeddable | Limitado | Limitado | Nativo |
| Threads isolates | Limitado | Sí | Sí (limitado) |
| Licencia | BSD | Apple | MIT |

### 1.2.2 Ventajas para 3va

1. **Licencia MIT**: Compatible con distribución comercial
2. **WASM nativo**: Fácil compilación a WebAssembly
3. **Código fuente pequeño**: Fácil auditoría de seguridad
4. **Sin dependencias externas**: Binario autocontenido
5. **Rápido inicio**: Ideal para serverless/edge

### 1.2.3 Limitaciones y Mitigaciones

| Limitación | Mitigación |
|------------|------------|
| GC menos sofisticado | Transpilación optimizada |
| Sin JIT | Pre-compilación de módulos frecuentes |
| ES2024 parcial | Polyfills selectivos |

## 1.3 Integración con rquickjs

### 1.3.1 Estructura de Integración

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
        // 1. Crear runtime de QuickJS
        let runtime = Runtime::new();

        // 2. Configurar límite de memoria (default: 256MB)
        runtime.set_memory_limit(256 * 1024 * 1024);

        // 3. Crear contexto
        let context = Context::full(&runtime)?;

        // 4. Inicializar módulos y polyfills
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

### 1.3.2 Ciclo de Vida

```
1. Crear Runtime
       │
       ▼
2. Configurar límites (memoria, stack)
       │
       ▼
3. Crear Context
       │
       ▼
4. Cargar globals y polyfills
       │
       ▼
5. Evaluar código / cargar módulos
       │
       ▼
6. Recolectar recursos
       │
       ▼
7. Destruir Context
       │
       ▼
8. Destruir Runtime
```

### 1.3.3 Contexto de Ejecución

```rust
pub fn eval(&self, code: &str) -> anyhow::Result<Value> {
    self.context.with(|ctx| {
        // Evaluar código en el contexto
        let result = ctx.eval(code)?;
        Ok(result)
    })
}

pub fn eval_module(&self, code: &str, path: &str) -> anyhow::Result<Value> {
    self.context.with(|ctx| {
        // Crear módulo desde código
        let module = ctx.compile(path, code)?;
        // Evaluar módulo (para efectos secundarios)
        module.evaluate()?;
        // Devolver exports
        module.get("default").unwrap_or(Value::undefined())
    })
}
```

## 1.4 Gestión de Memoria

### 1.4.1 Límites de Memoria

```rust
// Configuración de límites de memoria
pub struct MemoryLimits {
    pub heap_max: usize,      // 256MB default
    pub stack_limit: usize,   // 1MB default
    pub memory_warning: usize, // 80% del máximo
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

### 1.4.2 Manejo de Memoria Excedida

```rust
// Callback cuando se excede la memoria
runtime.set_memory_limit_callback(|| {
    // Opciones:
    // 1. Forzar GC
    // 2. Throwing error
    // 3. Terminar proceso
    rquickjs::Error::new_error("Memory limit exceeded")
});
```

## 1.5 thread_isolate (WASM)

### 1.5.1 Aislamiento para WebAssembly

```rust
// Para futuro soporte WASM-first
pub struct WasmIsolate {
    runtime: Runtime,
    isolate: Isolates,
}

impl WasmIsolate {
    pub fn new() -> Self {
        // QuickJS puede compilarse a WASM
        // permitiendo múltiples isolate en el mismo proceso
    }

    pub fn spawn(&self) -> WasmInstance {
        // Crear nueva instancia aislada
    }
}
```

## 1.6 Configuración de Opciones

### 1.6.1 Opciones del Runtime

```rust
let runtime = Runtime::new()
    .set_memory_limit(512 * 1024 * 1024)  // 512MB
    .set_max_stack_size(2 * 1024 * 1024) // 2MB
    .set_unhandled_promise_rejection_mode(
        UnhandledPromiseRejection::Throw
    )
    .set_optimizer(false)  // Desactivar optimize para debugging
    .set_strict(true);    // Modo estricto por defecto
```

---

*Integración conforme a documentación de rquickjs y QuickJS.*