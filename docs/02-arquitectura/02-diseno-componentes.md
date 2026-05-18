# 02 - DISEÑO DE COMPONENTES

## 2.1 Componente: vvva_core

### 2.1.1 Descripción
El componente core proporciona el runtime asíncrono basado en Tokio, gestionando el event loop principal, el scheduling de tareas y la coordinación entre componentes.

### 2.1.2 Estructura

```rust
pub struct Runtime {
    pub permissions: PermissionState,
    event_loop: EventLoop,
    scheduler: Scheduler,
    module_cache: ModuleCache,
}
```

### 2.1.3 Responsabilidades
- Inicialización y gestión del event loop asíncrono
- Scheduling de tareas concurrentes
- Coordinación de módulos cargados
- Gestión del ciclo de vida del proceso

### 2.1.4 Interfaces

#### run()
```rust
pub async fn run(&self) -> anyhow::Result<()>
```
Inicia el event loop principal y espera la finalización de tareas.

#### spawn_task()
```rust
pub fn spawn_task(&self, task: Task) -> Handle
```
Crea una nueva tarea asíncrona y devuelve un handle para controlarla.

### 2.1.5 Dependencias
- tokio (async runtime)
- vvva_permissions (verificación de capacidades)
- vvva_js (ejecución de código)

## 2.2 Componente: vvva_cli

### 2.2.1 Descripción
El componente CLI proporciona la interfaz de línea de comandos, parseando argumentos y enrutando comandos a los componentes apropiados.

### 2.2.2 Estructura

```rust
pub struct Cli {
    command: Command,
    permissions: PermissionState,
    config: Config,
}
```

### 2.2.3 Subcomandos Soportados

| Comando | Descripción | Ejemplo |
|---------|-------------|---------|
| run | Ejecuta un archivo JS/TS | `3va run app.ts` |
| install | Instala un paquete | `3va install axios` |
| test | Ejecuta tests | `3va test` |
| build | Empaqueta código | `3va build index.ts` |
| eval | Evalúa código inline | `3va eval "console.log(1)"` |

### 2.2.4 Flags de Permisos

| Flag | Descripción | Ejemplo |
|------|-------------|---------|
| --allow-read | Permite lectura de archivos | `--allow-read=/app` |
| --allow-write | Permite escritura de archivos | `--allow-write=/tmp` |
| --allow-net | Permite acceso a red | `--allow-net=api.example.com` |
| --allow-env | Permite acceso a variables de entorno | `--allow-env` |
| --allow-child-process | Permite spawn de procesos | `--allow-child-process` |
| --deny-* | Deniega un permiso específico | `--deny-env` |

## 2.3 Componente: vvva_permissions

### 2.3.1 Descripción
El sistema de permisos implementa el modelo de capabilities, almacenando y verificando los permisos granted por el usuario.

### 2.3.2 Estructura

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    FileRead(PathBuf),
    FileWrite(PathBuf),
    Network(String), // Hostname o IP
    SpawnProcess,
    EnvAccess,
}

pub struct PermissionState {
    pub granted: Vec<Capability>,
}
```

### 2.3.3 Algoritmo de Verificación

```
1. Receive operation request (op_type, resource)
2. FOR each capability IN granted:
   3. IF capability matches op_type AND resource:
      4. RETURN ALLOW
5. RETURN DENY
```

### 2.3.4 Matching de Patrones

Los permisos de red y archivo soportan patrones glob:
- `*.example.com` - cualquier subdominio
- `/app/*` - cualquier archivo en /app
- `192.168.*` - cualquier IP en el rango

## 2.4 Componente: vvva_js

### 2.4.1 Descripción
El componente JS integra QuickJS, proporcionando la ejecución de JavaScript y TypeScript con soporte para módulos y APIs web.

### 2.4.2 Estructura

```rust
pub struct JsEngine {
    runtime: Runtime,
    context: Context,
    module_loader: ModuleLoader,
    polyfills: PolyfillRegistry,
}
```

### 2.4.3 Funcionalidades
- Ejecución de código JavaScript/TypeScript
- Soporte ESM y CommonJS
- Polyfills para Node.js APIs
- APIs web estándar (fetch, WebSocket, etc.)

## 2.5 Componente: vvva_pm

### 2.5.1 Descripción
El gestor de paquetes maneja la instalación de dependencias con verificación de seguridad.

### 2.5.2 Estructura

```rust
pub struct PackageManager {
    registry: RegistryClient,
    cache: PackageCache,
    verifier: SignatureVerifier,
    sandbox: Sandbox,
}
```

### 2.5.3 Políticas de Seguridad
- Post-install scripts: Deshabilitados por defecto
- Paquetes no confiables hasta verificación
- Ejecución en sandbox aislado

## 2.6 Componente: vvva_bundler [POR IMPLEMENTAR]

### 2.6.1 Descripción
El bundler transpila y empaqueta código TypeScript/JSX para distribución.

### 2.6.2 Funcionalidades Planeadas
- Transpilación TSX/TS a JS
- Tree shaking
- Code splitting
- Source maps

---

*Diseño conforme a IEEE 1012 y arquitectura de componentes.*