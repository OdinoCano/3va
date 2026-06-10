# 04 - PROCESS MANAGEMENT

## 4.1 Process Object

The `process` object is a global object that provides information about the 3va runtime process and allows controlling it.

## 4.2 Process Properties

### 4.2.1 Environment Information

```rust
// Process object properties
pub struct ProcessInfo {
    pub version: String,           // "3va/x.x.x"
    pub versions: Versions,        // Detailed version information
    pub platform: String,          // "linux", "darwin", "win32"
    pub arch: String,              // "x64", "arm64", etc.
    pub execPath: String,          // Path to 3va executable
    pub cwd: String,               // Current working directory
    pub home: String,              // User home directory
    pub tmpdir: String,            // Temporary directory
}
```

```javascript
// Access in JavaScript
console.log(process.version);        // "3va/2.0.1"
console.log(process.platform);      // "linux"
console.log(process.arch);           // "x64"
console.log(process.cwd());          // "/home/user/project"
console.log(process.execPath);      // "/usr/local/bin/3va"
```

### 4.2.2 Environment Variables

`process.env` is populated at runtime startup. Only variables that pass the
permission check are included in the object — absent variables are simply not
present (not `undefined`).

```javascript
// process.env is an ordinary JS object; only permitted keys are visible.
process.env.NODE_ENV    // present if --allow-env= or --allow-env=NODE_ENV was passed
process.env.SECRET_KEY  // absent unless that variable was explicitly allowed
Object.keys(process.env) // lists only the granted variables
```

**Permission modes**

| CLI flag | Variables visible in `process.env` |
|---|---|
| (omitted) | `{}` — empty, no variables accessible |
| `--allow-env=` | Full host environment (all variables) |
| `--allow-env=NODE_ENV` | Only `NODE_ENV` |
| `--allow-env=NODE_ENV,PORT` | Only `NODE_ENV` and `PORT` |

```bash
# Expose only two variables — all others are hidden
3va run app.ts --allow-env=NODE_ENV,PORT

# Expose everything (equivalent to Node.js default behaviour)
3va run app.ts --allow-env=
```

The filtering happens in `inject_process` (`crates/js/src/builtins/process.rs`)
before the JS context starts. It calls `PermissionState::check(&Capability::EnvVar(key))`
for each host variable; because `caps_match` maps `EnvAccess → EnvVar(_)`, the
full-grant case and the scoped case both resolve through the same code path.

### 4.2.3 Memory Information

```javascript
// Reads real RSS from /proc/self/status on Linux.
process.memoryUsage()
// → { rss: 35287040, heapTotal: 20971520, heapUsed: 12400704,
//     external: 0, arrayBuffers: 0 }

// Shortcut — just the RSS in bytes:
process.memoryUsage.rss()           // → 35287040
```

### 4.2.4 CPU Information

```javascript
// Reads user+system times from /proc/self/stat on Linux (µs).
process.cpuUsage()                  // → { user: 100000, system: 50000 }
process.cpuUsage(previous)          // → differential from previous snapshot
```

## 4.3 Process Methods

### 4.3.1 Flow Control

```javascript
// Exit process
process.exit(code?: number): void
// code: 0 = success, other = error
// Example:
process.exit(0);   // Clean exit
process.exit(1);   // Error

// Similar to process.exit but runs cleanup first
process.exitCode = 1;
process.exit();    // Uses the set code

// nextTick - execute on next event loop iteration
process.nextTick(callback: () => void): void
// Higher priority than setTimeout(0)
process.nextTick(() => console.log('nextTick'));

// setImmediate - execute on next check phase
process.setImmediate(callback: () => void): void
// Equivalent to setTimeout(() => {}, 0) but more predictable
```

### 4.3.2 Signals y EventEmitter

`process` implementa la interfaz EventEmitter completa. Los listeners de señales se registran
igual que en Node.js:

```javascript
// Escuchar señales
process.on('SIGTERM', () => {
    console.log('Received SIGTERM, cleaning up...');
    process.exit(0);
});

process.on('SIGINT', () => {
    console.log('Received Ctrl+C');
    process.exit(0);
});

// Listener al inicio de la cola (prependListener)
process.prependListener('SIGTERM', () => { /* primero en ejecutar */ });

// Listener de una sola vez
process.once('SIGUSR2', () => { /* solo la primera vez */ });

// Remover un listener
process.off('SIGTERM', handler);

// Listar eventos activos
process.eventNames()               // → ['SIGTERM', 'SIGINT', ...]
process.listenerCount('SIGTERM')   // → número de listeners

// Emitir un evento (para testing / integración interna)
process.emit('SIGTERM');

// Señales disponibles:
// SIGTERM, SIGINT, SIGUSR1, SIGUSR2, SIGWINCH, SIGHUP, SIGPIPE
```

### 4.3.3 Métodos adicionales

```javascript
process.uptime()                    // Segundos desde inicio del proceso
process.abort()                     // Equivale a process.exit(1)
process.kill(pid)                   // Si pid === process.pid, llama exit(0)
process.title                       // Getter/setter — valor inicial: 'node'
process.execPath                    // Ruta al binario de 3va
process.execArgv                    // Array de flags de runtime (vacío)

// Callbacks de excepción no capturada
process.setUncaughtExceptionCaptureCallback(fn)
process.hasUncaughtExceptionCaptureCallback()  // → boolean

// Reporte de diagnóstico (stub compatible)
process.report.writeReport()        // → ''
process.report.getReport()          // → {}
```

### 4.3.3 Streams

```javascript
// Standard streams
process.stdin           // Readable stream - standard input
process.stdout          // Writable stream - standard output
process.stderr          // Writable stream - standard error

// Example: reading from stdin
process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => {
    console.log('Received:', chunk);
});
```

### 4.3.4 Process IO

```javascript
// Timings
process.uptime()                   // Seconds since start
process.hrtime()                    // High-resolution time [sec, nanosec]
process.hrtime.bigint()             // In nanoseconds as BigInt

// Ticks
process.uptime();                   // seconds since start
process.resourceUsage();            // system resources

// Configuration
process.title                       // Process title (setter/getter)
process.stderr.fd                   // stderr file descriptor
process.stdout.fd                   // stdout file descriptor
process.stdin.fd                    // stdin file descriptor
```

## 4.4 Process Events

### 4.4.1 Available Events

```javascript
// beforeExit - before exiting the event loop
process.on('beforeExit', (code) => {
    console.log('beforeExit with code:', code);
});

// exit - when the process is about to exit
process.on('exit', (code) => {
    console.log('Process exiting with code:', code);
});

// uncaughtException - unhandled error
process.on('uncaughtException', (err) => {
    console.error('Unhandled exception:', err);
    process.exit(1);
});

// unhandledRejection - unhandled rejected promise
process.on('unhandledRejection', (reason, promise) => {
    console.error('Unhandled rejection:', reason);
});

// warning - runtime warning
process.on('warning', (warning) => {
    console.warn('Warning:', warning.name, warning.message);
});

// message (for IPC)
process.on('message', (message, sendHandle) => { });
```

### 4.4.2 Exit Event Flow

```
1. User calls process.exit(code)
2. Emit 'beforeExit' event with code
3. Execute event loop tasks (including 'exit')
4. Emit 'exit' event with code
5. Clean up resources
6. Terminate process with code
```

## 4.5 child_process

### 4.5.1 Process Spawning

```javascript
// Only available with --allow-child-process

const { spawn } = require('child_process');

// Execute command
const child = spawn('ls', ['-la', '/tmp']);

// Stdout
child.stdout.on('data', (data) => {
    console.log('stdout:', data.toString());
});

// Stderr
child.stderr.on('data', (data) => {
    console.error('stderr:', data.toString());
});

// Exit
child.on('close', (code) => {
    console.log('Child process exited with code:', code);
});
```

### 4.5.2 exec - Command Execution

```javascript
const { exec } = require('child_process');

exec('ls -la', (error, stdout, stderr) => {
    if (error) {
        console.error('Error:', error);
        return;
    }
    console.log('stdout:', stdout);
    console.log('stderr:', stderr);
});

// With promise (if available)
const { execSync } = require('child_process');
const output = execSync('ls -la', { encoding: 'utf8' });
```

## 4.6 Process Permissions

### 4.6.1 Related Permission Flags

| Flag | Allows |
|------|--------|
| --allow-child-process | spawn, exec, execSync |
| --allow-env | process.env |
| --allow-read | process.cwd, fs |
| --allow-write | fs write |

### 4.6.2 Security Example

```bash
# Allow specific child processes
3va run app.ts --allow-child-process

# Deny processes but allow environment
3va run app.ts --allow-env --deny-child-process

# Process with stdin/stdout only
3va run app.ts --allow-child-process
```

---

*Process management conforming to Node.js API and POSIX standards.*
