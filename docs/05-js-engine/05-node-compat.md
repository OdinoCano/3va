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

## `Buffer` — subclase real de `Uint8Array`

```javascript
const buf = Buffer.from([0x41, 0x42, 0x43]);

buf instanceof Uint8Array   // → true  (antes: false)
buf instanceof Buffer       // → true
buf[0]                      // → 65 (byte directo, sin ._b)
[...buf]                    // → [65, 66, 67]  (spread nativo)

// Compatible con APIs de TypedArray
const u = new Uint8Array(4);
u.set(buf, 0);              // funciona sin conversión

// DataView sobre el mismo buffer
const view = new DataView(buf.buffer, buf.byteOffset);
view.getUInt8(0);           // → 65

// Todas las operaciones de Buffer siguen funcionando
buf.readUInt32BE(0);
buf.readFloatLE(0);
buf.readBigUInt64LE(0);
buf.subarray(1, 3);         // → Buffer (no Uint8Array genérico)
```

Impacto: `ws`, `msgpackr`, `protobufjs`, `bl`, y cualquier librería que comprueba
`instanceof Uint8Array` o accede a bytes por índice ya funcionan sin conversiones.

---

## `crypto` — Firma y verificación asimétrica

```javascript
const crypto = require('crypto');

// ── Generar y usar claves RSA ────────────────────────────────────────────────
const { privateKey, publicKey } = crypto.generateKeyPairSync('rsa', {
  modulusLength: 2048,
});

const data = Buffer.from('payload a firmar');
const sig = crypto.createSign('RSA-SHA256').update(data).sign(privateKey);
const ok  = crypto.createVerify('RSA-SHA256').update(data).verify(publicKey, sig);
// ok → true

// ── Algoritmos RSA soportados ────────────────────────────────────────────────
// RSA-SHA1, RSA-SHA224, RSA-SHA256, RSA-SHA384, RSA-SHA512
// También: SHA256, SHA384, SHA512 (la clave determina el padding)

// ── ECDSA (P-256 / P-384) ────────────────────────────────────────────────────
const ecPair = crypto.generateKeyPairSync('ec', { namedCurve: 'P-256' });
const ecSig  = crypto.createSign('SHA256').update(data).sign(ecPair.privateKey);
const ecOk   = crypto.createVerify('SHA256').update(data).verify(ecPair.publicKey, ecSig);
// ecOk → true

// ── Importar claves PEM existentes (createPrivateKey / createPublicKey) ──────
const priv = crypto.createPrivateKey('-----BEGIN PRIVATE KEY-----\n...\n-----END PRIVATE KEY-----');
const pub  = crypto.createPublicKey('-----BEGIN PUBLIC KEY-----\n...\n-----END PUBLIC KEY-----');
const sec  = crypto.createSecretKey(crypto.randomBytes(32));  // clave simétrica

priv.type              // → 'private'
priv.asymmetricKeyType // → 'rsa' o 'ec'
priv.export()          // → PEM string

// ── API one-shot (Node.js 15+) ────────────────────────────────────────────────
const sig2 = crypto.sign('SHA256', data, privateKey);
const ok2  = crypto.verify('SHA256', data, publicKey, sig2);

// ── MD5 (para fingerprinting, no seguridad) ───────────────────────────────────
crypto.createHash('md5').update('hello').digest('hex');
// → '5d41402abc4b2a76b9719d911017c592'

// ── Enumerar algoritmos soportados ────────────────────────────────────────────
crypto.getHashes();  // → ['md5', 'sha1', 'sha224', 'sha256', 'sha384', 'sha512']
crypto.getCurves();  // → ['P-256', 'P-384', 'prime256v1', 'secp384r1']
```

**Formato de firmas:** DER por defecto (compatible con `jsonwebtoken`, `passport-jwt`, `jose`).
La verificación acepta tanto DER como P1363 (raw r‖s de 64/96 bytes).

---

## `assert` — deepStrictEqual completo

```javascript
const assert = require('assert');

// Objetos, arrays, anidados
assert.deepStrictEqual({ a: 1, b: { c: 2 } }, { a: 1, b: { c: 2 } });
assert.deepStrictEqual([1, [2, 3]], [1, [2, 3]]);

// TypedArrays — el impl anterior (JSON.stringify) fallaba aquí
assert.deepStrictEqual(new Uint8Array([1,2,3]), new Uint8Array([1,2,3]));

// Valores undefined — JSON.stringify los elimina, deepStrictEqual no
assert.deepStrictEqual({ x: undefined }, { x: undefined });

// Date, RegExp
assert.deepStrictEqual(new Date('2024-01-01'), new Date('2024-01-01'));

// Referencias circulares — ya no lanza
const obj = {}; obj.self = obj;
assert.deepStrictEqual(obj, obj);  // ok (mismo objeto → true)

// API extendida
assert.notDeepStrictEqual({ a: 1 }, { a: 2 });
assert.ifError(null);            // no lanza
assert.ifError(new Error('!'));  // lanza
assert.fail('mensaje');          // siempre lanza
```

---

## `crypto` — Pares de claves asimétricas (nuevo)

```javascript
const crypto = require('crypto');

// RSA — síncrono
const { publicKey, privateKey } = crypto.generateKeyPairSync('rsa', {
  modulusLength: 2048,  // 1024, 2048, 4096
});
console.log(publicKey.export());   // → "-----BEGIN PUBLIC KEY-----\n..."
console.log(privateKey.export());  // → "-----BEGIN PRIVATE KEY-----\n..."

// RSA — asíncrono
crypto.generateKeyPair('rsa', { modulusLength: 2048 }, (err, pub, priv) => {
  if (err) throw err;
  console.log(pub.export());
});

// EC — curvas soportadas: P-256 (prime256v1), P-384 (secp384r1)
const { publicKey: ecPub, privateKey: ecPriv } =
  crypto.generateKeyPairSync('ec', { namedCurve: 'P-256' });

// webcrypto — accesible tanto por crypto.subtle como crypto.webcrypto.subtle
const subtle = crypto.webcrypto.subtle;  // equivalente a crypto.subtle

// scryptSync — implementación real (ya no es un alias de PBKDF2)
const key = crypto.scryptSync('password', 'salt', 32, { N: 16384, r: 8, p: 1 });
// key es Uint8Array de 32 bytes
```

**Formatos de clave devueltos:** PEM PKCS#8 para privadas, PEM SPKI para públicas.
Compatibles directamente con `jsonwebtoken`, `passport-jwt`, `node-jose`.

---

## `Buffer.isBuffer` — acepta `Uint8Array` nativo

```javascript
Buffer.isBuffer(new Uint8Array(4))   // → true  (antes: false)
Buffer.isBuffer(Buffer.from('hi'))   // → true
Buffer.isBuffer('hello')             // → false
```

Esto permite que librerías como `msgpackr`, `bl`, y otros stream helpers funcionen
sin necesidad de envolver todos los bytes en `Buffer`.

---

## `util.inspect` — referencias circulares y custom symbol

```javascript
const util = require('util');

// Referencia circular → [Circular *]
const obj = {}; obj.self = obj;
util.inspect(obj); // → "{\n  self: [Circular *]\n}"

// Symbol.for('nodejs.util.inspect.custom')
class MyClass {
  [Symbol.for('nodejs.util.inspect.custom')]() { return 'MyClass { x: 1 }'; }
}
util.inspect(new MyClass()); // → "MyClass { x: 1 }"

// Control de profundidad
util.inspect({ a: { b: { c: 1 } } }, { depth: 1 }); // anida hasta nivel 1
```

---

## `util.parseArgs` (Node.js 18+)

```javascript
const { parseArgs } = require('util');

const { values, positionals } = parseArgs({
  args: process.argv.slice(2),
  options: {
    port:    { type: 'string', default: '3000' },
    verbose: { type: 'boolean', default: false },
    host:    { type: 'string' },
  },
  allowPositionals: true,
});

// $ node app.js --port=8080 --verbose file.txt
// values   → { port: '8080', verbose: true, host: undefined }
// positionals → ['file.txt']
```

Soporta: `--flag`, `--key value`, `--key=value`, `--`, defaults, valores múltiples (`multiple: true`), y el campo `tokens`.

---

## `reflect-metadata` — decoradores TypeScript

```javascript
require('reflect-metadata');  // inicializa el polyfill

// Definir metadata en una clase (patrón NestJS)
@Injectable()
class MyService {}

function Injectable() {
  return (target) => Reflect.defineMetadata('injectable', true, target);
}

Reflect.getMetadata('injectable', MyService);    // → true
Reflect.hasOwnMetadata('injectable', MyService); // → true
Reflect.getOwnMetadataKeys(MyService);           // → ['injectable']

// Patrón de decoradores de propiedad (TypeORM, tsyringe)
Reflect.defineMetadata('design:type', String, MyClass.prototype, 'name');
Reflect.getMetadata('design:type', MyClass.prototype, 'name'); // → String
```

También accesible como `globalThis.Reflect` (los decoradores TS lo usan directamente).

---

## `child_process.execSync` / `spawnSync`

```javascript
const { execSync, spawnSync } = require('child_process');

// execSync — bloquea hasta que el comando termina
const output = execSync('echo hello', { encoding: 'utf8' });
// → 'hello\n'

// Lanza si exit code ≠ 0
try {
  execSync('exit 1');
} catch (err) {
  err.status;  // → 1
  err.stderr;  // → ''
}

// spawnSync — control granular de args
const result = spawnSync('ls', ['-la', '/tmp'], { encoding: 'utf8' });
result.status;   // → 0
result.stdout;   // → '...'
result.stderr;   // → ''
```

> **Requiere** `--allow-child-process`. Sin este flag, ambas funciones lanzan un error de permisos.
> El hilo se bloquea durante la ejecución (comportamiento idéntico a Node.js).

---

*Última actualización: 2026-05-28 (Sprint 1). Compatible con Node.js 20 LTS como referencia.*
