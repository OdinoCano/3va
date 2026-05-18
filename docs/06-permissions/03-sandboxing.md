# 03 - SANDBOXING Y AISLAMIENTO

## 3.1 Filosofía de Sandboxing

3va implementa múltiples capas de aislamiento para ejecutar código de manera segura, protegiendo el sistema host de operaciones maliciosas o accidentales.

## 3.2 Niveles de Aislamiento

### 3.2.1 Arquitectura de Seguridad

```
┌─────────────────────────────────────────────────────────────────┐
│                        Nivel 0: Host                           │
│                  (Sistema Operativo, Kernel)                   │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                        Nivel 1: Proceso 3va                    │
│              (Binary ejecutable, permisos SO)                  │
│    - Usuario no root                                          │
│    - Caps/Seccomp/BPF filters                                 │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                        Nivel 2: Runtime JS                     │
│              (QuickJS isolate, permisos 3va)                   │
│    - Memory limits                                            │
│    - Execution time limits                                    │
│    - Capability-based access                                  │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                        Nivel 3: Paquetes npm                  │
│              (node_modules sandbox)                             │
│    - Sandbox de archivos                                      │
│    - Sin acceso a red por defecto                             │
│    - Sin post-install scripts                                 │
└─────────────────────────────────────────────────────────────────┘
```

## 3.3 Aislamiento del Runtime

### 3.3.1 Límites de Recursos

```rust
pub struct RuntimeLimits {
    pub max_memory: usize,        // 256MB default
    pub max_stack: usize,          // 1MB default
    pub max_execution_time: Duration,  // Sin límite por defecto
    pub max_cpu_time: Duration,   // Sin límite por defecto
}

impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            max_memory: 256 * 1024 * 1024,
            max_stack: 1024 * 1024,
            max_execution_time: Duration::MAX,
            max_cpu_time: Duration::MAX,
        }
    }
}
```

### 3.3.2 Enforce de Límites

```rust
// Verificación de memoria
pub fn check_memory_limit(&self) -> Result<(), Error> {
    let usage = self.runtime.memory_usage();
    if usage.heap_used > self.limits.max_memory {
        Err(Error::MemoryLimitExceeded)
    } else {
        Ok(())
    }
}

// Timeout de ejecución
pub fn with_timeout<T, F>(
    future: F,
    duration: Duration
) -> Result<T, TimeoutError>
where
    F: Future<Output = T>,
{
    tokio::time::timeout(duration, future)
}
```

### 3.3.3 Thread Isolation

```rust
// Aislamiento de threads para WebAssembly
pub struct IsolatePool {
    pool: VecDeque<Isolate>,
    max_isolates: usize,
}

impl IsolatePool {
    pub fn spawn(&mut self) -> IsolateHandle {
        // Crear nuevo isolate si hay espacio
        // o esperar a que uno se libere
    }
}

// En WASM
let isolate = Isolate::new(isolate_memory);
isolate.enter();
```

## 3.4 Sandboxing de Archivos

### 3.4.1 Virtual File System

```rust
pub struct VirtualFs {
    root: PathBuf,
    mounts: HashMap<PathBuf, MountPoint>,
}

pub struct MountPoint {
    pub source: PathBuf,     // Real path (solo con permisos)
    pub read_only: bool,
    pub max_size: u64,
}

impl VirtualFs {
    // El código JS ve /app como raíz virtual
    // pero está montado en un directorio específico
    pub fn mount(&mut self, virtual_path: &str, source: &str) {
        self.mounts.insert(
            PathBuf::from(virtual_path),
            MountPoint::new(PathBuf::from(source), false),
        );
    }

    pub fn resolve(&self, path: &Path) -> Result<PathBuf, Error> {
        for (vp, mount) in &self.mounts {
            if path.starts_with(vp) {
                let relative = path.strip_prefix(vp).unwrap();
                let real = mount.source.join(relative);
                // Verificar que no hay path traversal
                if real.canonicalize()?.starts_with(mount.source.canonicalize()?) {
                    return Ok(real);
                }
                return Err(Error::PathTraversalBlocked);
            }
        }
        Err(Error::PathNotMounted)
    }
}
```

### 3.4.2 Path Traversal Prevention

```rust
pub fn is_safe_path(base: &Path, target: &Path) -> bool {
    let base_canonical = base.canonicalize().unwrap();
    let target_canonical = target.canonicalize().unwrap();

    // El target debe estar dentro del base
    target_canonical.starts_with(base_canonical) &&
    // No contener secuencias ..
    !target.components().any(|c| c == Component::ParentDir)
}
```

## 3.5 Sandboxing de Red

### 3.5.1 Red Virtual

```rust
pub struct VirtualNetwork {
    allowed_hosts: HashSet<String>,
    dns_resolver: DnsResolver,
    proxy_config: Option<ProxyConfig>,
}

impl VirtualNetwork {
    pub fn connect(&self, host: &str, port: u16) -> Result<Box<dyn Connection>> {
        // 1. Verificar que el host está permitido
        if !self.is_allowed(host) {
            return Err(NetworkError::HostNotAllowed(host.to_string()));
        }

        // 2. Resolver DNS (con cache)
        let ip = self.dns_resolver.resolve(host).await?;

        // 3. Validar IP resolved no es bypass
        if self.is_ip_banned(&ip) {
            return Err(NetworkError::IpBanned(ip));
        }

        // 4. Conectar (a través de proxy si está configurado)
        self.do_connect(&ip, port).await
    }

    fn is_allowed(&self, host: &str) -> bool {
        // Check exacto o wildcard
        self.allowed_hosts.iter().any(|allowed| {
            allowed == host ||
            (allowed.starts_with("*.") && host.ends_with(&allowed[1..]))
        })
    }
}
```

### 3.5.2 Restricciones de DNS

```rust
// Prevenir DNS rebinding attacks
pub fn validate_dns(host: &str, resolved_ip: &IpAddr) -> bool {
    // No permitir localhostresolve como IP pública
    if resolved_ip.is_loopback() && !host.ends_with(".localhost") {
        return false;
    }

    // No permitirlink-local
    if resolved_ip.is_link_local() {
        return false;
    }

    // No permitirmulticast
    if resolved_ip.is_multicast() {
        return false;
    }

    true
}
```

## 3.6 Sandboxing de Paquetes npm

### 3.6.1 Instalación Aislada

```rust
pub struct PackageSandbox {
    base_dir: PathBuf,
    allowed_scripts: HashSet<String>,
    block_post_install: bool,
}

impl PackageSandbox {
    pub fn install(&self, package: &Package) -> Result<(), Error> {
        // 1. Verificar firma del paquete
        self.verify_signature(package)?;

        // 2. Escanear por malware
        self.scan_for_malware(package)?;

        // 3. Extraer en directorio aislado
        let extract_dir = self.base_dir.join(&package.name);
        self.extract(package, &extract_dir)?;

        // 4. NO ejecutar post-install por defecto
        if self.block_post_install {
            self.disable_scripts(&extract_dir)?;
        }

        Ok(())
    }
}
```

### 3.6.2 Ejecución de Scripts

```rust
// Por defecto, denegar scripts de paquetes
pub fn should_run_script(script: &str, package: &PackageInfo) -> bool {
    // Whitelist de scripts seguros
    let safe_scripts = ["prepublish", "prepare"];

    // Por defecto: denegar todos
    // Usuario debe usar --allow-scripts explícitamente
    false
}

// Con flag --allow-scripts=package
pub fn should_run_script_for_package(
    script: &str,
    package: &PackageInfo,
    allowed_packages: &HashSet<String>
) -> bool {
    allowed_packages.contains(&package.name) &&
    !is_dangerous_script(script)
}

fn is_dangerous_script(script: &str) -> bool {
    matches!(script.as_str(), "preinstall" | "install" | "postinstall")
}
```

---

*Sandboxing conforme a principios de Chrome Sandbox, gvisor, y WASI.*