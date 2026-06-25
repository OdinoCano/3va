//! Serde-compatible schema for `3va.config.*`.

use serde::{Deserialize, Serialize};

/// Top-level project configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectConfig {
    pub run: RunConfig,
    pub dev: DevConfig,
    pub test: TestConfig,
    pub audit: AuditConfig,
    pub bundle: BundleConfig,
    pub workspace: WorkspaceConfig,
    pub firewall: FirewallConfig,
}

// ── firewall ──────────────────────────────────────────────────────────────────

/// HTTP server firewall — rate limiting, DDoS/Slowloris/RUDY protection.
/// All fields have safe defaults so adding `firewall: {}` is enough to opt in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FirewallConfig {
    pub enabled: bool,
    /// Token-bucket refill rate: max sustained requests per second per IP.
    #[serde(rename = "rateLimitRps")]
    pub rate_limit_rps: u32,
    /// Burst capacity: requests an IP can fire before the rate limit activates.
    #[serde(rename = "rateLimitBurst")]
    pub rate_limit_burst: u32,
    /// Consecutive violations before an IP is automatically blocked.
    #[serde(rename = "autoBlockThreshold")]
    pub auto_block_threshold: u32,
    /// How long to keep an offending IP blocked, in seconds.
    #[serde(rename = "blockDurationSecs")]
    pub block_duration_secs: u64,
    /// Max simultaneous open connections from a single IP.
    #[serde(rename = "maxConnectionsPerIp")]
    pub max_connections_per_ip: u32,
    /// Max total simultaneous open connections.
    #[serde(rename = "maxConnectionsTotal")]
    pub max_connections_total: u32,
    /// Timeout (ms) to receive the full request line + headers. Stops Slowloris.
    #[serde(rename = "headerTimeoutMs")]
    pub header_timeout_ms: u64,
    /// Timeout (ms) to receive the full request body after headers. Stops RUDY.
    #[serde(rename = "bodyTimeoutMs")]
    pub body_timeout_ms: u64,
    /// Maximum number of HTTP headers per request.
    #[serde(rename = "maxHeaderCount")]
    pub max_header_count: u32,
    /// Maximum combined size of all headers in bytes.
    #[serde(rename = "maxHeaderBytes")]
    pub max_header_bytes: u32,
    /// Maximum body size in bytes (0 = runtime default of 100 MB).
    #[serde(rename = "maxBodyBytes")]
    pub max_body_bytes: u32,
    /// Minimum body receive rate in bytes per second. Connections slower than
    /// this are dropped (RUDY mitigation). 0 = disabled.
    #[serde(rename = "minBodyRateBps")]
    pub min_body_rate_bps: u32,
}

impl Default for FirewallConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            rate_limit_rps: 100,
            rate_limit_burst: 200,
            auto_block_threshold: 10,
            block_duration_secs: 300,
            max_connections_per_ip: 50,
            max_connections_total: 10_000,
            header_timeout_ms: 10_000,
            body_timeout_ms: 30_000,
            max_header_count: 100,
            max_header_bytes: 16_384,
            max_body_bytes: 0,
            min_body_rate_bps: 100,
        }
    }
}

// ── run ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RunConfig {
    pub permissions: RunPermissions,
    /// Default `--inspect` address when the flag is passed without a value.
    pub inspect: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RunPermissions {
    pub net: Vec<String>,
    pub read: Vec<String>,
    pub write: Vec<String>,
    pub env: Vec<String>,
    #[serde(rename = "childProcess")]
    pub child_process: bool,
    pub ffi: Vec<String>,
}

// ── dev ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DevConfig {
    pub port: u16,
    pub host: String,
    #[serde(rename = "publicDir")]
    pub public_dir: String,
    pub open: bool,
    /// Content-Security-Policy settings for the dev server.
    pub csp: CspConfig,
}

impl Default for DevConfig {
    fn default() -> Self {
        Self {
            port: 3000,
            host: "127.0.0.1".to_string(),
            public_dir: "./public".to_string(),
            open: false,
            csp: CspConfig::default(),
        }
    }
}

/// CSP directive values. Each field is a list of source expressions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CspConfig {
    /// Set to `false` to disable CSP injection entirely.
    pub enabled: bool,
    #[serde(rename = "defaultSrc")]
    pub default_src: Vec<String>,
    #[serde(rename = "scriptSrc")]
    pub script_src: Vec<String>,
    #[serde(rename = "styleSrc")]
    pub style_src: Vec<String>,
    #[serde(rename = "imgSrc")]
    pub img_src: Vec<String>,
    #[serde(rename = "connectSrc")]
    pub connect_src: Vec<String>,
}

impl Default for CspConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default_src: vec!["'self'".into()],
            script_src: vec!["'self'".into(), "'unsafe-inline'".into()],
            style_src: vec!["'self'".into(), "'unsafe-inline'".into()],
            img_src: vec!["'self'".into(), "data:".into()],
            connect_src: vec!["'self'".into(), "ws:".into(), "wss:".into()],
        }
    }
}

impl CspConfig {
    /// Render the CSP directives as a single `Content-Security-Policy` header value.
    pub fn to_header_value(&self) -> String {
        let join = |v: &Vec<String>| v.join(" ");
        let mut parts: Vec<String> = Vec::new();
        if !self.default_src.is_empty() {
            parts.push(format!("default-src {}", join(&self.default_src)));
        }
        if !self.script_src.is_empty() {
            parts.push(format!("script-src {}", join(&self.script_src)));
        }
        if !self.style_src.is_empty() {
            parts.push(format!("style-src {}", join(&self.style_src)));
        }
        if !self.img_src.is_empty() {
            parts.push(format!("img-src {}", join(&self.img_src)));
        }
        if !self.connect_src.is_empty() {
            parts.push(format!("connect-src {}", join(&self.connect_src)));
        }
        parts.join("; ")
    }
}

// ── test ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TestConfig {
    pub paths: Vec<String>,
    pub watch: bool,
    pub coverage: bool,
    #[serde(rename = "updateSnapshots")]
    pub update_snapshots: bool,
    /// Maximum number of test files to run concurrently (0 = number of CPUs).
    pub concurrency: usize,
}

impl Default for TestConfig {
    fn default() -> Self {
        Self {
            paths: vec!["tests/".into(), "src/".into()],
            watch: false,
            coverage: false,
            update_snapshots: false,
            concurrency: 0,
        }
    }
}

// ── audit ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AuditConfig {
    pub deny: bool,
    pub secrets: bool,
    #[serde(rename = "updateCache")]
    pub update_cache: bool,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            deny: true,
            secrets: false,
            update_cache: false,
        }
    }
}

// ── bundle ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BundleConfig {
    #[serde(rename = "outDir")]
    pub out_dir: String,
    pub minify: bool,
    #[serde(rename = "sourceMap")]
    pub source_map: bool,
    pub split: bool,
}

impl Default for BundleConfig {
    fn default() -> Self {
        Self {
            out_dir: "./dist".to_string(),
            minify: false,
            source_map: true,
            split: false,
        }
    }
}

// ── workspace ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceConfig {
    pub hoisting: bool,
    /// Max concurrent package scripts during `3va workspace run`.
    pub parallelism: usize,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            hoisting: true,
            parallelism: 4,
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dev_port_is_3000() {
        let cfg = DevConfig::default();
        assert_eq!(cfg.port, 3000);
    }

    #[test]
    fn csp_header_contains_default_src() {
        let csp = CspConfig::default();
        let header = csp.to_header_value();
        assert!(header.contains("default-src 'self'"));
        assert!(header.contains("connect-src 'self' ws: wss:"));
    }

    #[test]
    fn csp_disabled_returns_no_directives() {
        let csp = CspConfig {
            enabled: false,
            default_src: vec![],
            script_src: vec![],
            style_src: vec![],
            img_src: vec![],
            connect_src: vec![],
        };
        assert!(csp.to_header_value().is_empty());
    }

    #[test]
    fn project_config_deserializes_from_json() {
        let json = r#"{"dev":{"port":8080},"test":{"coverage":true}}"#;
        let cfg: ProjectConfig = serde_json::from_str(json).unwrap();
        assert_eq!(cfg.dev.port, 8080);
        assert!(cfg.test.coverage);
        // Non-specified fields keep defaults
        assert!(!cfg.dev.open);
    }

    #[test]
    fn workspace_default_parallelism() {
        let ws = WorkspaceConfig::default();
        assert_eq!(ws.parallelism, 4);
        assert!(ws.hoisting);
    }
}
