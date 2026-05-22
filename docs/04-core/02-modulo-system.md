# 02 - SYSTEM MODULES

## 2.1 Module System

3va supports both ECMAScript Modules (ESM) and CommonJS (CJS), prioritizing ESM while maintaining compatibility with the npm ecosystem.

## 2.2 Built-in Modules

### 2.2.1 Available Core Modules

| Module | Description | Status |
|--------|-------------|--------|
| buffer | Binary data buffer | Implemented |
| console | Output console | Implemented |
| crypto | Cryptography | Partial |
| events | EventEmitter | Implemented |
| fs | File system | Partial |
| http | HTTP client/server | Partial |
| https | HTTP with TLS | To implement |
| net | TCP/UDP sockets | To implement |
| os | System information | Implemented |
| path | Path utilities | Implemented |
| process | Current process | Implemented |
| querystring | Query string parsing | Implemented |
| stream | Data streams | Partial |
| tls | TLS/SSL | To implement |
| url | URL parsing | Implemented |
| util | Various utilities | Implemented |
| zlib | Compression | To implement |

### 2.2.2 Module Implementation

#### console
```rust
// Implementation in crates/js/src/builtins/console.rs
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
// Implementation in crates/js/src/builtins/buffer.rs
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
// Implementation in crates/js/src/builtins/events.rs
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

## 2.3 Module Loading

### 2.3.1 Resolution Algorithm

```
1. If absolute or relative URL:
   - Resolve against __dirname
2. If package lookup:
   - Search in node_modules
   - Resolve main in package.json
   - Search index.js, index.ts
3. If built-in:
   - Return native module
4. If not found:
   - Throw MODULE_NOT_FOUND
```

### 2.3.2 Extension Mapping

| Extension | Action |
|-----------|--------|
| .mjs | Treat as ESM |
| .cjs | Treat as CJS |
| .js | According to package.json type |
| .ts | Transpile to JS |
| .tsx | Transpile to JSX |
| .jsx | Transpile to JS |

### 2.3.3 Module Cache

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

### 2.4.1 require() Implementation

```javascript
// In 3va environment
const fs = require('fs');
const _ = require('lodash');

// With exports
module.exports = { foo: 'bar' };
module.exports.foo = 'bar';
```

### 2.4.2 Module Wrapping

All CJS code is wrapped in:
```javascript
(function(exports, require, module, __filename, __dirname) {
    // user code
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
// Available in ESM modules
const data = await fetch('/api/data');
export default data;
```

## 2.6 Package.json Integration

### 2.6.1 main Resolution

```json
{
  "main": "dist/index.js",
  "exports": {
    ".": "./dist/index.js",
    "./feature": "./dist/feature.js"
  }
}
```

### 2.6.2 Conditional Exports

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

*Modules conforming to Node.js specifications and ECMAScript standards.*
