# 03 - PACKAGE SANDBOXING
## 3.1 Security Philosophy
3va's package sandboxing system treats all dependencies as potentially untrusted, applying multiple layers of protection.
## 3.2 Security Model
### 3.2.1 Trust Levels
┌─────────────────────────────────────────────────────────────────┐
│                    Trust Level                                │
├─────────────────────────────────────────────────────────────────┤
│  Package from registry ──► Untrusted                            │
│          │                                                          │
│          ▼                                                          │
│  Signature verification ──► Partially Trusted                    │
│          │                                                          │
│          ▼                                                          │
│  Malware scan ───────────► Verified Trust                        │
│          │                                                          │
│          ▼                                                          │
│  Sandbox installation ──► Complete Isolation                     │
└─────────────────────────────────────────────────────────────────┘
## 3.2 Signature Verification
### 3.2.1 Process
pub struct SignatureVerifier {
    trusted_keys: HashSet<String>,
    registry_client: RegistryClient,
}
impl SignatureVerifier {
    pub async fn verify(&self, package: &Package) -> Result<VerificationResult> {
        // 1. Get signature information from registry
        let sig_info = self.registry_client.get_signatures(&package.name, &package.version)
            .await?;
        // 2. If no signature, flag it
        if sig_info.signatures.is_empty() {
            return Ok(VerificationResult::Unsigned {
                package: package.name.clone(),
                recommendation: "Verify manually".to_string(),
            });
        }
        // 3. Verify each signature
        for sig in &sig_info.signatures {
            let key = self.fetch_key(&sig.key_id).await?;
            let valid = self.verify_detached(&sig.signature, &package.tarball, &key)?;
            if !valid {
                return Ok(VerificationResult::InvalidSignature {
                    package: package.name.clone(),
                    reason: "Invalid signature".to_string(),
                });
            }
        }
        // 4. If all signatures are valid
        Ok(VerificationResult::Verified {
            package: package.name.clone(),
            signers: sig_info.signers,
        })
    }
}
### 3.2.2 Configuration
# Enable signature verification
3va install lodash --verify-signatures
# Default configuration
3va config set pm.verifySignatures=true
## 3.3 Malware Scanning
### 3.3.1 Static Analysis
pub struct MalwareScanner {
    signatures: Vec<MalwareSignature>,
    heuristics: Vec<HeuristicRule>,
}
impl MalwareScanner {
    pub fn scan(&self, package: &Package) -> ScanResult {
        let mut findings = Vec::new();
        // 1. Check known malicious files
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
        // 2. Heuristic analysis
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
        // 3. Check suspicious scripts
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
### 3.3.2 Detections
Type	Description
Known malware	Hash matches database
Malicious script	Script executes system commands
Path traversal	Attempt to write outside the directory
Overwrite	Overwrites system files
Network exfiltration	Sends data to unrelated servers
## 3.4 Isolated Installation
### 3.4.1 Directory Structure
project/
├── node_modules/
│   ├── lodash/
│   │   ├── package/
│   │   │   └── ...
│   │   └── 3va-sandbox.json    # Sandbox metadata
│   ├── react/
│   │   └── ...
│   └── .3va-lock
└── package.json
### 3.4.2 Access Restrictions
pub struct PackageSandbox {
    base_path: PathBuf,
    allowed_operations: HashSet<String>,
    blocked_operations: HashSet<String>,
}
impl PackageSandbox {
    pub fn install(&self, package: &Package) -> Result<()> {
        // 1. Create package directory
        let pkg_dir = self.base_path.join(&package.name);
        fs::create_dir_all(&pkg_dir)?;
        // 2. Extract with restrictions
        self.extract_restricted(&package.tarball, &pkg_dir)?;
        // 3. Write sandbox metadata
        self.write_sandbox_metadata(&pkg_dir, package)?;
        // 4. Disable scripts by default
        self.disable_scripts(&pkg_dir)?;
        Ok(())
    }
    fn extract_restricted(&self, tarball: &[u8], dest: &Path) -> Result<()> {
        // Extract tarball with checks
        // - Verify paths do not leave the destination
        // - Verify total size
        // - Verify file types
    }
}
## 3.5 Script Execution
### 3.5.1 Policies
Policy	Description
none (default)	Do not execute any scripts
whitelist	Only scripts on the allowed list
all	Execute all scripts (dangerous)
### 3.5.2 Configuration
# Disable scripts (default)
3va install lodash
# Enable specific scripts
3va install lodash --allow-scripts=build,test
# Enable all scripts (NOT recommended)
3va install lodash --allow-scripts
### 3.5.3 Implementation
pub struct ScriptRunner {
    allowed_scripts: HashSet<String>,
    sandbox: bool,
}
impl ScriptRunner {
    pub fn run(&self, script: &str, cwd: &Path) -> Result<ExitCode> {
        // 1. Check if allowed
        if !self.allowed_scripts.contains(script) {
            return Err(Error::ScriptNotAllowed(script.to_string()));
        }
        // 2. Run in sandbox if enabled
        if self.sandbox {
            self.run_sandboxed(script, cwd)
        } else {
            self.run_direct(script, cwd)
        }
    }
    fn run_sandboxed(&self, script: &str, cwd: &Path) -> Result<ExitCode> {
        // Create process with restricted permissions
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(script);
        cmd.current_dir(cwd);
        // Restricciones de sandbox
        cmd.uid(sandbox_uid);  // Unprivileged user
        cmd.stdin(Stdio::null());
        cmd.env_clear();       // Minimal environment variables
        // Timeout
        match timeout(Duration::from_secs(30), cmd.output()) {
            Ok(Ok(output)) => ...,
            Ok(Err(e)) => Err(e),
            Err(_) => Err(Error::ScriptTimeout),
        }
    }
}
## 3.6 Package Audit
### 3.6.1 Audit Report
# Run audit
3va audit
# Output:
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
### 3.6.2 Vulnerability Database Integration
pub async fn check_vulnerabilities(pkg: &Package) -> Vec<Vulnerability> {
    // Query vulnerability databases
    // - npm audit
    // - OSV (Open Source Vulnerabilities)
    // - GitHub Advisory Database
}
Sandboxing compliant with supply chain security best practices.