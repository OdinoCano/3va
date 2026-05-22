# 03 - POLYFILLS AND SHIMS

## 3.1 Polyfill System

3va implements polyfills for Node.js and browser APIs that are not natively available in QuickJS.

## 3.2 Node.js Polyfills

### 3.2.1 Built-in Modules

| Module | Type | Description |
|--------|------|-------------|
| buffer | Built-in | Binary data buffer |
| console | Built-in | Console (with audit) |
| crypto | Built-in | Basic cryptography |
| events | Built-in | EventEmitter |
| fs | Built-in (partial) | File system |
| http | Built-in (partial) | HTTP client/server |
| os | Built-in | System information |
| path | Built-in | Path utilities |
| process | Built-in | Current process |
| stream | Built-in (partial) | Streams |
| url | Built-in | URL parsing |
| util | Built-in | Utilities |

### 3.2.2 Buffer

```javascript
// Buffer polyfill available globally
const buf = Buffer.from('Hello World');
const buf = Buffer.alloc(8);
const buf = Buffer.allocUnsafe(8);

// Supported encodings
buf.toString('utf8')
buf.toString('base64')
buf.toString('hex')
buf.toString('latin1')

// Methods
Buffer.isBuffer(obj)
Buffer.byteLength(string, encoding)
Buffer.concat(buffers)
```

### 3.2.3 crypto

```javascript
const crypto = require('crypto');

// Hash
const hash = crypto.createHash('sha256');
hash.update('data');
console.log(hash.digest('hex'));

// HMAC
const hmac = crypto.createHmac('sha256', 'key');
hmac.update('data');
console.log(hmac.digest('hex'));

// Random
const bytes = crypto.randomBytes(16);

// Utilities
crypto.createCipheriv()
crypto.createDecipheriv()
crypto.sign()
crypto.verify()
```

### 3.2.4 Events

```javascript
const EventEmitter = require('events');

class MyEmitter extends EventEmitter {}

const emitter = new MyEmitter();

emitter.on('event', (arg) => {
    console.log('event:', arg);
});

emitter.emit('event', { data: 'value' });

// Methods
emitter.on(event, listener)
emitter.once(event, listener)
emitter.off(event, listener)
emitter.emit(event, ...args)
emitter.removeAllListeners(event)
```

## 3.3 Web APIs Polyfills

### 3.3.1 fetch

```javascript
// fetch available globally
const response = await fetch('https://api.example.com/data');
const json = await response.json();

// Request
const req = new Request('/api', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ key: 'value' })
});

// Response
new Response(JSON.stringify(data), {
    status: 200,
    headers: { 'Content-Type': 'application/json' }
});

// AbortController
const controller = new AbortController();
fetch('/api', { signal: controller.signal });
controller.abort();
```

### 3.3.2 TextEncoder/TextDecoder

```javascript
// TextEncoder
const encoder = new TextEncoder();
const encoded = encoder.encode('Hello');  // Uint8Array

// TextDecoder
const decoder = new TextDecoder('utf-8');
const decoded = decoder.decode(encoded);  // "Hello"

// With stream
const transform = encoder.encodeStreaming(data);
```

### 3.3.3 URL and URLSearchParams

```javascript
// URL
const url = new URL('https://user:pass@example.com:8080/path?query=1#hash');

console.log(url.protocol);   // "https:"
console.log(url.host);       // "example.com:8080"
console.log(url.pathname);   // "/path"
console.log(url.search);     // "?query=1"
console.log(url.hash);       // "#hash"
console.log(url.username);   // "user"
console.log(url.password);   // "pass"

// URLSearchParams
const params = new URLSearchParams('foo=bar&baz=qux');
params.get('foo');           // "bar"
params.set('foo', 'new');
params.append('extra', 'value');
params.delete('baz');
```

### 3.3.4 Performance

```javascript
// Performance
const start = performance.now();

// ... code ...

const end = performance.now();
console.log(`Time: ${end - start}ms`);

// Marks and measures
performance.mark('start');
// ... code ...
performance.mark('end');
performance.measure('total', 'start', 'end');

// Navigation timing (partial)
performance.timing;
performance.navigation;
```

## 3.4 Security Polyfills

### 3.4.1 Secure Fetch

```rust
// 3va's fetch implements:
// 1. Permission verification
// 2. URL validation
// 3. Header sanitization
// 4. Response size limits

pub struct SecureFetch {
    permissions: PermissionState,
    max_response_size: usize,
}

impl SecureFetch {
    pub async fn fetch(&self, url: &str, init: RequestInit) -> Result<Response> {
        // Check network permission
        let url_parsed = Url::parse(url)?;
        if !self.permissions.check(&Capability::Network(url_parsed.host_str().unwrap_or(""))) {
            return Err(Error::PermissionDenied);
        }

        // Check URL is not malicious
        if url_parsed.username().is_some() && url_parsed.password().is_some() {
            return Err(Error::SecurityError("URL with embedded credentials"));
        }

        // Fetch with limits
        let response = self.raw_fetch(url, init).await?;

        // Check size
        if let Some(len) = response.content_length() {
            if len > self.max_response_size {
                return Err(Error::ResponseTooLarge);
            }
        }

        Ok(response)
    }
}
```

## 3.5 Polyfill Registration

### 3.5.1 PolyfillRegistry

```rust
pub struct PolyfillRegistry {
    builtins: HashMap<String, Polyfill>,
    globals: HashMap<String, Value>,
}

impl PolyfillRegistry {
    pub fn init(context: &Context) -> anyhow::Result<()> {
        // 1. Load built-in modules in context
        // 2. Define globals
        // 3. Configure resolveHook

        Ok(())
    }

    pub fn register_builtin(&mut self, name: &str, module: Polyfill) {
        self.builtins.insert(name.to_string(), module);
    }
}
```

### 3.5.2 Polyfill Configuration

```javascript
// In JavaScript: disable specific polyfills
3va run app.ts --no-polyfill=fetch
3va run app.ts --no-polyfill=stream
```

---

*Polyfills conforming to Node.js API and WHATWG standards.*
