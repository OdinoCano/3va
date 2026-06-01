# 02 - Node.js Compatibility v2

## 2.1 Scope

v2.0.0 adds the Node.js modules most commonly needed by Express, Fastify, Koa, and their middleware ecosystem. The goal is to run these frameworks without any shim or polyfill layer.

---

## 2.2 New Modules

### `worker_threads`

```js
const { Worker, isMainThread, parentPort, workerData } = require('worker_threads');

if (isMainThread) {
  const worker = new Worker('./worker.js', { workerData: { n: 10 } });
  worker.on('message', (result) => console.log('Result:', result));
} else {
  parentPort.postMessage(workerData.n * 2);
}
```

**Implementation:** Each `Worker` spawns a new OS thread with its own `JsEngine` instance (QuickJS is not thread-safe; isolation is enforced). `MessageChannel` passes data via `serde_json` serialization over a `tokio::sync::mpsc` channel bridged across the thread boundary with `std::sync::mpsc`.

**Permission model:** Workers inherit a read-only copy of the parent's `PermissionState`. The parent cannot grant new permissions to a worker after creation.

---

### `dgram` (UDP)

```js
const dgram = require('dgram');
const socket = dgram.createSocket('udp4');

socket.on('message', (msg, rinfo) => {
  console.log(`Received ${msg} from ${rinfo.address}:${rinfo.port}`);
});
socket.bind(41234);
socket.send('hello', 41234, 'localhost');
```

Requires `--allow-net=<host>`. Backed by `tokio::net::UdpSocket`.

---

### `dns`

```js
const dns = require('dns');
const { promises: dnsPromises } = require('dns');

// Callback API
dns.resolve4('example.com', (err, addresses) => { ... });

// Promise API
const addrs = await dnsPromises.resolve4('example.com');
const { address } = await dnsPromises.lookup('example.com');
```

Backed by `tokio::net::lookup_host`. Requires `--allow-net=<host>`.

---

### `globalThis.crypto` (Web Crypto / SubtleCrypto)

```js
// Available as global — no require() needed
const key = await crypto.subtle.generateKey(
  { name: 'AES-GCM', length: 256 },
  true,
  ['encrypt', 'decrypt']
);
const encrypted = await crypto.subtle.encrypt(
  { name: 'AES-GCM', iv: crypto.getRandomValues(new Uint8Array(12)) },
  key,
  new TextEncoder().encode('hello')
);
```

`globalThis.crypto.subtle` is backed by the existing `vvva_crypto` crate (AES-GCM, SHA-*, HMAC, ECDSA, ECDH). `crypto.getRandomValues` is already implemented in v1.0.0.

---

### `stream/web` (WHATWG Streams)

```js
const { ReadableStream, WritableStream, TransformStream } = require('stream/web');
// Also available as globals (no require needed)

const readable = new ReadableStream({
  start(controller) {
    controller.enqueue('chunk1');
    controller.close();
  },
});
```

Pure-JS implementation injected via `modules.rs`, compatible with the Fetch API body streaming already implemented in v1.0.0.

---

### `timers/promises`

```js
const { setTimeout, setInterval, setImmediate } = require('timers/promises');

await setTimeout(100);                       // resolves after 100 ms
for await (const _ of setInterval(1000)) {  // fires every second
  console.log('tick');
}
```

Thin wrappers over the existing `__setTimeout` / `__setInterval` runtime primitives.

---

### `readline` (full implementation)

v1.0.0 has a partial readline. v2.0.0 completes:

- `readline.createInterface({ input, output })`
- `rl.question(prompt, callback)`
- `rl.prompt()`
- Async iterator: `for await (const line of rl) { ... }`
- `readline.promises.createInterface` (Node 17+ promise API)

---

## 2.3 Improved Modules

| Module | v1.0.0 gap | v2.0.0 fix |
|--------|-----------|-----------|
| `events` | Missing `EventEmitter.once` / `EventEmitter.on` static helpers (Node 16+) | Added |
| `path` | Missing `path.toNamespacedPath` (Windows no-op on POSIX) | Added no-op |
| `os` | `os.cpus()` returns empty array | Returns real CPU info via `sysinfo` crate |
| `fs` | `fs.cp` (recursive copy, Node 16+) missing | Implemented |
| `http` | `http.globalAgent` not exposed | Exposed as `{ maxSockets: Infinity }` stub |
| `crypto` | `crypto.generateKeyPair` (async callback form) missing | Implemented |

---

## 2.4 Compatibility Test Suite

v2.0.0 will add a dedicated compatibility suite under `tests/node-compat/` that runs known-good Node.js snippets and asserts identical output. Each new module ships with at least 10 compatibility tests.
