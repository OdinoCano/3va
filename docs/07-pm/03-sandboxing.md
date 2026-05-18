# 03 - SANDBOXING DE PAQUETES
## 3.1 Filosofía de Seguridad
El sistema de sandboxing de paquetes de 3va trata todas las dependencias como potencialmente no confiables, aplicando múltiples capas de protección.
## 3.2 Modelo de Seguridad
### 3.2.1 Niveles de Confianza
┌─────────────────────────────────────────────────────────────────┐
│                    Nivel de Confianza                          │
├─────────────────────────────────────────────────────────────────┤
│  Paquete del registry  ──────► No Confiable                    │
│          │                                                          │
│          ▼                                                          │
│  Verificación de firma ──► Parcialmente Confiable               │
│          │                                                          │
│          ▼                                                          │
│  Escaneo de malware ────► Confianza Verificada                  │
│          │                                                          │
│          ▼                                                          │
│  Instalación sandbox ──► Aislamiento Completo                   │
└─────────────────────────────────────────────────────────────────┘
## 3.2 Verificación de Firmas
### 3.2.1 Proceso
pub struct SignatureVerifier {
    trusted_keys: HashSet<String>,
    registry_client: RegistryClient,
}
impl SignatureVerifier {
    pub async fn verify(&self, package: &Package) -> Result<VerificationResult> {
        // 1. Obtener información de firma del registry
        let sig_info = self.registry_client.get_signatures(&package.name, &package.version)
            .await?;
        // 2. Si no hay firma, marcar
        if sig_info.signatures.is_empty() {
            return Ok(VerificationResult::Unsigned {
                package: package.name.clone(),
                recommendation: "Verificar manualmente".to_string(),
            });
        }
        // 3. Verificar cada firma
        for sig in &sig_info.signatures {
            let key = self.fetch_key(&sig.key_id).await?;
            let valid = self.verify_detached(&sig.signature, &package.tarball, &key)?;
            if !valid {
                return Ok(VerificationResult::InvalidSignature {
                    package: package.name.clone(),
                    reason: "Firma no válida".to_string(),
                });
            }
        }
        // 4. Si todas las firmas son válidas
        Ok(VerificationResult::Verified {
            package: package.name.clone(),
            signers: sig_info.signers,
        })
    }
}
### 3.2.2 Configuración
# Habilitar verificación de firmas
3va install lodash --verify-signatures
# Configuración por defecto
3va config set pm.verifySignatures=true
## 3.3 Escaneo de Malware
### 3.3.1 Análisis Estático
pub struct MalwareScanner {
    signatures: Vec<MalwareSignature>,
    heuristics: Vec<HeuristicRule>,
}
impl MalwareScanner {
    pub fn scan(&self, package: &Package) -> ScanResult {
        let mut findings = Vec::new();
        // 1. Verificar archivos conocidos maliciosos
        for (path, content) in &package.files {
            for sig in &self.signatures {
                if sig.matches(path, content) {
                    findings.push(Finding {
                        severity: Severity::Critical,
                        detection: sig.name.clone(),
                        file: path.clone(),
                    });
                }
            }
        }
        // 2. Análisis heurístico
        for (path, content) in &package.files {
            for heuristic in &self.heuristics {
                if heuristic.matches(path, content) {
                    findings.push(Finding {
                        severity: heuristic.severity,
                        detection: heuristic.name.clone(),
                        file: path.clone(),
                    });
                }
            }
        }
        // 3. Verificar scripts sospechosos
        for script in &package.scripts {
            if self.is_suspicious_script(script) {
                findings.push(Finding {
                    severity: Severity::High,
                    detection: "Suspicious script".to_string(),
                    details: script.clone(),
                });
            }
        }
        ScanResult { findings }
    }
}
### 3.3.2 Detecciones
Tipo	Descripción
Malware conocido	Hash coincides con base de datos
Script malicioso	Script ejecuta comandos del sistema
Path traversal	Intento de escribir fuera del directorio
Sobrescritura	Sobrescribe archivos del sistema
Network exfiltration	Envía datos a servidores no relacionados
## 3.4 Instalación Aislada
### 3.4.1 Estructura de Directorios
project/
├── node_modules/
│   ├── lodash/
│   │   ├── package/
│   │   │   └── ...
│   │   └── 3va-sandbox.json    # Metadata de sandbox
│   ├── react/
│   │   └── ...
│   └── .3va-lock
└── package.json
### 3.4.2 Restricciones de Acceso
pub struct PackageSandbox {
    base_path: PathBuf,
    allowed_operations: HashSet<String>,
    blocked_operations: HashSet<String>,
}
impl PackageSandbox {
    pub fn install(&self, package: &Package) -> Result<()> {
        // 1. Crear directorio de paquete
        let pkg_dir = self.base_path.join(&package.name);
        fs::create_dir_all(&pkg_dir)?;
        // 2. Extraer con restricciones
        self.extract_restricted(&package.tarball, &pkg_dir)?;
        // 3. Escribir metadata de sandbox
        self.write_sandbox_metadata(&pkg_dir, package)?;
        // 4. Deshabilitar scripts por defecto
        self.disable_scripts(&pkg_dir)?;
        Ok(())
    }
    fn extract_restricted(&self, tarball: &[u8], dest: &Path) -> Result<()> {
        // Extraer tarball con verificaciones
        // - Verificar paths no salen del destino
        // - Verificar tamaño total
        // - Verificar tipos de archivos
    }
}
## 3.5 Ejecución de Scripts
### 3.5.1 Políticas
Política	Descripción
none (default)	No ejecutar ningún script
whitelist	Solo scripts en lista permitida
all	Ejecutar todos los scripts (peligroso)
### 3.5.2 Configuración
# Deshabilitar scripts (default)
3va install lodash
# Habilitar scripts específicos
3va install lodash --allow-scripts=build,test
# Habilitar todos los scripts (NO recomendado)
3va install lodash --allow-scripts
### 3.5.3 Implementación
pub struct ScriptRunner {
    allowed_scripts: HashSet<String>,
    sandbox: bool,
}
impl ScriptRunner {
    pub fn run(&self, script: &str, cwd: &Path) -> Result<ExitCode> {
        // 1. Verificar si está permitido
        if !self.allowed_scripts.contains(script) {
            return Err(Error::ScriptNotAllowed(script.to_string()));
        }
        // 2. Ejecutar en sandbox si está habilitado
        if self.sandbox {
            self.run_sandboxed(script, cwd)
        } else {
            self.run_direct(script, cwd)
        }
    }
    fn run_sandboxed(&self, script: &str, cwd: &Path) -> Result<ExitCode> {
        // Crear proceso con permisos restringidos
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(script);
        cmd.current_dir(cwd);
        // Restricciones de sandbox
        cmd.uid(sandbox_uid);  // Usuario sin privilegios
        cmd.stdin(Stdio::null());
        cmd.env_clear();       // Variables de entorno mínimas
        // Timeout
        match timeout(Duration::from_secs(30), cmd.output()) {
            Ok(Ok(output)) => ...,
            Ok(Err(e)) => Err(e),
            Err(_) => Err(Error::ScriptTimeout),
        }
    }
}
## 3.6 Auditoría de Paquetes
### 3.6.1 Reporte de Auditoría
# Ejecutar auditoría
3va audit
# Salida:
# === Security Audit ===
# Found 2 vulnerabilities:
#
# HIGH: Prototype Pollution in lodash <4.17.21
#   Package: lodash@4.17.20
#   Fix: Upgrade to lodash@4.17.21
#
# MEDIUM: Regular Expression Denial of Service
#   Package: minimatch@3.0.4
#   Fix: Upgrade to minimatch@3.0.5
### 3.6.2 Integración con DB de Vulnerabilidades
pub async fn check_vulnerabilities(pkg: &Package) -> Vec<Vulnerability> {
    // Consultar bases de datos de vulnerabilidades
    // - npm audit
    // - OSV (Open Source Vulnerabilities)
    // - GitHub Advisory Database
}
Sandboxing conforme a mejores prácticas de supply chain security.