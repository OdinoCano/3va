# 02 - APLICACIÓN DE POLÍTICAS

## 2.1 Sistema de Enforcement

El sistema de enforcement aplica las políticas de permisos en tiempo de ejecución, interceptando operaciones sensibles y verificando contra el PermissionState.

## 2.2 Puntos de Intercepción

### 2.2.1 Puntos de Verificación

```
┌─────────────────────────────────────────────────────────────────┐
│                        Usuario Code                            │
│         (JavaScript/TypeScript en el runtime)                 │
└─────────────────────────┬───────────────────────────────────────┘
                          │
    ┌─────────────────────┴─────────────────────┐
    │                                         │
    ▼                                         ▼
┌─────────────┐                        ┌─────────────┐
│  filesystem │                        │   network   │
│   .read()   │                        │  fetch()   │
└──────┬──────┘                        └──────┬──────┘
       │                                        │
       ▼                                        ▼
┌─────────────┐                        ┌─────────────┐
│   fs_hook   │                        │ net_hook    │
│  (verifier) │                        │ (verifier)  │
└──────┬──────┘                        └──────┬──────┘
       │                                        │
       ▼                                        ▼
┌───────────────────────────────────────────────────────────────┐
│                   PermissionState                             │
│              (verificación de capabilities)                   │
└───────────────────────────────────────────────────────────────┘
                          │
                     ┌────┴────┐
                     │ ALLOW   │ DENY
                     ▼         ▼
              ┌─────────┐  ┌─────────┐
              │ ejecutar│  │ throw   │
              │ operación│  │ Security│
              └─────────┘  └─────────┘
```

### 2.2.2 Operaciones Interceptadas

| Módulo | Operación | Capability Requerida |
|--------|-----------|---------------------|
| fs.readFile | Leer archivo | FileRead |
| fs.writeFile | Escribir archivo | FileWrite |
| fs.readDir | Listar directorio | FileRead |
| fetch | HTTP request | Network |
| net.connect | TCP/UDP | Network |
| process.env | Leer entorno | EnvAccess |
| child_process.spawn | Crear proceso | SpawnProcess |

## 2.3 FileSystem Enforcement

### 2.3.1 Implementación del Hook

```rust
pub struct FsEnforcer {
    permission_state: Arc<PermissionState>,
}

impl FsEnforcer {
    pub fn new(state: PermissionState) -> Self {
        Self {
            permission_state: Arc::new(state),
        }
    }

    pub fn check_read(&self, path: &Path) -> Result<(), PermissionError> {
        let cap = Capability::FileRead(path.to_path_buf());

        if self.permission_state.check(&cap) {
            Ok(())
        } else {
            Err(PermissionError::FileReadDenied {
                path: path.to_path_buf(),
            })
        }
    }

    pub fn check_write(&self, path: &Path) -> Result<(), PermissionError> {
        let cap = Capability::FileWrite(path.to_path_buf());

        if self.permission_state.check(&cap) {
            Ok(())
        } else {
            Err(PermissionError::FileWriteDenied {
                path: path.to_path_buf(),
            })
        }
    }

    pub fn check_read_recursive(&self, path: &Path) -> Result<(), PermissionError> {
        // Para operaciones que leen recursively
        // Verificar el path base
        for cap in &self.permission_state.granted {
            if let Capability::FileRead(allowed) = cap {
                if path.starts_with(allowed) || allowed.starts_with(path) {
                    return Ok(());
                }
            }
        }

        Err(PermissionError::FileReadDenied {
            path: path.to_path_buf(),
        })
    }
}
```

### 2.3.2 Integración con Polyfills

```rust
// En el polyfill de fs
pub fn read_file_sync(path: &str) -> Result<String, Error> {
    // 1. Verificar permisos
    enforcer.check_read(Path::new(path))?;

    // 2. Si está permitido, ejecutar operación
    std::fs::read_to_string(path)
}
```

## 2.4 Network Enforcement

### 2.4.1 Verificación de Red

```rust
pub struct NetEnforcer {
    permission_state: Arc<PermissionState>,
}

impl NetEnforcer {
    pub fn check_connect(&self, host: &str, port: u16) -> Result<(), PermissionError> {
        let cap = Capability::Network(host.to_string());

        if self.permission_state.check(&cap) {
            Ok(())
        } else {
            Err(PermissionError::NetworkDenied {
                host: host.to_string(),
                port,
            })
        }
    }

    pub fn check_url(&self, url: &Url) -> Result<(), PermissionError> {
        let host = url.host_str().ok_or_else(|| {
            PermissionError::InvalidUrl(url.to_string())
        })?;

        self.check_connect(host, url.port().unwrap_or(80))
    }
}
```

### 2.4.2 fetch Interception

```rust
// Polyfill de fetch con verificación
pub async fn secure_fetch(url: &str, init: RequestInit) -> Result<Response> {
    let parsed_url = Url::parse(url)?;

    // Verificar permiso
    enforcer.check_url(&parsed_url)?;

    // Validaciones de seguridad adicionales
    validate_no_malicious_redirects(&parsed_url)?;
    validate_content_length(init.body)?;

    // Ejecutar fetch real
    native_fetch(url, init).await
}
```

## 2.5 Environment Enforcement

### 2.5.1 Acceso a Variables de Entorno

```rust
pub struct EnvEnforcer {
    permission_state: Arc<PermissionState>,
    allowed_vars: HashSet<String>,
}

impl EnvEnforcer {
    pub fn get(&self, key: &str) -> Result<Option<String>, PermissionError> {
        if !self.permission_state.check(&Capability::EnvAccess) {
            return Err(PermissionError::EnvAccessDenied);
        }

        // Opcional:限制 allowed_vars
        if !self.allowed_vars.is_empty() && !self.allowed_vars.contains(key) {
            return Err(PermissionError::EnvVarNotAllowed(key.to_string()));
        }

        Ok(std::env::var(key).ok())
    }

    pub fn all(&self) -> Result<HashMap<String, String>, PermissionError> {
        if !self.permission_state.check(&Capability::EnvAccess) {
            return Err(PermissionError::EnvAccessDenied);
        }

        Ok(std::env::vars().collect())
    }
}
```

## 2.6 Proceso Enforcement

### 2.6.1 Spawn de Procesos

```rust
pub struct ProcessEnforcer {
    permission_state: Arc<PermissionState>,
    allowed_commands: HashSet<String>,
}

impl ProcessEnforcer {
    pub fn spawn(&self, cmd: &str, args: &[String]) -> Result<(), PermissionError> {
        if !self.permission_state.check(&Capability::SpawnProcess) {
            return Err(PermissionError::ProcessSpawnDenied);
        }

        // Verificar si el comando está en lista blanca
        if !self.allowed_commands.is_empty() && !self.allowed_commands.contains(cmd) {
            return Err(PermissionError::CommandNotAllowed(cmd.to_string()));
        }

        Ok(())
    }
}
```

## 2.7 Manejo de Errores

### 2.7.1 Tipos de Errores

```rust
#[derive(Error, Debug)]
pub enum PermissionError {
    #[error("Permission denied: FileRead({path})")]
    FileReadDenied { path: PathBuf },

    #[error("Permission denied: FileWrite({path})")]
    FileWriteDenied { path: PathBuf },

    #[error("Permission denied: Network({host}:{port})")]
    NetworkDenied { host: String, port: u16 },

    #[error("Permission denied: EnvAccess")]
    EnvAccessDenied,

    #[error("Permission denied: ProcessSpawn")]
    ProcessSpawnDenied,

    #[error("Environment variable not allowed: {0}")]
    EnvVarNotAllowed(String),

    #[error("Command not allowed: {0}")]
    CommandNotAllowed(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
}
```

### 2.7.2 Throw en JavaScript

```rust
// Convertir error de permisos a error JavaScript
pub fn throw_permission_error(ctx: &Context, error: PermissionError) {
    ctx.with(|ctx| {
        let error_msg = error.to_string();
        let _ = ctx.eval(&format!(
            "throw new Error('{}: {}')",
            error.category(),
            error_msg
        ));
    });
}
```

## 2.8 Cumplimiento Normativo (ISO/IEC)

Este diseño de Enforcers se alinea estrictamente con los controles de seguridad de la información **ISO/IEC 27002**:
- Las políticas de segmentación de red (`NetEnforcer`) garantizan la protección contra exposición no autorizada.
- La interceptación del sistema de archivos (`FsEnforcer`) apoya el cumplimiento de protección de medios de almacenamiento.
- El modelo basado en capacidades asegura una implementación rigurosa del **Mínimo Privilegio (Least Privilege)** y **Defensa en Profundidad (Defense in Depth)** requeridos por la norma.

---

*Implementado íntegramente en `crates/permissions/src/enforcement.rs`.*