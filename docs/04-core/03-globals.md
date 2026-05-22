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

// Console
console          // console object

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

### 3.4.2 Web APIs

| API | Description |
|-----|-------------|
| AbortController | Abort control |
| AbortSignal | Abort signal |
| BroadcastChannel | Cross-tab communication |
| Crypto | Cryptographic operations |
| CryptoKey | Cryptographic keys |
| Performance | Performance measurement |
| PerformanceEntry | Performance entry |
| PerformanceMark | Performance mark |
| PerformanceMeasure | Performance measure |
| TextEncoder | Text encoder |
| TextDecoder | Text decoder |
| TransformStream | Transform streams |
| ReadableStream | Readable streams |
| WritableStream | Writable streams |
| Headers | HTTP headers |
| Request | HTTP request |
| Response | HTTP response |
| FormData | Form data |
| URLSearchParams | URL parameters |
| WebSocket | WebSockets |

## 3.5 Security Polyfills

### 3.5.1 Verified Fetch

```rust
// 3va's fetch includes permission verification
pub async fn secure_fetch(url: &str, options: RequestInit) -> Result<Response> {
    // 1. Check network permission
    if !permissions.check(&Capability::Network(url)) {
        return Err(Error::PermissionDenied);
    }
    // 2. Validate URL
    let parsed = Url::parse(url)?;
    validate_no_malicious_redirect(&parsed)?;
    // 3. Execute fetch
    let response = native_fetch(url, options).await?;
    // 4. Validate response
    validate_response(&response)?;
    Ok(response)
}
```

### 3.5.2 Console with Audit Logging

```rust
// Console writes to audit log
pub fn log(&self, level: Level, args: Vec<Value>) {
    // Normal output
    self.output.write(args);
    // Audit log
    audit::log(AuditEvent {
        event_type: "console".to_string(),
        level: level.to_string(),
        timestamp: now(),
        data: args.clone(),
    });
}
```

Globals conforming to Node.js API and WHATWG standards.
