# 04 - GESTIÓN DE PROCESOS

## 4.1 Objeto Process

El objeto `process` es un objeto global que proporciona información sobre el proceso de runtime de 3va y permite controlarlo.

## 4.2 Propiedades del Proceso

### 4.2.1 Información del Entorno

```rust
// Propiedades del objeto process
pub struct ProcessInfo {
    pub version: String,           // "3va/x.x.x"
    pub versions: Versions,         // Información detallada de versiones
    pub platform: String,           // "linux", "darwin", "win32"
    pub arch: String,               // "x64", "arm64", etc.
    pub execPath: String,           // Ruta del ejecutable 3va
    pub cwd: String,                // Directorio de trabajo actual
    pub home: String,               // Directorio home del usuario
    pub tmpdir: String,             // Directorio temporal
}
```

```javascript
// Acceso en JavaScript
console.log(process.version);        // "3va/1.0.0"
console.log(process.platform);      // "linux"
console.log(process.arch);           // "x64"
console.log(process.cwd());          // "/home/user/project"
console.log(process.execPath);      // "/usr/local/bin/3va"
```

### 4.2.2 Variables de Entorno

```javascript
// Lectura de entorno
process.env                         // Objeto con todas las variables
process.env.PATH                    // Variable específica
process.env.NODE_ENV                // Ambiente de ejecución

// Con permisos: process.env es accesible solo con --allow-env
```

### 4.2.3 Información de Memoria

```javascript
// Memoria
process.memoryUsage()              // Returns: rss, heapTotal, heapUsed, external

// Ejemplo de salida:
{
    rss: 35287040,
    heapTotal: 20971520,
    heapUsed: 12400704,
    external: 1048576
}

// Memoria en V8 (si disponible)
process.memoryUsage.jsHeap;
```

### 4.2.4 Información de CPU

```javascript
// Uso de CPU
process.cpuUsage()                  // { user: 100000, system: 50000 }
process.cpuUsage(previous)          // Diferencial desde previous
```

## 4.3 Métodos del Proceso

### 4.3.1 Control de Flujo

```javascript
// Salir del proceso
process.exit(code?: number): void
// code: 0 = éxito, otro = error
// Ejemplo:
process.exit(0);   // Salida limpia
process.exit(1);   // Error

// Similar a process.exit pero ejecuta cleanup primero
process.exitCode = 1;
process.exit();    // Usa el código seteado

// nextTick - ejecutar en siguiente iteración del event loop
process.nextTick(callback: () => void): void
// Mayor prioridad que setTimeout(0)
process.nextTick(() => console.log('nextTick'));

// setImmediate - ejecutar en siguiente fase check
process.setImmediate(callback: () => void): void
// Equivalente a setTimeout(() => {}, 0) pero más predecible
```

### 4.3.2 Señales

```javascript
// Manejo de señales
process.on('SIGTERM', () => {
    console.log('Recibido SIGTERM, limpiando...');
    process.exit(0);
});

process.on('SIGINT', () => {
    console.log('Recibido Ctrl+C');
    process.exit(0);
});

// Enviar señal a proceso actual
process.kill(process.pid, 'SIGTERM');

// Señales disponibles:
'SIGTERM', 'SIGINT', 'SIGUSR1', 'SIGUSR2', 'SIGWINCH'
```

### 4.3.3 Streams

```javascript
// Streams estándar
process.stdin           // Readable stream - entrada estándar
process.stdout          // Writable stream - salida estándar
process.stderr          // Writable stream - error estándar

// Ejemplo: leer de stdin
process.stdin.setEncoding('utf8');
process.stdin.on('data', (chunk) => {
    console.log('Recibido:', chunk);
});
```

### 4.3.4 IO del Proceso

```javascript
// Tiempos
process.uptime()                   // Segundos desde inicio
process.hrtime()                    // Tiempo de alta precisión [seg, nanoseg]
process.hrtime.bigint()             // En nanasegundos como BigInt

// Ticks
process.uptime();                   // seconds since start
process.resourceUsage();            // recursos del sistema

// Configuración
process.title                       // Título del proceso (setter/getter)
process.stderr.fd                   // File descriptor de stderr
process.stdout.fd                   // File descriptor de stdout
process.stdin.fd                    // File descriptor de stdin
```

## 4.4 Eventos del Proceso

### 4.4.1 Eventos Disponibles

```javascript
// beforeExit - antes de salir del event loop
process.on('beforeExit', (code) => {
    console.log('beforeExit con código:', code);
});

// exit - cuando el proceso está por salir
process.on('exit', (code) => {
    console.log('Proceso saliendo con código:', code);
});

// uncaughtException - error no manejado
process.on('uncaughtException', (err) => {
    console.error('Excepción no manejada:', err);
    process.exit(1);
});

// unhandledRejection - promesa rechazada no manejada
process.on('unhandledRejection', (reason, promise) => {
    console.error('Rechazo no manejado:', reason);
});

// warning - advertencia del runtime
process.on('warning', (warning) => {
    console.warn('Advertencia:', warning.name, warning.message);
});

// message (para IPC)
process.on('message', (message, sendHandle) => { });
```

### 4.4.2 Flujo de Eventos de Salida

```
1. Usuario llama process.exit(code)
2. Emitir evento 'beforeExit' con código
3. Ejecutar tareas del event loop (incluyendo 'exit')
4. Emitir evento 'exit' con código
5. Limpiar recursos
6. Terminar proceso con código
```

## 4.5 child_process

### 4.5.1 Spawn de Procesos

```javascript
// Solo disponible con --allow-child-process

const { spawn } = require('child_process');

// Ejecutar comando
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
    console.log('Proceso hijo salió con código:', code);
});
```

### 4.5.2 exec - Ejecución de Comandos

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

// Con promise (si disponível)
const { execSync } = require('child_process');
const output = execSync('ls -la', { encoding: 'utf8' });
```

## 4.6 Permisos de Proceso

### 4.6.1 Flags de Permisos Relacionados

| Flag | Permite |
|------|---------|
| --allow-child-process | spawn, exec, execSync |
| --allow-env | process.env |
| --allow-read | process.cwd, fs |
| --allow-write | fs write |

### 4.6.2 Ejemplo de Seguridad

```bash
# Permitir procesos hijos específicos
3va run app.ts --allow-child-process

# Denegar procesos pero permitir entorno
3va run app.ts --allow-env --deny-child-process

# Proceso solo con stdin/stdout
3va run app.ts --allow-child-process
```

---

*Gestión de procesos conforme a Node.js API y estándares POSIX.*