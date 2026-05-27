# 05 - NODE.JS COMPATIBILITY — DETALLES

Referencia rápida de los módulos Node.js implementados en 3va, con ejemplos de uso
y notas sobre diferencias respecto a Node.js oficial.

---

## EventEmitter (`require('events')`)

API completa equivalente a Node.js 20:

```javascript
const EventEmitter = require('events');
const ee = new EventEmitter();

// Agregar listeners
ee.on('data', (chunk) => console.log(chunk));
ee.once('end', () => console.log('done'));

// Agregar al inicio de la cola (útil para middleware)
ee.prependListener('data', (chunk) => { /* primero */ });
ee.prependOnceListener('drain', () => { /* primera vez, primero */ });

// Inspección
ee.eventNames()          // → ['data', 'end']
ee.listenerCount('data') // → 2
ee.listeners('data')     // → [fn, fn]   (funciones originales, sin wrappers once)
ee.rawListeners('data')  // → [fn, wrapper]  (con wrappers once intactos)
ee.getMaxListeners()     // → 10 (o lo configurado con setMaxListeners)
ee.setMaxListeners(20)

// Remover
ee.off('data', handler)           // alias de removeListener
ee.removeAllListeners('data')     // solo el evento indicado
ee.removeAllListeners()           // todos

// Estáticos
EventEmitter.defaultMaxListeners          // → 10
EventEmitter.listenerCount(ee, 'data')   // compatibilidad legacy
```

---

## `path` (`require('path')`)

Implementación completa con soporte para posix y win32:

```javascript
const path = require('path');

// Operaciones básicas
path.join('a', 'b', '../c')        // → 'a/c'
path.resolve('/foo', 'bar', 'baz') // → '/foo/bar/baz'
path.normalize('/a//b/./c/../d')   // → '/a/b/d'
path.dirname('/a/b/c.txt')         // → '/a/b'
path.basename('/a/b/c.txt', '.txt')// → 'c'
path.extname('/a/b/c.txt')         // → '.txt'
path.isAbsolute('/a/b')            // → true

// relative() ahora funciona correctamente
path.relative('/a/b/c', '/a/b/d') // → '../d'
path.relative('/a/b', '/a/b/c/d') // → 'c/d'
path.relative('/a/b/c', '/a/b/c') // → '.'

// Separadores por plataforma
path.posix.sep   // → '/'
path.win32.sep   // → '\\'

// Submódulos
const posix  = require('path/posix')
const win32  = require('path/win32')
// también: require('node:path/posix'), require('node:path/win32')

// parse / format
path.parse('/home/user/file.txt')
// → { root: '/', dir: '/home/user', base: 'file.txt', ext: '.txt', name: 'file' }

path.format({ dir: '/home/user', name: 'file', ext: '.txt' })
// → '/home/user/file.txt'
```

---

## `os` (`require('os')`)

Valores reales del sistema en Linux; aproximados en otras plataformas:

```javascript
const os = require('os');

os.hostname()    // nombre real del host vía gethostname(3)
os.platform()    // 'linux' | 'darwin' | 'win32'
os.arch()        // 'x64' | 'arm64'
os.type()        // 'Linux' | 'Darwin' | 'Windows_NT'
os.release()     // '6.0.0'
os.version()     // '#1 SMP'
os.machine()     // igual que arch()

// Memoria real (Linux: /proc/meminfo)
os.totalmem()    // → 16831930368  (bytes totales)
os.freemem()     // → 8589934592   (bytes libres — MemAvailable)

// Uptime real (Linux: /proc/uptime)
os.uptime()      // → 12345.67  (segundos)

// Rutas
os.homedir()     // respeta process.env.HOME
os.tmpdir()      // respeta process.env.TMPDIR

// CPUs (cantidad correcta, modelo genérico)
os.cpus()        // → [{ model, speed, times }]
os.availableParallelism()  // → 1

// Constantes
os.constants.signals.SIGTERM  // → 15
os.constants.errno.ENOENT     // → -2
os.constants.priority.PRIORITY_NORMAL  // → 0

os.EOL           // '\n' en Unix, '\r\n' en Windows
os.endianness()  // 'LE'

// Info del usuario
os.userInfo()    // → { username, uid: -1, gid: -1, shell, homedir }
os.getPriority() // → 0
```

---

## `fs` — File Descriptor API

Requiere `--allow-read` y/o `--allow-write`. Backed por `Arc<Mutex<FdTable>>` en Rust.

```javascript
const fs = require('fs');

// Abrir un archivo — flags: 'r', 'r+', 'w', 'w+', 'a', 'a+', 'wx', 'wx+', etc.
const fd = fs.openSync('/tmp/test.txt', 'w');

// Escribir
const data = new TextEncoder().encode('hello world');
const bytesWritten = fs.writeSync(fd, data, 0, data.length, null);
// También acepta string:
fs.writeSync(fd, 'hello', 0, 5, 0);

// Leer
const buf = new Uint8Array(1024);
const bytesRead = fs.readSync(fd, buf, 0, buf.length, 0);
const text = new TextDecoder().decode(buf.slice(0, bytesRead));

// Stat de un fd abierto
const stat = fs.fstatSync(fd);
// → { size, mode, isFile, isDirectory, isSymbolicLink, mtimeMs, ... }

// Cerrar
fs.closeSync(fd);

// Versión async (callback)
fs.open('/tmp/test.txt', 'r', (err, fd) => {
    if (err) throw err;
    const buf = new Uint8Array(64);
    fs.read(fd, buf, 0, 64, 0, (err, bytesRead, buf) => {
        fs.close(fd, () => {});
    });
});
```

### `fs.opendir` / `fs.opendirSync`

```javascript
// Async con callback
fs.opendir('/tmp', (err, dir) => {
    dir.read((err, entry) => {
        if (entry) console.log(entry.name);
        dir.close();
    });
});

// Sync
const dir = fs.opendirSync('/tmp');
let entry;
while ((entry = dir.readSync()) !== null) {
    console.log(entry.name, entry.isFile(), entry.isDirectory());
}
dir.closeSync();

// Async iterator (for await...of)
const dir2 = await fs.opendir('/tmp');
for await (const entry of dir2) {
    console.log(entry.name);
}
```

### `fs.mkdtemp` / `fs.mkdtempSync`

```javascript
// Crea un directorio temporal único con el prefijo dado
const tmpDir = fs.mkdtempSync('/tmp/myapp-');
// → '/tmp/myapp-12345'

fs.mkdtemp('/tmp/myapp-', (err, dir) => {
    console.log(dir); // → '/tmp/myapp-12345'
});

// También disponible como promesa
const dir = await fs.promises.mkdtemp('/tmp/myapp-');
```

---

## `zlib` — Transform Streams

### Callbacks asíncronos (sin cambios)

```javascript
const zlib = require('zlib');

zlib.gzip(Buffer.from('hello'), (err, compressed) => {
    zlib.gunzip(compressed, (err, decompressed) => {
        console.log(decompressed.toString()); // 'hello'
    });
});
```

### Métodos síncronos (ahora reales, sin lanzar excepción)

```javascript
const compressed   = zlib.gzipSync(Buffer.from('hello'));
const decompressed = zlib.gunzipSync(compressed);
console.log(decompressed.toString()); // 'hello'

// También disponibles:
zlib.deflateSync(buf)     / zlib.inflateSync(buf)
zlib.deflateRawSync(buf)  / zlib.inflateRawSync(buf)
```

### Transform streams (ahora reales)

```javascript
const gz = zlib.createGzip();
const gunzip = zlib.createGunzip();

// pipe — encadenamiento estándar
inputStream.pipe(gz).pipe(outputStream);

// EventEmitter manual
gz.on('data', (chunk) => { /* chunk comprimido */ });
gz.on('end', () => { /* compresión completa */ });
gz.write(Buffer.from('hello'));
gz.end();

// También disponibles:
zlib.createDeflate()     / zlib.createInflate()
zlib.createDeflateRaw()  / zlib.createInflateRaw()
zlib.createBrotliCompress() / zlib.createBrotliDecompress()
```

> **Nota:** los Transform streams de zlib usan la misma API de `write/end/on/pipe`
> que `stream.Transform` de Node.js. El evento `end` se emite sólo después de que
> **todos** los chunks escritos hayan sido procesados por Rust.

---

## `process` — API extendida

```javascript
// Métricas del sistema (Linux: valores reales)
process.memoryUsage()
// → { rss, heapTotal, heapUsed, external, arrayBuffers }

process.memoryUsage.rss()          // solo el RSS en bytes

process.cpuUsage()                 // → { user, system } en µs
process.cpuUsage(previousSnapshot) // diferencial

process.uptime()                   // segundos desde inicio
process.hrtime()                   // [segundos, nanosegundos]
process.hrtime.bigint()            // BigInt en nanosegundos

// EventEmitter completo sobre process
process.on('SIGTERM', handler)
process.once('SIGINT', handler)
process.prependListener('exit', handler)
process.removeListener('SIGTERM', handler)
process.eventNames()               // → ['SIGTERM', ...]
process.listenerCount('SIGTERM')   // → n

// Diagnóstico
process.abort()                    // exit(1)
process.report.writeReport()       // → ''
process.setUncaughtExceptionCaptureCallback(fn)
process.hasUncaughtExceptionCaptureCallback()  // → boolean
```

---

*Última actualización: 2026-05-27. Compatible con Node.js 20 LTS como referencia.*
