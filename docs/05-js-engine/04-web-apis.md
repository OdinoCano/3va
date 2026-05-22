# 04 - WEB APIs

## 4.1 Overview

3va exposes a subset of browser and Node.js-compatible APIs. The table below reflects what is **actually implemented** as of v0.1.0.

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
| `perf_hooks` (`performance.now`) | JS stub | `modules.rs`; backed by `Date.now()` |
| `AbortController` / `AbortSignal` | Not implemented | Planned |
| `ReadableStream` / `WritableStream` | Not implemented | Planned |
| `Blob` / `File` / `FileReader` | Not implemented | Planned |
| `BroadcastChannel` | Not implemented | Planned |
| `FormData` | Not implemented | Planned |
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

Response methods: `.json()`, `.text()`, `.arrayBuffer()`. Requires `--allow-net=<host>`.

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

## 4.6 `perf_hooks` (stub)

Available via `require('perf_hooks')`. Backed by `Date.now()` — not high-resolution.

```javascript
const { performance } = require('perf_hooks');
const t0 = performance.now();
// ... work ...
const elapsed = performance.now() - t0;
```

---

## 4.7 Planned APIs (not yet implemented)

> **Status: PENDING** — these APIs are part of the roadmap but do not work yet. Code using them will throw or silently no-op.

### `AbortController` / `AbortSignal`

```javascript
// PLANNED — not yet functional
const controller = new AbortController();
const signal = controller.signal;

fetch('/api/data', { signal })
  .catch(err => {
    if (err.name === 'AbortError') console.log('Fetch aborted');
  });

controller.abort();
```

### `ReadableStream` / `WritableStream`

```javascript
// PLANNED — not yet functional
const stream = new ReadableStream({
  start(controller) {
    controller.enqueue('chunk1');
    controller.close();
  }
});

const reader = stream.getReader();
const { value, done } = await reader.read();
```

### `Blob` / `File`

```javascript
// PLANNED — not yet functional
const blob = new Blob(['Hello World'], { type: 'text/plain' });
const file = new File(['content'], 'example.txt', { type: 'text/plain' });

const text = await blob.text();
const buffer = await blob.arrayBuffer();
```

### `FormData`

```javascript
// PLANNED — not yet functional
const form = new FormData();
form.append('username', 'alice');
form.append('file', blob, 'file.txt');

await fetch('/upload', { method: 'POST', body: form });
```

### `crypto.subtle`

```javascript
// PLANNED — requires ML-KEM/ML-DSA crates (v0.3.0)
const key = await crypto.subtle.generateKey(
  { name: 'AES-GCM', length: 256 },
  true,
  ['encrypt', 'decrypt']
);
const encrypted = await crypto.subtle.encrypt(
  { name: 'AES-GCM', iv },
  key,
  data
);
```

---

*Web API builtins in `crates/js/src/builtins/`. Node.js compat stubs in `crates/js/src/builtins/modules.rs`.*
