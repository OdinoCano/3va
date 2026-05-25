//! Secret and credential detection in source files.
//!
//! Scans source code for hardcoded credentials using regex patterns.
//! Each finding includes the file path, line number, secret type, severity, and
//! a remediation suggestion (use an environment variable instead).
//!
//! Patterns are derived from the 3va security documentation
//! (`docs/10-security/03-secrets-detection.md`) and extended with additional
//! real-world patterns from truffleHog / git-secrets.

use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::{StringLiteral, TemplateElement};
use oxc_ast_visit::Visit;
use oxc_parser::Parser;
use oxc_span::SourceType;
use regex::Regex;
use serde::{Deserialize, Serialize};

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretFinding {
    pub file: PathBuf,
    pub line: usize,
    /// Short machine-readable type name (e.g. `"aws_access_key"`).
    pub secret_type: String,
    pub severity: Severity,
    /// Redacted snippet showing where the secret was found.
    pub snippet: String,
    pub suggestion: String,
}

// ── Pattern registry ──────────────────────────────────────────────────────────

struct Pattern {
    name: &'static str,
    severity: Severity,
    /// Compiled regex. The first capturing group (if any) is the actual secret
    /// value; the full match is used for the snippet.
    regex: Regex,
    suggestion: &'static str,
}

fn build_patterns() -> Vec<Pattern> {
    let defs: &[(&str, Severity, &str, &str)] = &[
        // ── Cloud providers ───────────────────────────────────────────────────
        (
            "aws_access_key",
            Severity::Critical,
            r"AKIA[0-9A-Z]{16}",
            "Store in AWS_ACCESS_KEY_ID env var or use IAM roles",
        ),
        (
            "aws_secret_key",
            Severity::Critical,
            r#"(?i)aws[_\-\s]*secret[_\-\s]*(?:access[_\-\s]*)?key\s*[=:]\s*["']([A-Za-z0-9/+]{40})["']"#,
            "Store in AWS_SECRET_ACCESS_KEY env var or use IAM roles",
        ),
        (
            "gcp_service_account",
            Severity::Critical,
            r#""type"\s*:\s*"service_account""#,
            "Use GCP Workload Identity or Secret Manager",
        ),
        // ── Source control ────────────────────────────────────────────────────
        (
            "github_token",
            Severity::Critical,
            r"ghp_[A-Za-z0-9]{36}",
            "Use process.env.GITHUB_TOKEN instead",
        ),
        (
            "github_oauth",
            Severity::Critical,
            r"gho_[A-Za-z0-9]{36}",
            "Use process.env.GITHUB_TOKEN instead",
        ),
        (
            "github_app_token",
            Severity::Critical,
            r"ghs_[A-Za-z0-9]{36}",
            "Use process.env.GITHUB_TOKEN instead",
        ),
        (
            "gitlab_token",
            Severity::Critical,
            r"glpat-[A-Za-z0-9\-_]{20}",
            "Use process.env.GITLAB_TOKEN instead",
        ),
        // ── Payment ───────────────────────────────────────────────────────────
        (
            "stripe_secret_key",
            Severity::Critical,
            r"sk_live_[A-Za-z0-9]{24,}",
            "Use process.env.STRIPE_SECRET_KEY instead",
        ),
        (
            "stripe_restricted_key",
            Severity::High,
            r"rk_live_[A-Za-z0-9]{24,}",
            "Use environment variable for Stripe restricted key",
        ),
        // ── Messaging ─────────────────────────────────────────────────────────
        (
            "slack_token",
            Severity::High,
            r"xox[baprs]-[A-Za-z0-9\-]{10,}",
            "Use process.env.SLACK_TOKEN instead",
        ),
        (
            "sendgrid_api_key",
            Severity::High,
            r"SG\.[A-Za-z0-9_\-]{22,}\.[A-Za-z0-9_\-]{43,}",
            "Use process.env.SENDGRID_API_KEY instead",
        ),
        (
            "twilio_account_sid",
            Severity::High,
            r"AC[0-9a-fA-F]{32}",
            "Use process.env.TWILIO_ACCOUNT_SID instead",
        ),
        // ── PKI / TLS ─────────────────────────────────────────────────────────
        (
            "private_key_pem",
            Severity::Critical,
            r"-----BEGIN (?:RSA |EC |DSA |OPENSSH )?PRIVATE KEY-----",
            "Remove private key from source; load from secure vault or env var",
        ),
        (
            "private_key_pkcs8",
            Severity::Critical,
            r"-----BEGIN ENCRYPTED PRIVATE KEY-----",
            "Remove private key from source; load from secure vault",
        ),
        // ── JWT ───────────────────────────────────────────────────────────────
        (
            "jwt",
            Severity::High,
            r"eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
            "Never hardcode JWTs; they often contain sensitive claims",
        ),
        // ── NPM (must precede generic secret/token pattern) ──────────────────
        (
            "npm_token",
            Severity::Critical,
            r"npm_[A-Za-z0-9]{36}",
            "Use process.env.NPM_TOKEN instead",
        ),
        // ── Generic passwords / API keys ──────────────────────────────────────
        (
            "password_assignment",
            Severity::High,
            r#"(?i)password\s*[=:]\s*['"][^'"]{8,}['"]"#,
            "Use process.env.PASSWORD or a secrets manager",
        ),
        (
            "api_key_assignment",
            Severity::High,
            r#"(?i)api[_\-]?key\s*[=:]\s*['"][A-Za-z0-9]{20,}['"]"#,
            "Use process.env.API_KEY instead",
        ),
        (
            "secret_assignment",
            Severity::Medium,
            r#"(?i)(?:secret|token)\s*[=:]\s*['"][A-Za-z0-9+/=_\-]{16,}['"]"#,
            "Use an environment variable for secrets and tokens",
        ),
        // ── Database connection strings ────────────────────────────────────────
        (
            "db_connection_string",
            Severity::High,
            r"(?i)(?:mongodb|postgres|postgresql|mysql|redis|amqp)://[^:]+:[^@/\s]{4,}@",
            "Use process.env.DATABASE_URL instead; never hardcode credentials in URIs",
        ),
        // ── Env var leaks ─────────────────────────────────────────────────────
        // Literal assignment of known sensitive env var names
        (
            "sensitive_env_var",
            Severity::Medium,
            r#"(?i)(?:AWS_SECRET_ACCESS_KEY|GITHUB_TOKEN|GH_TOKEN|GITLAB_TOKEN|NPM_TOKEN|STRIPE_SECRET_KEY|SENDGRID_API_KEY|TWILIO_AUTH_TOKEN)\s*[=:]\s*['"][^'"]{4,}['"]"#,
            "Never hardcode environment variable values in source code",
        ),
    ];

    defs.iter()
        .filter_map(|(name, severity, pattern, suggestion)| {
            Regex::new(pattern).ok().map(|regex| Pattern {
                name,
                severity: severity.clone(),
                regex,
                suggestion,
            })
        })
        .collect()
}

// ── Scanner ───────────────────────────────────────────────────────────────────

fn get_line(source: &str, byte_offset: usize) -> usize {
    source[..byte_offset].chars().filter(|&c| c == '\n').count() + 1
}

struct SecretsVisitor<'a, 'b> {
    patterns: &'b [Pattern],
    findings: &'b mut Vec<SecretFinding>,
    file: &'b Path,
    source: &'a str,
}

impl<'a, 'b> Visit<'a> for SecretsVisitor<'a, 'b> {
    fn visit_string_literal(&mut self, lit: &StringLiteral<'a>) {
        let text = lit.value.as_str();
        for pat in self.patterns {
            if pat.name.ends_with("_assignment") || pat.name == "sensitive_env_var" {
                continue;
            }
            if pat.regex.is_match(text) {
                self.findings.push(SecretFinding {
                    file: self.file.to_path_buf(),
                    line: get_line(self.source, lit.span.start as usize),
                    secret_type: pat.name.to_string(),
                    severity: pat.severity.clone(),
                    snippet: redact(text.trim()),
                    suggestion: pat.suggestion.to_string(),
                });
                break;
            }
        }
    }

    fn visit_template_element(&mut self, elem: &TemplateElement<'a>) {
        let text = elem.value.raw.as_str();
        for pat in self.patterns {
            if pat.name.ends_with("_assignment") || pat.name == "sensitive_env_var" {
                continue;
            }
            if pat.regex.is_match(text) {
                self.findings.push(SecretFinding {
                    file: self.file.to_path_buf(),
                    line: get_line(self.source, elem.span.start as usize),
                    secret_type: pat.name.to_string(),
                    severity: pat.severity.clone(),
                    snippet: redact(text.trim()),
                    suggestion: pat.suggestion.to_string(),
                });
                break;
            }
        }
    }
}

pub struct SecretsScanner {
    patterns: Vec<Pattern>,
}

impl SecretsScanner {
    pub fn new() -> Self {
        Self {
            patterns: build_patterns(),
        }
    }

    /// Scan a single source string.
    ///
    /// `file` is used only for the `SecretFinding.file` field; the content is
    /// provided directly so callers can pre-process (transpile, etc.) if needed.
    pub fn scan_source(&self, source: &str, file: &Path) -> Vec<SecretFinding> {
        let mut findings: Vec<SecretFinding> = Vec::new();

        let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
        let is_js = matches!(ext, "js" | "ts" | "mjs" | "cjs" | "jsx" | "tsx");

        if is_js {
            let allocator = Allocator::default();
            let source_type = SourceType::default()
                .with_module(true)
                .with_typescript(ext.ends_with("ts") || ext.ends_with("tsx"));
            let ret = Parser::new(&allocator, source, source_type).parse();

            if ret.errors.is_empty() {
                let mut visitor = SecretsVisitor {
                    patterns: &self.patterns,
                    findings: &mut findings,
                    file,
                    source,
                };
                visitor.visit_program(&ret.program);
            }
        }

        for (line_idx, line) in source.lines().enumerate() {
            let line_no = line_idx + 1;

            let mut text_to_scan = line;
            if let Some(idx) = line.find("//") {
                text_to_scan = &line[..idx];
            } else if let Some(idx) = line.find('#') {
                text_to_scan = &line[..idx];
            }

            let trimmed = text_to_scan.trim();
            if trimmed.is_empty() || trimmed.starts_with('*') || trimmed.starts_with("/*") {
                continue;
            }

            for pat in &self.patterns {
                // If it's JS, token patterns are already checked via AST
                if is_js && !(pat.name.ends_with("_assignment") || pat.name == "sensitive_env_var")
                {
                    continue;
                }

                if pat.regex.is_match(text_to_scan) {
                    findings.push(SecretFinding {
                        file: file.to_path_buf(),
                        line: line_no,
                        secret_type: pat.name.to_string(),
                        severity: pat.severity.clone(),
                        snippet: redact(trimmed),
                        suggestion: pat.suggestion.to_string(),
                    });
                    break;
                }
            }
        }

        findings
    }

    /// Scan a file on disk.  Returns an empty vec if the file cannot be read or
    /// has an extension that is not source-code-like.
    pub fn scan_file(&self, path: &Path) -> Vec<SecretFinding> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        // Only scan text-based source files.
        if !matches!(
            ext,
            "js" | "ts"
                | "mjs"
                | "cjs"
                | "jsx"
                | "tsx"
                | "json"
                | "env"
                | "yaml"
                | "yml"
                | "toml"
                | "sh"
                | "bash"
                | "zsh"
                | "py"
                | "rb"
                | "go"
                | "rs"
        ) {
            return Vec::new();
        }
        match std::fs::read_to_string(path) {
            Ok(source) => self.scan_source(&source, path),
            Err(_) => Vec::new(),
        }
    }

    /// Recursively scan a directory, skipping `node_modules`, `.git`, and
    /// binary-heavy directories.
    pub fn scan_directory(&self, dir: &Path) -> Vec<SecretFinding> {
        let mut findings = Vec::new();
        self.scan_dir_recursive(dir, &mut findings);
        // Sort by file then line for deterministic output.
        findings.sort_by(|a, b| a.file.cmp(&b.file).then(a.line.cmp(&b.line)));
        findings
    }

    fn scan_dir_recursive(&self, dir: &Path, out: &mut Vec<SecretFinding>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if matches!(
                    name.as_ref(),
                    ".git" | "node_modules" | "dist" | "target" | ".cache"
                ) {
                    continue;
                }
                self.scan_dir_recursive(&path, out);
            } else {
                out.extend(self.scan_file(&path));
            }
        }
    }

    /// Returns `true` if no secrets are found in the given source.
    pub fn is_clean(&self, source: &str) -> bool {
        self.scan_source(source, Path::new("<inline>")).is_empty()
    }
}

impl Default for SecretsScanner {
    fn default() -> Self {
        Self::new()
    }
}

/// Redact the middle portion of a line for safe display in findings.
fn redact(line: &str) -> String {
    if line.len() <= 40 {
        // Short line: keep first 8 chars, mask the rest.
        let visible = line.chars().take(8).collect::<String>();
        format!("{}[REDACTED]", visible)
    } else {
        let prefix: String = line.chars().take(16).collect();
        let suffix: String = line
            .chars()
            .rev()
            .take(6)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        format!("{}...[REDACTED]...{}", prefix, suffix)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    fn scanner() -> SecretsScanner {
        SecretsScanner::new()
    }

    fn findings_for(source: &str) -> Vec<SecretFinding> {
        scanner().scan_source(source, Path::new("test.js"))
    }

    // ── AWS ───────────────────────────────────────────────────────────────────

    #[test]
    fn detects_aws_access_key() {
        let src = r#"const key = "AKIAIOSFODNN7EXAMPLE";"#;
        let f = findings_for(src);
        assert!(!f.is_empty(), "should find AWS access key");
        assert_eq!(f[0].secret_type, "aws_access_key");
        assert_eq!(f[0].severity, Severity::Critical);
    }

    // ── GitHub ────────────────────────────────────────────────────────────────

    #[test]
    fn detects_github_token() {
        let src = r#"const token = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";"#;
        let f = findings_for(src);
        assert!(!f.is_empty(), "should find GitHub token");
        assert_eq!(f[0].secret_type, "github_token");
    }

    // ── Stripe ────────────────────────────────────────────────────────────────

    #[test]
    fn detects_stripe_secret_key() {
        let key = format!("sk_{}{}", "live_", "abcdefghijklmnopqrstuvwx");
        let src = format!(r#"const stripe = require('stripe')("{}");"#, key);
        let f = findings_for(&src);
        assert!(!f.is_empty(), "should find Stripe secret key");
        assert_eq!(f[0].secret_type, "stripe_secret_key");
    }

    // ── Private key ───────────────────────────────────────────────────────────

    #[test]
    fn detects_pem_private_key() {
        let src = r#"const pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIE...";"#;
        let f = findings_for(src);
        assert!(!f.is_empty(), "should find PEM private key");
        assert_eq!(f[0].secret_type, "private_key_pem");
        assert_eq!(f[0].severity, Severity::Critical);
    }

    // ── JWT ───────────────────────────────────────────────────────────────────

    #[test]
    fn detects_jwt() {
        let src = r#"const auth = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";"#;
        let f = findings_for(src);
        assert!(!f.is_empty(), "should find JWT");
        assert_eq!(f[0].secret_type, "jwt");
    }

    // ── Generic password ──────────────────────────────────────────────────────

    #[test]
    fn detects_hardcoded_password() {
        let src = r#"const password = "superSecret123!";"#;
        let f = findings_for(src);
        assert!(!f.is_empty(), "should find hardcoded password");
        assert_eq!(f[0].secret_type, "password_assignment");
    }

    // ── Database URL ──────────────────────────────────────────────────────────

    #[test]
    fn detects_db_connection_string() {
        let src = r#"const db = "postgresql://admin:hunter2@db.example.com/mydb";"#;
        let f = findings_for(src);
        assert!(!f.is_empty(), "should find DB connection string");
        assert_eq!(f[0].secret_type, "db_connection_string");
    }

    // ── NPM token ─────────────────────────────────────────────────────────────

    #[test]
    fn detects_npm_token() {
        let src = r#"const token = "npm_abcdefghijklmnopqrstuvwxyz0123456789AB";"#;
        let f = findings_for(src);
        assert!(!f.is_empty(), "should find NPM token");
        assert_eq!(f[0].secret_type, "npm_token");
    }

    // ── Clean code passes ─────────────────────────────────────────────────────

    #[test]
    fn clean_code_has_no_findings() {
        let src = r#"
            const token = process.env.GITHUB_TOKEN;
            const stripe = new Stripe(process.env.STRIPE_SECRET_KEY);
            const db = process.env.DATABASE_URL;
        "#;
        assert!(scanner().is_clean(src), "env-var usage should be clean");
    }

    #[test]
    fn comments_are_not_scanned() {
        // Example tokens inside comments should not trigger findings.
        let src = r#"
            // The GitHub token looks like: ghp_abcdefghijklmnopqrstuvwxyz0123456789
            // Use process.env.GITHUB_TOKEN instead.
        "#;
        assert!(
            scanner().is_clean(src),
            "comment lines must not trigger findings"
        );
    }

    // ── Suggestion is always set ──────────────────────────────────────────────

    #[test]
    fn finding_has_suggestion() {
        let src = r#"const key = "AKIAIOSFODNN7EXAMPLE";"#;
        let f = findings_for(src);
        assert!(!f[0].suggestion.is_empty());
    }

    // ── File scan ─────────────────────────────────────────────────────────────

    #[test]
    fn scan_file_with_secret() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.js");
        let mut file = std::fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"const token = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";"#
        )
        .unwrap();

        let f = scanner().scan_file(&path);
        assert!(!f.is_empty());
        assert_eq!(f[0].line, 1);
    }

    #[test]
    fn scan_file_skips_binary_extensions() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("image.png");
        std::fs::write(&path, b"AKIAIOSFODNN7EXAMPLE").unwrap();
        let f = scanner().scan_file(&path);
        assert!(f.is_empty(), "binary files must not be scanned");
    }

    #[test]
    fn scan_directory_finds_secrets_recursively() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("src");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(
            sub.join("db.js"),
            r#"const pw = "postgresql://u:password123@host/db";"#,
        )
        .unwrap();
        std::fs::write(sub.join("clean.js"), "module.exports = {};").unwrap();

        let f = scanner().scan_directory(dir.path());
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].secret_type, "db_connection_string");
    }

    #[test]
    fn line_numbers_are_accurate() {
        let src = "const a = 1;\nconst b = 2;\nconst key = \"AKIAIOSFODNN7EXAMPLE\";\n";
        let f = findings_for(src);
        assert_eq!(f[0].line, 3, "finding must be on line 3");
    }
}
