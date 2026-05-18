# 01 - MODELO DE CAPACIDADES

## 1.1 Filosofía del Modelo

El sistema de permisos de 3va implementa un modelo de capacidades basado en el principio de "denegar por defecto" (deny-by-default), donde ningún proceso tiene acceso a recursos del sistema sin una Capability explícitamente otorgada por el usuario.

## 1.2 Arquitectura del Sistema de Permisos

### 1.2.1 Diagrama de Componentes

```
┌────────────────────────────────────────────────────────────────┐
│                      CLI (Usuario)                            │
│              --allow-read --allow-net --allow-env             │
└─────────────────────────────────┬──────────────────────────────┘
                                  │
                                  ▼
┌────────────────────────────────────────────────────────────────┐
│                    PermissionState                            │
│  ┌─────────────────────────────────────────────────────────┐  │
│  │ Capabilities Concedidas:                                 │  │
│  │   - FileRead(PathBuf)                                   │  │
│  │   - Network(String)                                     │  │
│  │   - EnvAccess                                           │  │
│  │   - (deny-list vacía)                                   │  │
│  └─────────────────────────────────────────────────────────┘  │
└─────────────────────────────────┬──────────────────────────────┘
                                  │
          ┌───────────────────────┼───────────────────────┐
          ▼                       ▼                       ▼
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   FileSystem   │    │   Network      │    │   Environment  │
│   Verifier     │    │   Verifier     │    │   Verifier     │
└─────────────────┘    └─────────────────┘    └─────────────────┘
```

## 1.3 Enum Capability

### 1.3.1 Definición

```rust
// crates/permissions/src/capability.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    /// Permite lectura de archivos en el path especificado
    FileRead(PathBuf),
    /// Permite escritura de archivos en el path especificado
    FileWrite(PathBuf),
    /// Permite conexiones de red al host/IP especificado
    Network(String),
    /// Permite acceso a variables de entorno
    EnvAccess,
    /// Permite crear procesos hijos
    SpawnProcess,
    /// Permite acceso a APIs nativas/FFI
    FFI,
}
```

### 1.3.2 Descripción de Capacidades

| Capability | Recurso | Descripción |
|------------|---------|-------------|
| FileRead | PathBuf | Permite leer archivos/directorios |
| FileWrite | PathBuf | Permite escribir/crear archivos |
| Network | String | Permite conexiones TCP/UDP |
| EnvAccess | - | Permite leer variables de entorno |
| SpawnProcess | - | Permite crear procesos hijos |
| FFI | - | Permite llamadas a funciones nativas |

## 1.4 PermissionState

### 1.4.1 Estructura

```rust
#[derive(Debug, Default)]
pub struct PermissionState {
    /// Lista de capabilities concedidas
    pub granted: Vec<Capability>,
    /// Lista de capacidades explícitamente denegadas
    pub denied: Vec<Capability>,
    /// Flags de denegación global
    deny_all_fs: bool,
    deny_all_net: bool,
    deny_all_env: bool,
    deny_all_process: bool,
}

impl PermissionState {
    pub fn new() -> Self { ... }

    /// Conceder una capability
    pub fn grant(&mut self, cap: Capability) { ... }

    /// Denegar una capability específica
    pub fn deny(&mut self, cap: Capability) { ... }

    /// Verificar si una operación está permitida
    pub fn check(&self, required: &Capability) -> bool { ... }
}
```

### 1.4.2 Algoritmo de Verificación

```
check(required_capability):
    1. SI deny_all_<tipo> ES true:
       RETURN false

    2. SI required_capability ESTÁ en denied:
       RETURN false

    3. PARA cada cap EN granted:
       4. SI cap MATCHES required_capability:
          RETURN true

    5. RETURN false  (deny-by-default)
```

## 1.5 Matching de Patrones

### 1.5.1 File Patterns

```rust
// Soporte para patrones glob en paths
impl Capability {
    pub fn matches_path(&self, path: &PathBuf) -> bool {
        match self {
            Capability::FileRead(allowed) => {
                // Exact match
                path.starts_with(allowed) ||
                // Glob patterns (futuro)
                matches_glob(path, allowed)
            }
            _ => false
        }
    }

    pub fn matches_glob(path: &Path, pattern: &Path) -> bool {
        // Implementación de glob matching
        // *.js -> matches any .js file
        // /app/* -> matches anything in /app
        // /app/**/*.ts -> recursive .ts files
    }
}
```

### 1.5.2 Network Patterns

```rust
// Soporte para patrones de red
impl Capability {
    pub fn matches_host(&self, host: &str) -> bool {
        match self {
            Capability::Network(allowed) => {
                // Exact match
                host == allowed ||
                // Wildcard: *.example.com
                allowed.starts_with("*.") &&
                    host.ends_with(&allowed[1..]) ||
                // CIDR: 192.168.0.0/16 (futuro)
                matches_cidr(host, allowed)
            }
            _ => false
        }
    }
}
```

### 1.5.3 Ejemplos de Matching

| Pattern | Match | No Match |
|---------|-------|----------|
| `/app/*` | `/app/file.js` | `/app/sub/file.js` |
| `/app/**` | `/app/file.js`, `/app/sub/file.js` | `/other/file.js` |
| `*.example.com` | `api.example.com` | `example.com`, `evil.com` |
| `api.example.com` | `api.example.com` | `other.example.com` |

## 1.6 Construcción desde CLI

### 1.6.1 Parseo de Flags

```rust
pub fn from_args(args: &Args) -> PermissionState {
    let mut state = PermissionState::new();

    // --allow-read
    if args.flag_allow_read {
        // Permitir todo
        state.grant(Capability::FileRead(PathBuf::from("/")));
    } else if let Some(paths) = &args.flag_allow_read_paths {
        for path in paths {
            state.grant(Capability::FileRead(PathBuf::from(path)));
        }
    }

    // --allow-net
    if args.flag_allow_net {
        state.grant(Capability::Network("*".to_string()));
    } else if let Some(hosts) = &args.flag_allow_net_hosts {
        for host in hosts {
            state.grant(Capability::Network(host.clone()));
        }
    }

    // --allow-env
    if args.flag_allow_env {
        state.grant(Capability::EnvAccess);
    }

    // --allow-child-process
    if args.flag_allow_child_process {
        state.grant(Capability::SpawnProcess);
    }

    // --deny-* (revocar permisos específicos)
    if args.flag_deny_env {
        state.deny(Capability::EnvAccess);
    }

    state
}
```

### 1.6.2 Presets

```rust
pub enum PermissionPreset {
    /// Sin permisos (deny-all)
    None,
    /// Equivalente a Node.js (permite todo)
    Node,
    /// Simula navegador
    Browser,
    /// Entorno restringido
    Minimal,
}

impl PermissionPreset {
    pub fn apply(&self, state: &mut PermissionState) {
        match self {
            PermissionPreset::None => {
                // deny-by-default, sin grants
            }
            PermissionPreset::Node => {
                state.grant(Capability::FileRead(PathBuf::from("/")));
                state.grant(Capability::FileWrite(PathBuf::from("/")));
                state.grant(Capability::Network("*".to_string()));
                state.grant(Capability::EnvAccess);
                state.grant(Capability::SpawnProcess);
            }
            PermissionPreset::Browser => {
                state.grant(Capability::Network("*".to_string()));
                state.grant(Capability::FileRead(PathBuf::from(".")));
                state.grant(Capability::FileWrite(PathBuf::from("./.cache")));
            }
            PermissionPreset::Minimal => {
                // Solo stdio
            }
        }
    }
}
```

---

*Modelo de capacidades conforme a principios de seguridad de Chrome Sandbox y QubesOS.*