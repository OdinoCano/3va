// Pruebas de integración para el pipeline de seguridad del PM:
// malware scanner, secrets scanner, y auditor (sin red).
//
// Ejecutar: cargo test -p vvva_pm --test security_integration

use std::fs;
use tempfile::TempDir;
use vvva_pm::{
    auditor::run_audit_in,
    malware_scanner::MalwareScanner,
    secrets::{SecretsScanner, Severity as SecretSeverity},
};

// ── Malware scanner ──────────────────────────────────────────────────────────

#[test]
fn malware_scanner_clean_package_is_safe() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("index.js"),
        r#"
        function add(a, b) { return a + b; }
        module.exports = { add };
        "#,
    )
    .unwrap();

    let scanner = MalwareScanner::new();
    assert!(
        scanner.is_package_safe(dir.path()),
        "paquete limpio debe ser considerado seguro"
    );
}

#[test]
fn malware_scanner_detects_fork_bomb() {
    let dir = TempDir::new().unwrap();
    // Patrón clásico de fork bomb en shell ejecutado desde postinstall
    fs::write(
        dir.path().join("postinstall.js"),
        r#"require('child_process').exec(':(){ :|:& };:');"#,
    )
    .unwrap();

    let scanner = MalwareScanner::new();
    let result = scanner.scan_file(&dir.path().join("postinstall.js"));
    assert!(
        !result.threats.is_empty(),
        "fork bomb debe detectarse como amenaza"
    );
}

#[test]
fn malware_scanner_detects_curl_pipe_sh() {
    let dir = TempDir::new().unwrap();
    // El scanner busca la cadena literal "curl | sh"
    fs::write(
        dir.path().join("install.js"),
        r#"// postinstall: curl | sh"#,
    )
    .unwrap();

    let scanner = MalwareScanner::new();
    let result = scanner.scan_file(&dir.path().join("install.js"));
    assert!(
        !result.threats.is_empty(),
        "\"curl | sh\" debe detectarse como amenaza"
    );
}

#[test]
fn malware_scanner_scan_directory_aggregates_results() {
    let dir = TempDir::new().unwrap();

    // Un archivo limpio y uno con rm -rf / (patrón exacto del scanner)
    fs::write(dir.path().join("clean.js"), "const x = 1;").unwrap();
    fs::write(dir.path().join("malicious.js"), "// danger: rm -rf /").unwrap();

    let scanner = MalwareScanner::new();
    let results = scanner.scan_directory(dir.path());

    let has_threat = results.iter().any(|r| !r.threats.is_empty());
    assert!(has_threat, "debe detectar amenaza en al menos un archivo");
    assert!(
        !scanner.is_package_safe(dir.path()),
        "directorio no es seguro"
    );
}

// ── Secrets scanner ──────────────────────────────────────────────────────────

#[test]
fn secrets_scanner_clean_source_is_clean() {
    let scanner = SecretsScanner::new();
    assert!(
        scanner.is_clean("const PI = 3.14159;"),
        "código sin secretos debe ser limpio"
    );
}

#[test]
fn secrets_scanner_detects_aws_key() {
    let scanner = SecretsScanner::new();
    let source = r#"const key = "AKIAIOSFODNN7EXAMPLE";"#;
    assert!(
        !scanner.is_clean(source),
        "clave AWS debe detectarse como secreto"
    );
}

#[test]
fn secrets_scanner_detects_github_token() {
    let scanner = SecretsScanner::new();
    // Formato real de GitHub PAT clásico
    let source = r#"const token = "ghp_1234567890abcdefghijklmnopqrstuvwxyz";"#;
    let findings = scanner.scan_source(source, std::path::Path::new("config.js"));
    assert!(
        !findings.is_empty(),
        "token de GitHub debe detectarse: {:?}",
        findings
    );
}

#[test]
fn secrets_scanner_severity_levels_are_set() {
    let scanner = SecretsScanner::new();
    let source = r#"const AWS_SECRET = "AKIAIOSFODNN7EXAMPLE";"#;
    let findings = scanner.scan_source(source, std::path::Path::new("test.js"));

    assert!(!findings.is_empty());
    let has_high_or_critical = findings
        .iter()
        .any(|f| matches!(f.severity, SecretSeverity::High | SecretSeverity::Critical));
    assert!(
        has_high_or_critical,
        "clave AWS debe tener severidad High o Critical"
    );
}

#[test]
fn secrets_scanner_scan_file_reads_and_reports() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.js");
    fs::write(
        &path,
        r#"module.exports = { apiKey: "AKIAIOSFODNN7EXAMPLE" };"#,
    )
    .unwrap();

    let scanner = SecretsScanner::new();
    let findings = scanner.scan_file(&path);
    assert!(
        !findings.is_empty(),
        "scan_file debe reportar el secreto en el archivo"
    );
}

// ── Auditor (sin red) ────────────────────────────────────────────────────────

#[tokio::test]
async fn audit_empty_lockfile_returns_clean_report_without_network() {
    let dir = TempDir::new().unwrap();

    // Lockfile válido con cero dependencias → no se realiza ninguna llamada de red
    let lockfile_json = serde_json::json!({
        "lockfileVersion": 3,
        "name": "test-project",
        "version": "0.0.0",
        "packages": {},
        "dependencies": {}
    });
    fs::write(
        dir.path().join("3va-lock.json"),
        serde_json::to_string_pretty(&lockfile_json).unwrap(),
    )
    .unwrap();

    let report = run_audit_in(false, dir.path())
        .await
        .expect("audit debe completar con lockfile vacío");

    assert_eq!(report.total_packages, 0, "cero paquetes en lockfile vacío");
    assert_eq!(report.total_vulns, 0, "sin vulnerabilidades");
    assert!(report.findings.is_empty(), "sin hallazgos");
}

#[tokio::test]
async fn audit_no_lockfile_no_node_modules_returns_error() {
    let dir = TempDir::new().unwrap();
    // Sin lockfile ni node_modules → debe devolver error descriptivo
    let result = run_audit_in(false, dir.path()).await;
    assert!(result.is_err(), "debe fallar sin lockfile ni node_modules");
    let msg = match result {
        Err(e) => format!("{e}"),
        Ok(_) => unreachable!("expected Err from audit without lockfile or node_modules"),
    };
    assert!(
        msg.contains("3va-lock.json") || msg.contains("node_modules"),
        "error debe mencionar lockfile o node_modules: {msg}"
    );
}

#[tokio::test]
async fn audit_lockfile_with_unknown_versions_skips_them() {
    let dir = TempDir::new().unwrap();

    // Paquete con version "unknown" → se filtra antes de consultar la red
    let lockfile_json = serde_json::json!({
        "lockfileVersion": 3,
        "name": "test-project",
        "version": "0.0.0",
        "packages": {},
        "dependencies": {
            "some-pkg": {
                "version": "unknown",
                "resolved": null,
                "integrity": null,
                "dev": false
            }
        }
    });
    fs::write(
        dir.path().join("3va-lock.json"),
        serde_json::to_string_pretty(&lockfile_json).unwrap(),
    )
    .unwrap();

    // Paquetes con version "unknown" se filtran → cero paquetes a auditar → sin red
    let report = run_audit_in(false, dir.path())
        .await
        .expect("audit debe completar filtrando versiones unknown");

    assert_eq!(
        report.total_packages, 0,
        "versiones unknown deben filtrarse"
    );
    assert_eq!(report.total_vulns, 0);
}
