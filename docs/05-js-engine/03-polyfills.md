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

Permission-gated (`--allow-read` / `--allow-write`). All methods have a sync variant, an async callback variant, and a `fs.promises.*` variant.

```javascript
const fs = require('fs');  // or require('node:fs')

// ── Sync ──────────────────────────────────────────────────────────────────────
fs.existsSync('/path/file');                     // boolean
fs.readFileSync('/path/file', 'utf8');           // string
fs.readFileSync('/path/file');                   // Buffer (binary)
fs.writeFileSync('/path/file', 'content');
fs.appendFileSync('/path/file', 'more');
fs.mkdirSync('/path/dir');                       // recursive by default (create_dir_all)
fs.readdirSync('/path/dir');                     // string[]
fs.readdirSync('/path/dir', { withFileTypes: true }); // Dirent[]
fs.statSync('/path/file');                       // Stats object (see below)
fs.lstatSync('/path/file');                      // Stats (no symlink follow)
fs.accessSync('/path/file', fs.constants.R_OK); // throws on failure
fs.realpathSync('/path/link');                   // canonical path string
fs.renameSync('/old', '/new');
fs.unlinkSync('/path/file');
fs.rmSync('/path/dir');                          // removes file or directory tree
fs.copyFileSync('/src', '/dest');
fs.chmodSync('/path/file', 0o644);
fs.symlinkSync('/target', '/link');

// ── Stats object ──────────────────────────────────────────────────────────────
const s = fs.statSync('/path/file');
s.isFile()         // boolean
s.isDirectory()    // boolean
s.isSymbolicLink() // boolean
s.size             // bytes
s.mode             // Unix mode integer
s.mtime            // Date
s.atime            // Date
s.ctime            // Date
s.mtimeMs          // number (milliseconds)

// ── fs.constants ─────────────────────────────────────────────────────────────
fs.constants.F_OK  // 0 — existence check
fs.constants.R_OK  // 4 — read permission
fs.constants.W_OK  // 2 — write permission
fs.constants.X_OK  // 1 — execute permission

// ── Async (callback) ─────────────────────────────────────────────────────────
fs.readFile('/path/file', 'utf8', (err, data) => console.log(data));
fs.stat('/path/file', (err, stats) => console.log(stats.isFile()));
fs.access('/path/file', fs.constants.R_OK, (err) => {
  if (err) console.error('not readable');
});

// ── Streams ──────────────────────────────────────────────────────────────────
const rs = fs.createReadStream('/path/file');
rs.on('data', (chunk) => process.stdout.write(chunk));
rs.on('end', () => console.log('done'));
rs.resume();  // or pipe to a writable

const ws = fs.createWriteStream('/path/out');
ws.write('hello ');
ws.end('world');  // flushes to disk

// ── fs.promises ──────────────────────────────────────────────────────────────
const { promises: fsp } = require('fs');
const data = await fsp.readFile('/path/file', 'utf8');
await fsp.writeFile('/path/file', 'content');
const stats = await fsp.stat('/path/file');
```

**Limitations:**
- `createReadStream`/`createWriteStream` read/write the entire file in a single operation (no chunked streaming). Binary encoding is not supported in `createReadStream`.
- `watch()` returns a stub emitter — no real filesystem events (inotify is outside the sandbox).
- `chmod` and `symlink` require Unix (not available on Windows builds).

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

### `net` / `tls` (stub)

`createConnection()` returns an emitter. No real TCP connection is made. Planned: expose `tokio::net::TcpStream` as a Rust builtin.

### `perf_hooks`

```javascript
const { performance } = require('perf_hooks');
performance.now();  // Date.now() — millisecond precision, not high-resolution
```

## 3.4 `fetch` (global, Rust implementation)

```javascript
const res = await fetch('https://api.example.com/data');
const json = await res.json();

// AbortController integration
const controller = new AbortController();
fetch('https://api.example.com/data', { signal: controller.signal });
setTimeout(() => controller.abort(), 3000);
```

Permission-gated (`--allow-net`). Implemented in `builtins/fetch.rs` via `ureq`. Supports `AbortController` via `Promise.race`.

## 3.5 `http` / `https` (client, backed by `__fetchAsync`)

Real HTTP client — `request()` and `get()` use the same `ureq` backend as `fetch`. Requires `--allow-net`.

```javascript
const http  = require('http');
const https = require('https');

// Callback-style
const req = https.request({
  hostname: 'api.example.com',
  path: '/data',
  method: 'GET',
  headers: { 'Accept': 'application/json' }
}, (res) => {
  console.log(res.statusCode);   // 200
  res.on('data', (chunk) => console.log(chunk));
  res.on('end', () => console.log('done'));
});
req.end();

// GET shortcut
https.get('https://api.example.com/ping', (res) => {
  res.on('data', (d) => console.log(d));
});

// http.STATUS_CODES
console.log(http.STATUS_CODES[404]); // 'Not Found'
```

`createServer()` returns a no-op object (server-side HTTP requires TCP socket support).

## 3.6 `zlib` (Rust implementation via `flate2`)

Real compression/decompression backed by `flate2` (`builtins/zlib.rs`). All operations are async (offloaded to `spawn_blocking`).

```javascript
const zlib = require('zlib');  // or require('node:zlib')

// Compress
zlib.gzip(Buffer.from('hello world'), (err, compressed) => {
  // compressed is a Uint8Array of gzip bytes
  zlib.gunzip(compressed, (err, decompressed) => {
    console.log(Buffer.from(decompressed).toString('utf8'));  // 'hello world'
  });
});

// deflate / inflate (zlib framing)
zlib.deflate(data, (err, compressed) => { /* ... */ });
zlib.inflate(compressed, (err, data) => { /* ... */ });

// raw deflate / inflate (no framing)
zlib.deflateRaw(data, (err, compressed) => { /* ... */ });
zlib.inflateRaw(compressed, (err, data) => { /* ... */ });

// Constants
zlib.constants.Z_OK             // 0
zlib.constants.Z_BEST_SPEED     // 1
zlib.constants.Z_BEST_COMPRESSION // 9
```

**Note:** `gzipSync`/`gunzipSync`/`deflateSync`/`inflateSync` throw — use the async callback forms.

## 3.7 `child_process` (Rust implementation via `tokio::process`)

Real subprocess execution backed by `builtins/child_process.rs`. Requires `--allow-child-process`.

```javascript
const { exec, execFile, spawn } = require('child_process');

// exec — runs via shell (sh -c on Linux/Mac, cmd /C on Windows)
exec('ls -la /tmp', (err, stdout, stderr) => {
  if (err) throw err;
  console.log(stdout);
});

// execFile — runs binary directly (no shell)
execFile('/usr/bin/node', ['--version'], (err, stdout) => {
  console.log(stdout);
});

// spawn — returns ChildProcess-like with stdout/stderr streams
const child = spawn('cat', ['/etc/os-release']);
child.stdout.on('data', (chunk) => console.log(chunk));
child.on('exit', (code) => console.log('exited:', code));

// Promise wrapper
function execAsync(cmd) {
  return new Promise((resolve, reject) => {
    exec(cmd, (err, stdout, stderr) => {
      if (err) reject(err); else resolve({ stdout, stderr });
    });
  });
}
const { stdout } = await execAsync('echo hello');
```

**Note:** `execSync` and `spawnSync` throw — use the async forms.

## 3.9 `TextEncoder` / `TextDecoder` (global)

```javascript
const enc = new TextEncoder();
const bytes = enc.encode('hello');   // Uint8Array, UTF-8

const dec = new TextDecoder();
const str = dec.decode(bytes);       // 'hello'
```

## 3.10 `AbortController` / `AbortSignal` (global, JS implementation)

Implemented in `modules.rs`. Integrates with `fetch` via `Promise.race`.

```javascript
const controller = new AbortController();
controller.signal.addEventListener('abort', () => console.log('aborted'));
controller.abort('reason');
console.log(controller.signal.aborted);  // true

const timedSignal = AbortSignal.timeout(5000);
const preAborted = AbortSignal.abort('immediately');
```

## 3.11 `Blob` / `File` (global, JS implementation)

Implemented in `modules.rs`. `File` extends `Blob`.

```javascript
const blob = new Blob(['content'], { type: 'text/plain' });
const text = await blob.text();           // 'content'
const buf  = await blob.arrayBuffer();    // ArrayBuffer
const u8   = await blob.bytes();          // Uint8Array
const sub  = blob.slice(0, 3);            // new Blob
const rs   = blob.stream();               // ReadableStream

const file = new File(['data'], 'test.txt', { type: 'text/plain' });
console.log(file.name, file.size);        // 'test.txt' 4
console.log(file instanceof Blob);        // true
```

## 3.12 `FormData` (global, JS implementation)

Implemented in `modules.rs`.

```javascript
const fd = new FormData();
fd.append('key', 'value');
fd.set('key', 'new');
fd.get('key');           // 'new'
fd.getAll('key');        // ['new']
fd.has('key');           // true
fd.delete('key');
fd.forEach((v, k) => console.log(k, v));
for (const [k, v] of fd) { /* ... */ }
```

## 3.13 Streams (global, JS implementation)

Implemented in `modules.rs`.

```javascript
// ReadableStream
const rs = new ReadableStream({
  start(c) { c.enqueue('chunk'); c.close(); }
});

// WritableStream
const ws = new WritableStream({
  write(chunk) { console.log(chunk); },
  close() { console.log('done'); }
});

// TransformStream
const ts = new TransformStream({
  transform(chunk, controller) { controller.enqueue(chunk.toUpperCase()); }
});
// ts.readable instanceof ReadableStream → true
// ts.writable instanceof WritableStream → true

await rs.pipeTo(ws);
```

---

## 3.14 `URL` / `URLSearchParams` (global, JS implementation)

Full URL parsing implemented in `modules.rs`. Also exposed via `require('url')`.

```javascript
const url = new URL('https://user:pass@api.example.com:8080/path?q=1#frag');
url.protocol    // 'https:'
url.hostname    // 'api.example.com'
url.port        // '8080'
url.pathname    // '/path'
url.search      // '?q=1'
url.hash        // '#frag'
url.origin      // 'https://api.example.com:8080'
url.searchParams.get('q')  // '1'
url.toString()  // full href

// Relative resolution
new URL('/new', 'https://example.com/old').href  // 'https://example.com/new'

// Static helper
URL.canParse('https://example.com')  // true
URL.canParse('not a url')            // false

// URLSearchParams standalone
const params = new URLSearchParams('foo=bar&baz=qux');
params.get('foo')       // 'bar'
params.getAll('baz')    // ['qux']
params.set('foo', 'x');
params.delete('baz');
params.toString()       // 'foo=x'
params.size             // 1
for (const [k, v] of params) { /* ... */ }
```

## 3.15 `FileReader` (global, JS implementation)

Implemented in `modules.rs`. Reads data from `Blob`/`File` objects using their promise-based API.

```javascript
const blob = new Blob(['hello world'], { type: 'text/plain' });
const reader = new FileReader();

reader.onload = (e) => console.log(e.target.result);  // 'hello world'
reader.onerror = (e) => console.error(e.target.error);
reader.onabort = () => console.log('aborted');

reader.readAsText(blob);           // → result: string
reader.readAsDataURL(blob);        // → result: 'data:text/plain;base64,...'
reader.readAsArrayBuffer(blob);    // → result: ArrayBuffer
reader.abort();                    // cancels in-flight read

// Constants
FileReader.EMPTY    // 0
FileReader.LOADING  // 1
FileReader.DONE     // 2
```

## 3.16 `process` (global)

`process` is a global object (no `require()` needed, though `require('process')` also works).

```javascript
// Identity
process.pid              // number — OS PID of the 3va process
process.platform         // 'linux' | 'darwin' | 'win32' | 'unknown'
process.arch             // 'x64' | 'arm64' | 'unknown'
process.version          // '3va/0.1.0'

// Versions (Node-compatible keys for package compatibility checks)
process.versions.node    // '20.0.0'  — fake, but structurally compatible
process.versions.v8      // '11.3.244.8-node.20'
process.versions['3va']  // '0.1.0'

// Working directory
process.cwd()            // string — real CWD via std::env::current_dir()
process.chdir(path)      // no-op (sandbox does not change working directory)

// Arguments
process.argv[0]          // path to the 3va binary
process.argv[1]          // absolute path to the running script
process.argv[2+]         // arguments passed after -- on the CLI

// Environment (requires --allow-env)
process.env.HOME         // string | undefined

// Timing
process.hrtime()         // [seconds, nanoseconds] relative to process start
process.hrtime.bigint()  // BigInt nanoseconds

// Scheduling
process.nextTick(fn, ...args)  // schedules fn as a microtask (Promise.resolve().then)

// I/O
process.stdout.write('text')   // prints without newline
process.stderr.write('text')
process.stdout.fd              // 1
process.stderr.fd              // 2
process.stdout.isTTY           // false (sandbox, non-interactive)

// Exit
process.exit(code?)            // terminates the process with std::process::exit
```

**`setImmediate` / `clearImmediate`** are also available as globals (backed by `setTimeout(fn, 0)`).

---

## 3.17 `module` (`require('module')`)

The built-in `module` package exposes the Node.js `Module` API. Required by frameworks like Next.js that use `Module._resolveFilename` and `Module.createRequire`.

```javascript
const Module = require('module');  // or require('node:module')

// Resolve a module path from a given file
Module._resolveFilename('react', { filename: '/app/index.js' });
// → '/app/node_modules/react/index.js'

// Create a require function scoped to a specific file
const myRequire = Module.createRequire('/app/index.js');
myRequire('react');  // resolves relative to /app/

// List of built-in module names
Module.builtinModules;  // ['assert', 'buffer', 'fs', 'http', ...]
Module.isBuiltin('fs'); // true
Module.isBuiltin('axios'); // false

// Cache reference (same as globalThis.__requireCache)
Module._cache;

// No-op for ESM/CJS interop
Module.syncBuiltinESMExports();
```

**Limitations:** `Module._load` is a thin wrapper over `require()`. It does not replicate the full Node.js loader pipeline (no loader hooks, no `--experimental-loader`).

---

## 3.18 `global` / `GLOBAL` (globals)

```javascript
global === globalThis    // true
GLOBAL === globalThis    // true
```

Many npm packages reference `global.xxx` instead of `globalThis.xxx`. Both aliases point to the same QuickJS global object.

---

## 3.19 Planned Polyfills (not yet implemented)

> **Status: PENDING** — these APIs appear in the compatibility roadmap. Using them currently throws `ReferenceError` or silently no-ops.

### Real `crypto` (Web Crypto API)

The current `crypto` stub uses `Math.random()` and `btoa()`. Full Web Crypto API planned for v0.3.0:

```javascript
// PLANNED — requires ml-kem + ml-dsa crates
const bytes = crypto.getRandomValues(new Uint8Array(16)); // real CSPRNG
const hash = await crypto.subtle.digest('SHA-256', data);
```

---

*Rust builtins in `crates/js/src/builtins/`. JS stubs in `crates/js/src/builtins/modules.rs`.*
