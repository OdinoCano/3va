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

**Shared Memory Limitation:** Since QuickJS runs in isolated OS threads with independent heaps, `SharedArrayBuffer` and `Atomics` (sharing raw memory directly between workers) are **not supported** and are declared a *Non-Goal* for v2.0.0. All data sharing must go through message passing.

**Permission model:** Workers inherit a read-only copy of the parent's `PermissionState`. The parent cannot grant new permissions to a worker after creation. Optionally, a parent can restrict a worker further during instantiation by passing a restricted permission map in the `Worker` constructor option keys.

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

Requires `--allow-net=<host>`.

**Implementation note:** `tokio::net::lookup_host` only performs forward A/AAAA resolution and is **not sufficient** to implement the full `dns` API. Methods that perform arbitrary DNS record queries (`resolve4`, `resolve6`, `resolveMx`, `resolveTxt`, `resolveSrv`, `reverse`, etc.) require a dedicated DNS resolver crate such as `hickory-dns` (formerly `trust-dns-resolver`). This must be added to `Cargo.toml` as a dependency before implementing those methods.

---

### `globalThis.crypto` (Web Crypto / SubtleCrypto) — **already in v1.0.0**

`globalThis.crypto` (with `.subtle`, `.getRandomValues`, and `.randomUUID`) is **already set in v1.0.0** by `builtins/crypto.rs`. It does not need to be re-implemented.

v2.0.0 extends this global with post-quantum algorithm support via `crypto.subtle` (see [06-security-v2.md §6.6](06-security-v2.md)). The existing usage continues unchanged:

```js
// Available as global — no require() needed (v1.0.0+)
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

`globalThis.crypto.subtle` is backed by the `vvva_crypto` crate (AES-GCM, SHA-*, HMAC, ECDSA, ECDH).

---

### `stream/web` (WHATWG Streams)

```js
const { ReadableStream, WritableStream, TransformStream } = require('stream/web');
// Also available as globals (no require needed — already true in v1.0.0)

const readable = new ReadableStream({
  start(controller) {
    controller.enqueue('chunk1');
    controller.close();
  },
});
```

**Note:** `ReadableStream`, `WritableStream`, and `TransformStream` are **already available as globals in v1.0.0** (injected by `modules.rs`). What v2.0.0 adds is the `require('stream/web')` subpath, allowing destructured imports consistent with Node.js 16+ conventions. No new implementation is needed — only registering `'stream/web'` as an alias in the require cache pointing to the existing globals.

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
| `os` | `os.cpus()` returns placeholder objects (`model: 'Unknown', speed: 0`) | Returns real CPU info via `sysinfo` crate |
| `fs` | `fs.cp` (recursive copy, Node 16+) missing | Implemented |
| `http` | `http.globalAgent` not exposed | Exposed as `{ maxSockets: Infinity }` stub |

---

## 2.4 Compatibility Test Suite

Compatibility tests live in `crates/js/tests/` — see `pipeline.rs`, `framework_compat.rs`, `compat_sprint1.rs`, and `compat_fixes.rs`. Each new module ships with at least 10 compatibility tests.
