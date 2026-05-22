# 03 - POLYFILLS AND SHIMS

## 3.1 Overview

3va registers Node.js-compatible built-ins and JS stubs at engine startup so that packages can `require()` them without throwing. There is no `PolyfillRegistry` type; stubs are injected inline from `crates/js/src/builtins/modules.rs`.

For the complete status table see [04-core/02-modulo-system.md](../04-core/02-modulo-system.md).

## 3.2 Rust Builtins (full implementations)

### Buffer (global + `require('buffer')`)

```javascript
const buf = Buffer.from('Hello World');
const buf2 = Buffer.alloc(8);
buf.toString('utf8');
buf.toString('base64');
buf.toString('hex');
Buffer.concat([buf, buf2]);
Buffer.isBuffer(buf);   // true
```

### `fs` (`require('fs')`)

Permission-gated (`--allow-read` / `--allow-write`).

```javascript
const fs = require('fs');
fs.readFileSync('/path/file', 'utf8');
fs.writeFileSync('/path/file', 'content');
fs.existsSync('/path/file');
fs.mkdirSync('/path/dir', { recursive: true });
fs.readdirSync('/path/dir');
fs.statSync('/path/file');
fs.unlinkSync('/path/file');
```

### `events` (`require('events')`)

Full JS EventEmitter:

```javascript
const EventEmitter = require('events');

class MyEmitter extends EventEmitter {}
const emitter = new MyEmitter();

emitter.on('data', (chunk) => console.log(chunk));
emitter.once('end', () => console.log('done'));
emitter.emit('data', 'hello');
emitter.off('data', handler);
emitter.removeAllListeners('data');
```

## 3.3 JS Stubs (compatibility shims)

These allow packages to `require()` the module without throwing, but provide minimal or no real I/O.

### `crypto` (minimal stub)

```javascript
const crypto = require('crypto');

// randomBytes — uses Math.random(), NOT cryptographically secure
crypto.randomBytes(16);   // Uint8Array

// createHash — returns base64 of input, NOT a real hash
const h = crypto.createHash('sha256');
h.update('data');
h.digest('hex');  // returns btoa(data), not SHA-256

// createHmac — returns empty string
const hmac = crypto.createHmac('sha256', 'key');
hmac.update('data').digest('hex');  // ''
```

**Note:** `crypto.createHash` and `crypto.createHmac` are stubs — they do not produce correct cryptographic output. Use the `vvva_crypto` Rust crate for real hashing.

### `http` / `https` (stub)

`request()` and `get()` return a no-op emitter. No real HTTP is performed.

### `net` / `tls` (stub)

`createConnection()` returns an emitter. No real TCP connection.

### `zlib` (stub)

`gzip`/`gunzip`/`deflate`/`inflate` callbacks fire synchronously with identity (unchanged data).

### `child_process` (stub)

`exec(cmd, cb)` calls `cb(null, '', '')`. No real process is spawned.

### `perf_hooks`

```javascript
const { performance } = require('perf_hooks');
performance.now();  // Date.now() — millisecond precision, not high-resolution
```

## 3.4 `fetch` (global, Rust implementation)

```javascript
const res = await fetch('https://api.example.com/data');
const json = await res.json();
```

Permission-gated (`--allow-net`). Implemented in `builtins/fetch.rs` via `ureq`. Does not support `AbortController`.

## 3.5 `TextEncoder` / `TextDecoder` (global)

```javascript
const enc = new TextEncoder();
const bytes = enc.encode('hello');   // Uint8Array, UTF-8

const dec = new TextDecoder();
const str = dec.decode(bytes);       // 'hello'
```

---

## 3.6 Planned Polyfills (not yet implemented)

> **Status: PENDING** — these APIs appear in the compatibility roadmap. Using them currently throws `ReferenceError` or silently no-ops.

### `AbortController` / `AbortSignal`

```javascript
// PLANNED
const controller = new AbortController();
fetch('/api', { signal: controller.signal });
controller.abort(); // fires 'abort' on signal
```

### `ReadableStream` / `WritableStream` / `TransformStream`

```javascript
// PLANNED — streams pipeline
const readable = new ReadableStream({ start(c) { c.enqueue('chunk'); c.close(); } });
const writable = new WritableStream({ write(chunk) { console.log(chunk); } });
await readable.pipeTo(writable);
```

### `Blob` / `File` / `FileReader`

```javascript
// PLANNED
const blob = new Blob(['content'], { type: 'text/plain' });
const text = await blob.text();

const reader = new FileReader();
reader.onload = (e) => console.log(e.target.result);
reader.readAsText(blob);
```

### `URL` / `URLSearchParams` (global constructors)

Currently accessible only via `require('url')` and `require('querystring')`. Planned as global constructors:

```javascript
// PLANNED as globals (currently: require('url').parse())
const url = new URL('https://example.com/path?q=1');
const params = new URLSearchParams('foo=bar&baz=qux');
params.get('foo'); // 'bar'
```

### Real `crypto` (Web Crypto API)

The current `crypto` stub uses `Math.random()` and `btoa()`. Full Web Crypto API planned for v0.3.0:

```javascript
// PLANNED — requires ml-kem + ml-dsa crates
const bytes = crypto.getRandomValues(new Uint8Array(16)); // real CSPRNG
const hash = await crypto.subtle.digest('SHA-256', data);
```

---

*Rust builtins in `crates/js/src/builtins/`. JS stubs in `crates/js/src/builtins/modules.rs`.*
