# 03 - INTERFACES Y COMUNICACIÓN

## 3.1 Interfaz de Usuario (CLI)

### 3.1.1 Formato de Invocación
```
3va <comando> [opciones] [argumentos]
```

### 3.1.2 Códigos de Salida

| Código | Significado |
|--------|-------------|
| 0 | Ejecución exitosa |
| 1 | Error general |
| 2 | Error de parseo de argumentos |
| 3 | Error de permisos |
| 4 | Error de módulo |
| 5 | Error de runtime |
| 126 | Error de permisos denegados |
| 127 | Comando no encontrado |

### 3.1.3 Formato de Salida

#### Modo Normal
```
3va run app.ts
> Hello, World!
```

#### Modo Verbose
```
3va run app.ts -v
[DEBUG] Loading module: app.ts
[DEBUG] Checking permissions: FileRead(/path/app.ts)
[INFO] Module loaded successfully
> Hello, World!
```

#### Modo JSON (para scripting)
```json
{
  "success": true,
  "output": "Hello, World!",
  "exitCode": 0
}
```

## 3.2 Intercomunicación de Componentes

### 3.2.1 Interfaz Core ↔ Permissions

```rust
// Core solicita verificación de permiso
pub fn check_permission(&self, cap: &Capability) -> bool {
    self.permissions.check(cap)
}
```

### 3.2.2 Interfaz Core ↔ JS

```rust
// Core delega ejecución a JS
pub fn execute(&self, code: &str) -> Result<Value> {
    self.js_engine.eval(code)
}
```

### 3.2.3 Interfaz CLI ↔ Core

```rust
// CLI construye runtime y lo ejecuta
pub fn run_with_permissions(cmd: Command, perms: PermissionState) -> Result<()> {
    let runtime = Runtime::with_permissions(perms);
    runtime.run_command(cmd).await
}
```

## 3.3 Interfaz de Eventos

### 3.3.1 Eventos del Sistema

| Evento | Descripción | Datos |
|--------|-------------|-------|
| runtime.start | Inicio del runtime | timestamp, config |
| runtime.exit | Finalización del runtime | exit_code, duration |
| permission.check | Verificación de permiso | capability, result |
| module.load | Carga de módulo | path, type |
| module.error | Error de módulo | path, error |
| fs.access | Acceso al sistema de archivos | path, operation, allowed |
| net.connect | Conexión de red | host, port, allowed |

### 3.3.2 Formato de Eventos
```rust
pub struct Event {
    pub timestamp: DateTime<Utc>,
    pub event_type: EventType,
    pub payload: serde_json::Value,
}
```

## 3.4 Interfaz de Extensiones

### 3.4.1 Plugins de Seguridad
Los plugins pueden interceptar operaciones para análisis adicional:

```rust
pub trait SecurityPlugin {
    fn on_permission_check(&mut self, cap: &Capability) -> CheckResult;
    fn on_module_load(&mut self, path: &Path) -> LoadResult;
    fn on_fs_access(&mut self, path: &Path, op: FsOp) -> AccessResult;
}
```

### 3.4.2 Hooks de Lifecycle
```rust
pub trait LifecycleHook {
    fn pre_run(&mut self, config: &Config);
    fn post_run(&mut self, result: &RunResult);
    fn on_error(&mut self, error: &Error);
}
```

## 3.5 Interfaz de Configuración

### 3.5.1 Archivo de Configuración
Ubicación: `~/.3va/config.json` o `./.3va.json`

```json
{
  "permissions": {
    "defaults": {
      "allowRead": false,
      "allowWrite": false,
      "allowNet": false,
      "allowEnv": false,
      "allowChildProcess": false
    }
  },
  "pm": {
    "registry": "https://registry.npmjs.org",
    "postInstallScripts": false,
    "verifySignatures": true
  },
  "logging": {
    "level": "info",
    "format": "text"
  }
}
```

---

*Interfaces conforme a ISO/IEC/IEEE 24765 y arquitectura de software.*