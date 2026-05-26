# 02 - SYSTEM MODULES

## 2.1 Module System

3va supports both ECMAScript Modules (ESM) and CommonJS (CJS), prioritizing ESM while maintaining compatibility with the npm ecosystem.

## 2.2 Built-in Modules

### 2.2.1 Module Status

| Module | Description | Status |
|--------|-------------|--------|
| `buffer` | Binary data buffer | Rust builtin (`builtins/buffer.rs`) |
| `console` | Output console | Rust builtin (`builtins/console.rs`) |
| `crypto` | Hashing, HMAC, random | JS + Rust (`builtins/crypto.rs`) — `crypto.subtle`, `getRandomValues`, `randomUUID`, `createHash`, `createHmac` (async), `randomBytes`, `timingSafeEqual`, `pbkdf2` |
| `events` | EventEmitter | JS implementation (in `modules.rs`) |
| `fs` | File system | Rust builtin (`builtins/fs.rs`, permission-gated) |
| `http` / `https` | HTTP client/server | JS + Rust — `request()` backed by `__fetchAsync`; `createServer()` backed by `builtins/http_server.rs` (real Tokio TCP listener, HTTP/1.1 parser); requires `--allow-net` |
| `net` | TCP sockets + server | Rust-backed (`builtins/tcp.rs`) — `Socket` (client) via `TcpStream`; `createServer(handler)` via `__netListen`/`__netAcceptAsync`; permission-gated |
| `os` | System information | JS stub (static values: `platform`, `hostname`) |
| `path` | Path utilities | JS implementation (in `modules.rs`) |
| `process` | Current process | Rust builtin (`builtins/process.rs`) |
| `querystring` | Query string parsing | JS implementation (in `modules.rs`) |
| `stream` | Data streams | JS stub (Readable/Writable/Transform classes) |
| `tls` | TLS/SSL | Rust-backed (`builtins/tcp.rs`) — real `TlsStream` via `native-tls`; permission-gated |
| `url` | URL parsing | JS implementation (in `modules.rs`) |
| `util` | Various utilities | JS implementation (`inherits`, `promisify`, etc.) |
| `zlib` | Compression | Rust builtin (`builtins/zlib.rs`) — real gzip/deflate via `flate2`, async callbacks |
| `child_process` | Process spawning | Rust builtin (`builtins/child_process.rs`) — real exec via `tokio`, requires `--allow-child-process` |
| `http2` | HTTP/2 client | JS client backed by `__fetchAsync` — `connect()`, `request()`, NGHTTP2 constants |

**Rust builtins** are implemented as native Rust functions exposed to QuickJS via `rquickjs`. This includes `zlib` (real compression via `flate2`) and `child_process` (real subprocess execution via `tokio`).

**Rust builtins** include `http_server.rs`: `__httpListenAsync` / `__httpAcceptAsync` / `__httpRespond` / `__httpClose` expose a Tokio-backed HTTP/1.1 server to JS. The JS `http.createServer(handler)` API is fully Node.js-compatible — `IncomingMessage` and `ServerResponse` objects with `req.method`, `req.url`, `req.headers`, `req._body`, `res.writeHead()`, `res.write()`, `res.end()`.

**JS stubs** exist in the inline JS block inside `crates/js/src/builtins/modules.rs`. They allow packages that `require()` these modules to load without throwing. `http`/`https` back client requests through `__fetchAsync` and expose real `createServer()` via `builtins/http_server.rs`. `net` and `tls` are backed by real TCP/TLS connections via `builtins/tcp.rs`. `http2` exposes a client API backed by `__fetchAsync`.

### 2.2.2 Notable Implementations

#### console (`builtins/console.rs`)

Rust implementation. Supports: `log`, `error`, `warn`, `info`, `debug`, `trace`, `dir`, `table`, `time`, `timeEnd`, `group`, `groupEnd`.

#### buffer (`builtins/buffer.rs`)

Rust implementation. Supports: `Buffer.from()`, `Buffer.alloc()`, `Buffer.concat()`, `buf.toString(encoding)`.

#### events (JS in `modules.rs`)

JS `EventEmitter` class with: `on`, `once`, `emit`, `off`, `removeAllListeners`, `listeners`, `listenerCount`.

#### fs (`builtins/fs.rs`)

Permission-gated. Requires `--allow-read` / `--allow-write`.

Sync API: `readFileSync`, `writeFileSync`, `appendFileSync`, `existsSync`, `mkdirSync`, `readdirSync(path, {withFileTypes})`, `statSync`, `lstatSync`, `accessSync`, `realpathSync`, `unlinkSync`, `renameSync`, `copyFileSync`, `chmodSync`, `symlinkSync`.

Async (callback): `readFile`, `writeFile`, `appendFile`, `readdir`, `mkdir`, `stat`, `lstat`, `access`, `realpath`, `unlink`, `rename`, `copyFile`, `chmod`, `symlink`.

Promise API: `fs.promises.*` — mirrors all async methods.

Streams: `createReadStream(path)` — EventEmitter emitting `data`/`end`/`error`; `createWriteStream(path)` — `write(chunk)` / `end([chunk])`.

Constants: `fs.constants` — `{ F_OK: 0, R_OK: 4, W_OK: 2, X_OK: 1 }`.

---

## 2.3 Module Loading

### 2.3.1 Resolution Algorithm

```
1. Check __requireCache (built-in stubs registered at startup)
2. If absolute or relative path:
   - Resolve against __dirname
   - Try: exact path, + .js, + .ts, + /index.js, + /index.ts
3. If bare specifier (package name):
   - Search node_modules/
   - Resolve main/exports in package.json
   - Try index.js, index.ts
4. If not found → throw MODULE_NOT_FOUND
```

### 2.3.2 Extension Handling

| Extension | Action |
|-----------|--------|
| `.mjs` | Treated as ESM |
| `.cjs` | Treated as CJS |
| `.js` | ESM or CJS depending on `package.json "type"` |
| `.ts` | Transpiled to JS via Oxc |
| `.tsx` | Transpiled (TSX → JS) |
| `.jsx` | Transpiled (JSX → JS) |

### 2.3.3 Module Cache

All loaded modules are cached in `globalThis.__requireCache` (for CJS stubs) and via QuickJS's internal module registry (for ESM). This prevents double-evaluation and ensures singleton semantics.

---

## 2.4 CommonJS (`require()`)

```javascript
const fs = require('fs');
const _ = require('lodash');

module.exports = { foo: 'bar' };
module.exports.foo = 'bar';
```

CJS modules are wrapped in the standard envelope:

```javascript
(function(exports, require, module, __filename, __dirname) {
    // module code
})(exports, require, module, filename, dirname);
```

---

## 2.5 ECMAScript Modules (ESM)

```javascript
import { foo } from './module';
import defaultExport from './module';
import * as ns from './module';

export const foo = 'bar';
export default function() {}
```

Top-level `await` is supported in ESM context.

---

## 2.6 `package.json` Integration

### 2.6.1 `main` and `exports` fields

```json
{
  "main": "dist/index.js",
  "exports": {
    ".": "./dist/index.js",
    "./feature": "./dist/feature.js"
  }
}
```

### 2.6.2 Conditional Exports

```json
{
  "exports": {
    ".": {
      "import": "./dist/esm/index.mjs",
      "require": "./dist/cjs/index.js"
    }
  }
}
```

---

*Built-in stubs implemented in `crates/js/src/builtins/modules.rs`. Rust builtins in `crates/js/src/builtins/`.*
