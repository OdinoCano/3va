# 04 - WEB APIs

## 4.1 Overview

3va targets [WinterCG Minimum Common Web Platform API](https://common-min-api.proposal.wintercg.org/) compatibility. The table below reflects what is **actually implemented**.

## 4.2 API Status

| API | Status | Notes |
|-----|--------|-------|
| `fetch` | Implemented | `builtins/fetch.rs` — accepts `Request` or URL string; returns `Response`; requires `--allow-net` |
| `Request` | Implemented | `modules.rs`; full WinterCG constructor, all body methods, `clone()` |
| `Response` | Implemented | `modules.rs`; full WinterCG constructor, `Response.json()`, `Response.error()`, `Response.redirect()` |
| `Headers` | Implemented | `modules.rs`; case-insensitive, iterable (`entries()`/`keys()`/`values()`/`forEach()`/`for..of`) |
| `WebSocket` | Implemented | `builtins/websocket.rs`; requires `--allow-net` |
| `TextEncoder` / `TextDecoder` | Implemented | `builtins/buffer.rs`; UTF-8 only |
| `setTimeout` / `setInterval` | Implemented | `builtins/timers.rs` + `TimerWheel` |
| `clearTimeout` / `clearInterval` | Implemented | `builtins/timers.rs` |
| `console` | Implemented | `builtins/console.rs` |
| `Buffer` | Implemented | `builtins/buffer.rs`; Node.js-compatible |
| `process` | Implemented | `builtins/process.rs` |
| `AbortController` / `AbortSignal` | Implemented | `modules.rs`; integrates with `fetch` via `Promise.race` |
| `Blob` / `File` | Implemented | `modules.rs`; `text()`, `arrayBuffer()`, `bytes()`, `slice()`, `stream()` |
| `FormData` | Implemented | `modules.rs`; `append`, `set`, `get`, `getAll`, `delete`, `forEach`, iteration |
| `ReadableStream` | Implemented | `modules.rs`; pull model with `getReader`, `pipeTo`, `pipeThrough`, `tee` |
| `WritableStream` | Implemented | `modules.rs`; `getWriter`, `write`, `close` |
| `TransformStream` | Implemented | `modules.rs`; `readable` + `writable` pair with shared controller |
| `URL` / `URLSearchParams` | Implemented | `modules.rs`; full parsing, relative resolution, `canParse()`, iteration |
| `FileReader` | Implemented | `modules.rs`; `readAsText`, `readAsDataURL`, `readAsArrayBuffer`, `abort` |
| `crypto.subtle` | Implemented | `builtins/crypto.rs`; digest, HMAC, AES-GCM, HKDF, PBKDF2 — see §4.11 |
| `structuredClone` | Implemented | `modules.rs`; JSON-based deep clone; throws `DataCloneError` for non-serializable values |
| `navigator` | Implemented | `modules.rs`; `userAgent`, `language`, `languages`, `onLine`, `hardwareConcurrency`, `platform` |
| `self` | Implemented | `modules.rs`; `globalThis.self === globalThis` |
| `queueMicrotask` | Implemented | `builtins/timers.rs`; backed by `Promise.resolve().then()` |
| `perf_hooks` (`performance.now`) | JS stub | `modules.rs`; backed by `Date.now()` |
| `BroadcastChannel` | Not implemented | Planned |

## 4.3 `fetch`

```javascript
// GET
const res = await fetch('https://api.example.com/data');
const data = await res.json();

// POST
const res2 = await fetch('https://api.example.com/submit', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ key: 'value' }),
});
console.log(res2.status);
```

Response methods: `.json()`, `.text()`, `.arrayBuffer()`, `.bytes()`, `.blob()`, `.formData()`. Requires `--allow-net=<host>`.

### `response.formData()`

Parses the response body based on the `Content-Type` header:

| Content-Type | Behaviour |
|---|---|
| `application/x-www-form-urlencoded` | Decodes `name=value&...` pairs into a `FormData` |
| `multipart/form-data; boundary=...` | Splits on boundary, parses part headers; file parts become `File` objects |
| Anything else | Rejects with `TypeError` |

```javascript
// Form POST
const res = await fetch('https://api.example.com/upload', {
  method: 'POST',
  headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
  body: 'name=alice&role=admin',
});
const form = await res.formData();
console.log(form.get('name'));  // 'alice'
```

## 4.4 `Headers`

```javascript
// Construction: plain object, [[key,value]] pairs, or another Headers
const h = new Headers({ 'Content-Type': 'application/json', 'X-Token': 'abc' });

h.get('content-type');          // 'application/json'  (case-insensitive)
h.has('x-token');               // true
h.set('x-token', 'xyz');
h.append('accept', 'text/html');
h.delete('x-token');

// Iteration
for (const [name, value] of h) { /* ... */ }
h.forEach((value, name) => { /* ... */ });
[...h.keys()];    // ['content-type', 'accept']
[...h.values()]; // ['application/json', 'text/html']
[...h.entries()]; // [['content-type', 'application/json'], ...]

// Special: multiple Set-Cookie values are preserved separately
h.append('set-cookie', 'a=1');
h.append('set-cookie', 'b=2');
h.getSetCookie(); // ['a=1', 'b=2']
```

## 4.5 `Request`

```javascript
// URL string
const req = new Request('https://api.example.com/data', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ key: 'value' }),
});

req.url;                          // 'https://api.example.com/data'
req.method;                       // 'POST'
req.headers instanceof Headers;   // true
req.headers.get('content-type'); // 'application/json'

// Body consumption (each method marks bodyUsed = true)
const text = await req.text();
const json = await req.clone().json();
const blob = await req.clone().blob();

// fetch() accepts a Request directly
const res = await fetch(req);

// Clone — options override the original
const req2 = new Request(req, { method: 'GET' });
```

## 4.6 `Response`

```javascript
// Constructor
const res = new Response('Hello world', {
  status: 200,
  headers: { 'Content-Type': 'text/plain' },
});

res.ok;                          // true
res.status;                      // 200
res.headers instanceof Headers;  // true

await res.text();                // 'Hello world'

// Static constructors
const jsonRes = Response.json({ ok: true }, { status: 201 });
// Content-Type: application/json is set automatically

const errRes = Response.error();
// status: 0, type: 'error'

const redirect = Response.redirect('https://example.com', 302);
// status: 302, headers.location = 'https://example.com'
```

## 4.7 `WebSocket`

```javascript
const ws = new WebSocket('wss://echo.example.com');

ws.onopen = () => ws.send('hello');
ws.onmessage = (event) => console.log(event.data);
ws.onerror = (err) => console.error(err);
ws.onclose = () => console.log('closed');

ws.close();
```

Requires `--allow-net=<host>`.

## 4.8 `TextEncoder` / `TextDecoder`

```javascript
const enc = new TextEncoder();          // UTF-8
const bytes = enc.encode('hello');      // Uint8Array

const dec = new TextDecoder('utf-8');
const str = dec.decode(bytes);          // 'hello'
```

## 4.9 `AbortController` / `AbortSignal`

```javascript
const controller = new AbortController();
const signal = controller.signal;

fetch('https://api.example.com/data', { signal })
  .catch(err => {
    if (err.message === 'AbortError') console.log('Fetch aborted');
  });

setTimeout(() => controller.abort(), 5000);

// Static helpers
const timedSignal = AbortSignal.timeout(3000);
const alreadyAborted = AbortSignal.abort('reason');
```

The abort signal races against the HTTP response using `Promise.race`. Since `ureq` is synchronous, network I/O cannot be cancelled mid-flight, but the rejection propagates immediately.

## 4.10 `Blob` / `File`

```javascript
const blob = new Blob(['Hello World'], { type: 'text/plain' });
console.log(blob.size);           // 11
console.log(blob.type);           // 'text/plain'

const text = await blob.text();
const buffer = await blob.arrayBuffer();
const bytes = await blob.bytes();          // Uint8Array
const slice = blob.slice(0, 5);           // new Blob
const stream = blob.stream();              // ReadableStream

const file = new File(['content'], 'example.txt', { type: 'text/plain' });
console.log(file.name);           // 'example.txt'
console.log(file instanceof Blob);// true
```

## 4.11 `FormData`

```javascript
const form = new FormData();
form.append('username', 'alice');
form.append('tag', 'admin');
form.append('tag', 'user');

form.get('username');             // 'alice'
form.getAll('tag');               // ['admin', 'user']
form.has('username');             // true
form.set('username', 'bob');      // replaces all 'username' entries
form.delete('tag');

form.forEach((value, key) => console.log(key, value));

for (const [key, value] of form) {
  console.log(key, value);
}
```

## 4.12 Streams

```javascript
// ReadableStream
const stream = new ReadableStream({
  start(controller) {
    controller.enqueue('chunk1');
    controller.enqueue('chunk2');
    controller.close();
  }
});
const reader = stream.getReader();
const { value, done } = await reader.read();

// WritableStream
const writable = new WritableStream({
  write(chunk) { console.log('received:', chunk); },
  close() { console.log('done'); }
});

// TransformStream
const transform = new TransformStream({
  transform(chunk, controller) {
    controller.enqueue(chunk.toUpperCase());
  }
});
// transform.readable instanceof ReadableStream  → true
// transform.writable instanceof WritableStream  → true
```

## 4.13 `perf_hooks` (stub)

Available via `require('perf_hooks')`. Backed by `Date.now()` — not high-resolution.

```javascript
const { performance } = require('perf_hooks');
const t0 = performance.now();
// ... work ...
const elapsed = performance.now() - t0;
```

---

## 4.14 `crypto.subtle` (Web Crypto API)

`globalThis.crypto.subtle` and `require('crypto').subtle` both expose the same
`SubtleCrypto` object backed by Rust (`builtins/crypto.rs`).

### Supported operations

| Operation | Algorithms |
|---|---|
| `digest` | SHA-1, SHA-224, SHA-256, SHA-384, SHA-512 |
| `generateKey` | AES-GCM-128/256, AES-CBC, AES-CTR, HMAC |
| `importKey` | `raw` and `jwk` formats for all symmetric algorithms + HKDF + PBKDF2 |
| `exportKey` | `raw` and `jwk` formats |
| `sign` / `verify` | HMAC (all SHA variants) |
| `encrypt` / `decrypt` | AES-GCM-128 and AES-GCM-256 |
| `deriveBits` / `deriveKey` | HKDF, PBKDF2 |
| `wrapKey` / `unwrapKey` | Not implemented — throws `NotSupportedError` |

### `digest`

```javascript
const data = new TextEncoder().encode('hello');
const hash = await crypto.subtle.digest('SHA-256', data);
console.log(new Uint8Array(hash));  // 32 bytes
```

### `generateKey` + `encrypt` / `decrypt` (AES-GCM)

```javascript
const key = await crypto.subtle.generateKey(
  { name: 'AES-GCM', length: 256 },
  true,
  ['encrypt', 'decrypt']
);

const iv = crypto.getRandomValues(new Uint8Array(12));
const plaintext = new TextEncoder().encode('secret message');

const ciphertext = await crypto.subtle.encrypt(
  { name: 'AES-GCM', iv },
  key,
  plaintext
);

const recovered = await crypto.subtle.decrypt(
  { name: 'AES-GCM', iv },
  key,
  ciphertext
);
console.log(new TextDecoder().decode(recovered));  // 'secret message'
```

`ciphertext` is the concatenation of the encrypted data and the 16-byte GCM authentication tag.
Decryption fails (throws) if either the tag or the ciphertext has been tampered with.

### `importKey` + `sign` / `verify` (HMAC)

```javascript
const rawKey = crypto.getRandomValues(new Uint8Array(32));

const key = await crypto.subtle.importKey(
  'raw',
  rawKey,
  { name: 'HMAC', hash: 'SHA-256' },
  false,
  ['sign', 'verify']
);

const data = new TextEncoder().encode('message');
const signature = await crypto.subtle.sign('HMAC', key, data);
const valid = await crypto.subtle.verify('HMAC', key, signature, data);
console.log(valid);  // true
```

### `deriveBits` / `deriveKey` (HKDF)

```javascript
const ikm = await crypto.subtle.importKey(
  'raw',
  new TextEncoder().encode('input key material'),
  'HKDF',
  false,
  ['deriveBits', 'deriveKey']
);

const bits = await crypto.subtle.deriveBits(
  {
    name: 'HKDF',
    hash: 'SHA-256',
    salt: new TextEncoder().encode('salt'),
    info: new TextEncoder().encode('context'),
  },
  ikm,
  256   // bits
);
console.log(new Uint8Array(bits));  // 32 bytes

// Or derive a ready-to-use AES key:
const aesKey = await crypto.subtle.deriveKey(
  { name: 'HKDF', hash: 'SHA-256', salt: new Uint8Array(32), info: new Uint8Array() },
  ikm,
  { name: 'AES-GCM', length: 256 },
  true,
  ['encrypt', 'decrypt']
);
```

### `exportKey` (JWK)

```javascript
const key = await crypto.subtle.generateKey(
  { name: 'HMAC', hash: 'SHA-256' },
  true,
  ['sign', 'verify']
);
const jwk = await crypto.subtle.exportKey('jwk', key);
// { kty: 'oct', k: '<base64url>', alg: 'HS256', key_ops: [...], ext: true }
```

### AES-GCM limits

| Parameter | Constraint |
|---|---|
| Key length | 128 or 256 bits only |
| IV / nonce | Exactly 12 bytes |
| Tag length | Fixed 16 bytes (128 bits) |
| AAD | Optional; `algorithm.additionalData` |

---

*Web API builtins in `crates/js/src/builtins/`. Node.js compat stubs in `crates/js/src/builtins/modules.rs`.*
