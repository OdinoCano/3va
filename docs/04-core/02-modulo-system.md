# 02 - MÓDULOS DEL SISTEMA

## 2.1 Sistema de Módulos

3va soporta tanto ECMAScript Modules (ESM) como CommonJS (CJS), priorizando ESM pero manteniendo compatibilidad con el ecosistema npm.

## 2.2 Módulos Built-in

### 2.2.1 Módulos Core Disponibles

| Módulo | Descripción | Status |
|--------|-------------|--------|
| buffer | Buffer de datos binarios | Implementado |
| console | Consola de salida | Implementado |
| crypto | Criptografía | Parcial |
| events | EventEmitter | Implementado |
| fs | Sistema de archivos | Parcial |
| http | Cliente/servidor HTTP | Parcial |
| https | HTTP con TLS | Por implementar |
| net | TCP/UDP sockets | Por implementar |
| os | Información del sistema | Implementado |
| path | Utilidades de rutas | Implementado |
| process | Proceso actual | Implementado |
| querystring | Parseo de query strings | Implementado |
| stream | Streams de datos | Parcial |
| tls | TLS/SSL | Por implementar |
| url | Parseo de URLs | Implementado |
| util | Utilidades varias | Implementado |
| zlib | Compresión | Por implementar |

### 2.2.2 Implementación de Módulos

#### console
```rust
// Implementación en crates/js/src/builtins/console.rs
pub struct Console;

impl Console {
    pub fn log(&self, args: Vec<Value>) { ... }
    pub fn error(&self, args: Vec<Value>) { ... }
    pub fn warn(&self, args: Vec<Value>) { ... }
    pub fn info(&self, args: Vec<Value>) { ... }
    pub fn debug(&self, args: Vec<Value>) { ... }
    pub fn trace(&self, args: Vec<Value>) { ... }
    pub fn dir(&self, obj: Value) { ... }
    pub fn table(&self, data: Value) { ... }
    pub fn time(&self, label: String) { ... }
    pub fn timeEnd(&self, label: String) { ... }
    pub fn group(&self) { ... }
    pub fn groupEnd(&self) { ... }
}
```

#### buffer
```rust
// Implementación en crates/js/src/builtins/buffer.rs
pub struct Buffer;

impl Buffer {
    pub fn from(data: &[u8]) -> Buffer { ... }
    pub fn to_string(&self, encoding: &str) -> String { ... }
    pub fn write(&mut self, data: &[u8]) -> usize { ... }
    pub fn concat(&self, buffers: &[Buffer]) -> Buffer { ... }
}
```

#### events
```rust
// Implementación en crates/js/src/builtins/events.rs
pub struct EventEmitter {
    listeners: HashMap<String, Vec<Function>>,
}

impl EventEmitter {
    pub fn on(&mut self, event: String, handler: Function) { ... }
    pub fn once(&mut self, event: String, handler: Function) { ... }
    pub fn emit(&mut self, event: String, args: Vec<Value>) -> bool { ... }
    pub fn off(&mut self, event: String, handler: Option<Function>) { ... }
    pub fn removeAllListeners(&mut self, event: Option<String>) { ... }
}
```

## 2.3 Carga de Módulos

### 2.3.1 Resolution Algorithm

```
1. Si es URL absoluta o relativa:
   - Resolver contra __dirname
2. Si es package lookup:
   - Buscar en node_modules
   - Resolver main en package.json
   - Buscar index.js, index.ts
3. Si es built-in:
   - Devolver módulo nativo
4. Si no se encuentra:
   - Throw MODULE_NOT_FOUND
```

### 2.3.2 Mapeo de Extensiones

| Extensión | Acción |
|-----------|--------|
| .mjs | Tratar como ESM |
| .cjs | Tratar como CJS |
| .js | Según package.json type |
| .ts | Transpilar a JS |
| .tsx | Transpilar a JSX |
| .jsx | Transpilar a JS |

### 2.3.3 Cache de Módulos

```rust
pub struct ModuleCache {
    modules: HashMap<PathBuf, Module>,
    esm_cache: HashMap<Url, Module>,
}

impl ModuleCache {
    pub fn get(&self, key: &ModuleKey) -> Option<&Module> { ... }
    pub fn set(&mut self, key: ModuleKey, module: Module) { ... }
    pub fn clear(&mut self) { ... }
}
```

## 2.4 CommonJS

### 2.4.1 Implementación de require()

```javascript
// En entorno 3va
const fs = require('fs');
const _ = require('lodash');

// Con exports
module.exports = { foo: 'bar' };
module.exports.foo = 'bar';
```

### 2.4.2 Wrapping de Módulos

Todo código CJS se envuelve en:
```javascript
(function(exports, require, module, __filename, __dirname) {
    // código del usuario
})(exports, require, module, filename, dirname);
```

## 2.5 ESM

### 2.5.1 Import/Export

```javascript
// Named imports
import { foo, bar } from './module';

// Default import
import defaultExport from './module';

// Namespace import
import * as namespace from './module';

// Named exports
export const foo = 'bar';
export function test() { }

// Default export
export default function() { }
```

### 2.5.2 Top-level await

```javascript
// Disponible en módulos ESM
const data = await fetch('/api/data');
export default data;
```

## 2.6 Package.json Integration

### 2.6.1 Resolution de main

```json
{
  "main": "dist/index.js",
  "exports": {
    ".": "./dist/index.js",
    "./feature": "./dist/feature.js"
  }
}
```

### 2.6.2 Exports Conditional

```json
{
  "exports": {
    ".": {
      "import": "./dist/esm/index.mjs",
      "require": "./dist/cjs/index.js"
    }
  }
}
```

---

*Módulos conforme a especificaciones Node.js y estándares ECMAScript.*