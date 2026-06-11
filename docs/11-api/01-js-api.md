# 01 - PUBLIC JAVASCRIPT API

## 1.1 Overview

APIs available to JavaScript/TypeScript code running inside the 3va runtime. All are injected at startup; no `import` or `require` is needed for globals.

## 1.2 Global Objects

| Global | Source | Notes |
|--------|--------|-------|
| `console` | `builtins/console.rs` | `log`, `error`, `warn`, `info`, `debug`, `trace`, `dir`, `table`, `time`, `timeEnd`, `group`, `groupEnd` |
| `setTimeout` / `clearTimeout` | `builtins/timers.rs` | Returns numeric `TimerId` |
| `setInterval` / `clearInterval` | `builtins/timers.rs` | Returns numeric `TimerId` |
| `fetch` | `builtins/fetch.rs` | HTTP via `ureq`; requires `--allow-net`; supports `AbortController` signal |
| `WebSocket` | `builtins/websocket.rs` | Requires `--allow-net` |
| `TextEncoder` / `TextDecoder` | `builtins/buffer.rs` | UTF-8 encode/decode |
| `process` | `builtins/process.rs` | `argv`, `env`, `exit()`, `cwd()`, `pid`, `version`, `platform` |
| `Buffer` | `builtins/buffer.rs` | `Buffer.from()`, `Buffer.alloc()`, `Buffer.concat()`, `.toString(encoding)` |
| `crypto` | `builtins/crypto.rs` | `randomBytes()`, `getRandomValues()`, `createHash()`, `createHmac()`, `randomUUID()`, `crypto.subtle` (AES-GCM, ECDH, RSA-OAEP, SHA-*) |
| `URL` | `builtins/modules.rs` | Full URL parsing + relative resolution; `.protocol`, `.host`, `.pathname`, `.searchParams`, `URL.canParse()` |
| `URLSearchParams` | `builtins/modules.rs` | `append`, `set`, `get`, `getAll`, `has`, `delete`, `forEach`, iteration, `.size` |
| `FileReader` | `builtins/modules.rs` | `readAsText`, `readAsDataURL`, `readAsArrayBuffer`, `abort`; `onload`, `onerror`, `onabort` callbacks |
| `AbortController` / `AbortSignal` | `builtins/modules.rs` | `abort()`, `addEventListener('abort', ...)`, `AbortSignal.timeout()`, `AbortSignal.abort()` |
| `Blob` / `File` | `builtins/modules.rs` | `text()`, `arrayBuffer()`, `bytes()`, `slice()`, `stream()`; `File` extends `Blob` |
| `FormData` | `builtins/modules.rs` | `append`, `set`, `get`, `getAll`, `has`, `delete`, `forEach`, `Symbol.iterator` |
| `ReadableStream` | `builtins/modules.rs` | `getReader()`, `pipeTo()`, `pipeThrough()`, `tee()` |
| `WritableStream` | `builtins/modules.rs` | `getWriter()`, `write()`, `close()` |
| `TransformStream` | `builtins/modules.rs` | `.readable` + `.writable` pair; custom `transform()` and `flush()` |

## 1.3 `require()` / ESM Modules

Both CommonJS `require()` and ESM `import` are supported. Built-in module IDs follow Node.js conventions (`'fs'`, `'path'`, `'events'`, etc.). For the full list and status of each module, see [04-core/02-modulo-system.md](../04-core/02-modulo-system.md).

## 1.4 `fs` Module

Permission-gated. Requires `--allow-read` and/or `--allow-write`.

```javascript
const fs = require('fs');

fs.readFileSync('/path/to/file', 'utf8');
fs.writeFileSync('/path/to/file', 'content');
fs.existsSync('/path/to/file');
fs.mkdirSync('/path/to/dir', { recursive: true });
fs.readdirSync('/path/to/dir');
fs.statSync('/path/to/file');
fs.unlinkSync('/path/to/file');
```

## 1.5 `fetch` API

Requires `--allow-net=<host>`. Throws a permission denied error if the target host is not in the allowed list.

```javascript
const res = await fetch('https://api.example.com/data');
const json = await res.json();

// POST
await fetch('https://api.example.com/submit', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ key: 'value' }),
});
```

## 1.6 Timers

```javascript
const id = setTimeout(() => console.log('fired'), 500);
clearTimeout(id);

const interval = setInterval(() => console.log('tick'), 1000);
clearInterval(interval);
```

## 1.7 `process`

```javascript
process.argv       // string[] — command-line arguments
process.env        // object  — environment variables (requires --allow-env)
process.exit(0);   // terminate with exit code
process.cwd();     // current working directory
process.pid;       // process ID
process.version;   // 3va version string
process.platform;  // 'linux', 'darwin', 'win32'
```

## 1.8 `Buffer`

```javascript
const buf = Buffer.from('hello', 'utf8');
buf.toString('hex');
buf.toString('base64');

const alloc = Buffer.alloc(16);
const concat = Buffer.concat([buf, alloc]);
```

## 1.9 `TextEncoder` / `TextDecoder`

```javascript
const enc = new TextEncoder();
const bytes = enc.encode('hello');   // Uint8Array

const dec = new TextDecoder();
const str = dec.decode(bytes);       // 'hello'
```

---

## 1.10 Planned APIs (not yet implemented)

> **Status: PENDING** — the following are design goals, not current behavior.

### Runtime namespace

```javascript
// PLANNED — does not exist yet. Note: `3va` is not a valid JS identifier
// (it starts with a digit), so the namespace would be exposed as `vvva`
// (matching the crate prefix) or accessed via globalThis['3va'].
vvva.version          // runtime version string
vvva.versions.node    // Node.js compatibility version
vvva.gc()             // trigger garbage collection hint

// PLANNED — security introspection from JS
vvva.security.checkPermission('fs', 'read', '/path')
vvva.security.getAuditLog()
```

*There is no `vvva.*`, `3va.*`, or `Deno.*` global namespace today. These are planned for a future version.*
