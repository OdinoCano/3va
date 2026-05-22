# 04 - WEB APIs

## 4.1 Overview

3va exposes a subset of browser and Node.js-compatible APIs. The table below reflects what is **actually implemented** as of v0.2.0.

## 4.2 API Status

| API | Status | Notes |
|-----|--------|-------|
| `fetch` | Implemented | `builtins/fetch.rs` — HTTP via `ureq`; requires `--allow-net` |
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
| `perf_hooks` (`performance.now`) | JS stub | `modules.rs`; backed by `Date.now()` |
| `BroadcastChannel` | Not implemented | Planned |
| `crypto.subtle` | Not implemented | Planned (requires ML-KEM/ML-DSA crates) |

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

Response methods: `.json()`, `.text()`, `.arrayBuffer()`, `.bytes()`, `.blob()`. Requires `--allow-net=<host>`.

## 4.4 `WebSocket`

```javascript
const ws = new WebSocket('wss://echo.example.com');

ws.onopen = () => ws.send('hello');
ws.onmessage = (event) => console.log(event.data);
ws.onerror = (err) => console.error(err);
ws.onclose = () => console.log('closed');

ws.close();
```

Requires `--allow-net=<host>`.

## 4.5 `TextEncoder` / `TextDecoder`

```javascript
const enc = new TextEncoder();          // UTF-8
const bytes = enc.encode('hello');      // Uint8Array

const dec = new TextDecoder('utf-8');
const str = dec.decode(bytes);          // 'hello'
```

## 4.6 `AbortController` / `AbortSignal`

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

## 4.7 `Blob` / `File`

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

## 4.8 `FormData`

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

## 4.9 Streams

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

## 4.10 `perf_hooks` (stub)

Available via `require('perf_hooks')`. Backed by `Date.now()` — not high-resolution.

```javascript
const { performance } = require('perf_hooks');
const t0 = performance.now();
// ... work ...
const elapsed = performance.now() - t0;
```

---

## 4.11 Planned APIs (not yet implemented)

> **Status: PENDING** — these APIs are part of the roadmap but do not work yet.

### `crypto.subtle`

```javascript
// PLANNED — requires ML-KEM/ML-DSA crates (v0.3.0)
const key = await crypto.subtle.generateKey(
  { name: 'AES-GCM', length: 256 },
  true,
  ['encrypt', 'decrypt']
);
```

---

*Web API builtins in `crates/js/src/builtins/`. Node.js compat stubs in `crates/js/src/builtins/modules.rs`.*
