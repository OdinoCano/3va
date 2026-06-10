# 03 - GLOBAL OBJECTS

## 3.1 JavaScript Environment Globals

3va exposes a set of global objects compatible with Node.js and browsers, following the ECMAScript specification and standard web APIs.

## 3.2 Standard ECMAScript Globals

### 3.2.1 Core Objects

| Global | Description |
|--------|-------------|
| Object | Object constructor |
| Function | Function constructor |
| Array | Array constructor |
| Boolean | Boolean constructor |
| Number | Number constructor |
| BigInt | Big integers |
| String | String constructor |
| Symbol | Unique symbols |
| Date | Dates |
| RegExp | Regular expressions |
| Error | Error class |
| Map | Key-value collection |
| Set | Unique values collection |
| WeakMap | Weak Map |
| WeakSet | Weak Set |
| ArrayBuffer | Binary buffer |
| Promise | Promises |
| Proxy | Metaprogramming |
| Reflect | Reflection |

### 3.2.2 Global Functions

| Function |
|----------|
| eval() |
| isFinite() |
| isNaN() |
| parseFloat() |
| parseInt() |
| decodeURI() |
| encodeURI() |
| decodeURIComponent() |
| encodeURIComponent() |

### 3.2.3 Type Constructors

```javascript
// Typed Arrays
Int8Array, Uint8Array, Uint8ClampedArray
Int16Array, Uint16Array
Int32Array, Uint32Array
Float32Array, Float64Array
BigInt64Array, BigUint64Array

// Structured Clone
SharedArrayBuffer
Atomics
```

## 3.3 Node.js Globals

### 3.3.1 Node Objects

```javascript
// Process
process          // global process object
global           // global namespace

// Console — full Node.js API
console.log / console.warn / console.error / console.info / console.debug
console.trace    // stack trace
console.dir      // object inspection
console.table    // tabular output
console.time / console.timeEnd / console.timeLog   // timers
console.group / console.groupCollapsed / console.groupEnd  // nesting
console.count / console.countReset  // counters
console.assert   // conditional log
console.clear    // clear output

// Timers (functions)
setTimeout       // execute after delay
setInterval      // execute repeatedly
setImmediate     // execute in next phase
clearTimeout     // cancel timeout
clearInterval    // cancel interval
clearImmediate   // cancel immediate

// Modules
module           // current module
exports          // module exports
require          // require function
__dirname        // directory of current module
__filename       // filename of current module
```

### 3.3.2 Global Buffer

```javascript
// Global buffer
Buffer           // Buffer class available globally

// Create buffer
Buffer.from('hello')      // from string
Buffer.alloc(8)            // allocation
Buffer.allocUnsafe(8)      // uninitialized
```

### 3.3.3 URL and Utils

```javascript
// URL
URL              // URL constructor
URLSearchParams  // Query parameters

// Utilities
```

## 3.4 Compatible Web APIs

### 3.4.1 fetch API

```javascript
// fetch polyfill (implemented in QuickJS)
const response = await fetch('https://api.example.com/data');
const data = await response.json();

// Options
await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key: 'value' }),
});
```

### 3.4.2 Web Globals (implemented)

| API | Status | Notes |
|-----|--------|-------|
| `TextEncoder` / `TextDecoder` | ✅ Implemented | UTF-8; global |
| `fetch` | ✅ Implemented | Requires `--allow-net` |
| `WebSocket` | ✅ Implemented | Requires `--allow-net` |
| `AbortController` / `AbortSignal` | ✅ Implemented | Full signal + timeout + abort |
| `ReadableStream` / `WritableStream` / `TransformStream` | ✅ Implemented | WHATWG Streams standard |
| `FormData` | ✅ Implemented | append/set/get/delete/forEach/entries |
| `Blob` / `File` | ✅ Implemented | `.text()`, `.arrayBuffer()`, `.bytes()`, `.stream()`, `.slice()` |
| `FileReader` | ✅ Implemented | `readAsText`, `readAsDataURL`, `readAsArrayBuffer` |
| `URLSearchParams` | ✅ Implemented | Standalone constructor; also via `require('url')` |
| `URL` | ✅ Implemented | Full WHATWG URL; also via `require('url')` |
| `Headers` / `Request` / `Response` | ✅ Implemented | Standalone constructors + `fetch` integration |
| `atob` / `btoa` | ✅ Implemented | Global base64 encode/decode |
| `crypto.getRandomValues` | ✅ Implemented | CSPRNG via Rust |
| `crypto.subtle` | ✅ Implemented | SHA-256/384/512, AES-GCM, HMAC, ECDH, RSA-OAEP |

## 3.5 React Native / Expo Globals

3va sets up a React Native environment so Expo packages load without a device or bundler.

### 3.5.1 Environment flags

```javascript
globalThis.__REACT_NATIVE__ = true   // signals an RN host environment
globalThis.__DEV__           = false  // production mode
process.env.EXPO_OS          = 'web'  // Expo web/server platform identifier
```

### 3.5.2 Platform

```javascript
Platform.OS      // 'web'
Platform.Version // '1'
Platform.select({ web: 'a', native: 'b', default: 'c' })  // 'a'
Platform.isPad   // false
Platform.isTV    // false
```

`Platform` is available both as a global and via `require('react-native').Platform`. Expo's `Platform.select()` uses it to choose branch values.

### 3.5.3 NativeModules proxy

```javascript
// Pre-registered modules (ExpoConstants, ExpoFileSystem, ExpoFont, etc.):
typeof NativeModules.ExpoConstants   // 'object'
NativeModules.ExpoConstants.anyMethod()  // undefined (proxy)

// Unregistered modules:
NativeModules.EXDevLauncher  // undefined  ← important: NOT a truthy proxy
```

The proxy returns `undefined` for unknown module names so that truthiness guards like `if (NativeModules.EXDevLauncher)` correctly skip the block.

### 3.5.4 requestAnimationFrame / cancelAnimationFrame

```javascript
const id = requestAnimationFrame((timestamp) => { /* ... */ });
cancelAnimationFrame(id);
```

Backed by `setTimeout(fn, 16)`. Provides a 60fps-compatible callback cycle for animation or frame-based logic.

---

*Globals conforming to Node.js API and WHATWG standards. For full module status see [04-core/02-modulo-system.md](02-modulo-system.md).*
