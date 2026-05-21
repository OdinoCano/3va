//! OSV-based vulnerability auditor for 3va dependencies.
//!
//! Queries https://api.osv.dev/v1/querybatch in batches and caches results
//! locally in ~/.cache/3va/audit/ with a 24-hour TTL per package@version.
//! On network failure the stale cache is used with a warning so the command
//! never hard-fails due to connectivity.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::lockfile::Lockfile;

const OSV_BATCH_URL: &str = "https://api.osv.dev/v1/querybatch";
const CACHE_TTL_SECS: u64 = 86_400; // 24 h
const BATCH_CHUNK_SIZE: usize = 100; // OSV supports up to 1000; 100 keeps requests snappy

// ── OSV API: request types ────────────────────────────────────────────────────

#[derive(Serialize)]
struct OsvQuery {
    version: String,
    package: OsvPackageRef,
}

#[derive(Serialize)]
struct OsvPackageRef {
    name: String,
    ecosystem: String,
}

#[derive(Serialize)]
struct OsvBatchRequest {
    queries: Vec<OsvQuery>,
}

// ── OSV API: response types ───────────────────────────────────────────────────

#[derive(Deserialize)]
struct OsvBatchResponse {
    #[serde(default)]
    results: Vec<OsvQueryResult>,
}

#[derive(Deserialize)]
struct OsvQueryResult {
    #[serde(default)]
    vulns: Vec<RawVuln>,
}

/// Raw OSV vulnerability record (subset of the full schema we care about).
#[derive(Deserialize, Serialize, Clone)]
struct RawVuln {
    id: String,
    summary: Option<String>,
    #[serde(default)]
    severity: Vec<RawSeverityEntry>,
    #[serde(default)]
    affected: Vec<RawAffected>,
    #[serde(default)]
    references: Vec<RawReference>,
    database_specific: Option<serde_json::Value>,
}

#[derive(Deserialize, Serialize, Clone)]
struct RawSeverityEntry {
    #[serde(rename = "type")]
    severity_type: String,
    score: String,
}

#[derive(Deserialize, Serialize, Clone)]
struct RawAffected {
    #[serde(default)]
    ranges: Vec<RawRange>,
    database_specific: Option<serde_json::Value>,
}

#[derive(Deserialize, Serialize, Clone)]
struct RawRange {
    #[serde(rename = "type")]
    range_type: String,
    #[serde(default)]
    events: Vec<HashMap<String, String>>,
}

#[derive(Deserialize, Serialize, Clone)]
struct RawReference {
    #[serde(rename = "type")]
    ref_type: String,
    url: String,
}

// ── Cache ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize, Serialize)]
struct CacheEntry {
    fetched_at_unix: u64,
    vulns: Vec<RawVuln>,
}

// ── Public output types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum VulnSeverity {
    Unknown = 0,
    Low = 1,
    Medium = 2,
    High = 3,
    Critical = 4,
}

impl VulnSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Critical => "CRITICAL",
            Self::High => "HIGH",
            Self::Medium => "MEDIUM",
            Self::Low => "LOW",
            Self::Unknown => "UNKNOWN",
        }
    }

    pub fn is_high_or_critical(&self) -> bool {
        matches!(self, Self::High | Self::Critical)
    }
}

#[derive(Debug, Clone)]
pub struct Vulnerability {
    pub id: String,
    pub summary: String,
    pub severity: VulnSeverity,
    pub fixed_versions: Vec<String>,
    pub details_url: String,
}

#[derive(Debug, Clone)]
pub struct VulnFinding {
    pub pkg_name: String,
    pub pkg_version: String,
    pub vulns: Vec<Vulnerability>,
}

pub struct AuditReport {
    pub findings: Vec<VulnFinding>,
    pub total_packages: usize,
    pub packages_with_vulns: usize,
    pub total_vulns: usize,
    pub critical_count: usize,
    pub high_count: usize,
    pub cache_hits: usize,
    pub network_errors: Vec<String>,
}

// ── CVSS v3.x base-score calculator ──────────────────────────────────────────
//
// Implements the CVSS v3.1 specification formula exactly so we can convert
// vector strings like "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H" to a
// numeric score without an external crate dependency.

fn compute_cvss3_score(vector: &str) -> Option<f64> {
    let tail = vector
        .strip_prefix("CVSS:3.1/")
        .or_else(|| vector.strip_prefix("CVSS:3.0/"))?;

    let mut m: HashMap<&str, &str> = HashMap::new();
    for part in tail.split('/') {
        let (k, v) = part.split_once(':')?;
        m.insert(k, v);
    }

    let av: f64 = match *m.get("AV")? {
        "N" => 0.85,
        "A" => 0.62,
        "L" => 0.55,
        "P" => 0.20,
        _ => return None,
    };
    let ac: f64 = match *m.get("AC")? {
        "L" => 0.77,
        "H" => 0.44,
        _ => return None,
    };
    let scope = *m.get("S")?;
    let pr: f64 = match (*m.get("PR")?, scope) {
        ("N", _) => 0.85,
        ("L", "U") => 0.62,
        ("L", "C") => 0.68,
        ("H", "U") => 0.27,
        ("H", "C") => 0.50,
        _ => return None,
    };
    let ui: f64 = match *m.get("UI")? {
        "N" => 0.85,
        "R" => 0.62,
        _ => return None,
    };
    let conf: f64 = match *m.get("C")? {
        "H" => 0.56,
        "L" => 0.22,
        "N" => 0.00,
        _ => return None,
    };
    let integ: f64 = match *m.get("I")? {
        "H" => 0.56,
        "L" => 0.22,
        "N" => 0.00,
        _ => return None,
    };
    let avail: f64 = match *m.get("A")? {
        "H" => 0.56,
        "L" => 0.22,
        "N" => 0.00,
        _ => return None,
    };

    let iss = 1.0 - (1.0 - conf) * (1.0 - integ) * (1.0 - avail);

    let impact = if scope == "U" {
        6.42 * iss
    } else {
        7.52 * (iss - 0.029) - 3.25 * (iss - 0.02_f64).powf(15.0)
    };

    if impact <= 0.0 {
        return Some(0.0);
    }

    let exploit = 8.22 * av * ac * pr * ui;

    let raw = if scope == "U" {
        (impact + exploit).min(10.0)
    } else {
        (1.08 * (impact + exploit)).min(10.0)
    };

    // CVSS roundup: ceiling to nearest 0.1
    Some((raw * 10.0).ceil() / 10.0)
}

fn numeric_severity(score: f64) -> VulnSeverity {
    if score >= 9.0 {
        VulnSeverity::Critical
    } else if score >= 7.0 {
        VulnSeverity::High
    } else if score >= 4.0 {
        VulnSeverity::Medium
    } else if score > 0.0 {
        VulnSeverity::Low
    } else {
        VulnSeverity::Unknown
    }
}

fn label_severity(label: &str) -> VulnSeverity {
    match label.to_uppercase().as_str() {
        "CRITICAL" => VulnSeverity::Critical,
        "HIGH" => VulnSeverity::High,
        "MODERATE" | "MEDIUM" => VulnSeverity::Medium,
        "LOW" => VulnSeverity::Low,
        _ => VulnSeverity::Unknown,
    }
}

fn parse_severity(vuln: &RawVuln) -> VulnSeverity {
    // 1. CVSS v3 vector — most precise
    for entry in &vuln.severity {
        if entry.severity_type.starts_with("CVSS_V3")
            && let Some(score) = compute_cvss3_score(&entry.score)
            && score > 0.0
        {
            return numeric_severity(score);
        }
    }

    // 2. CVSS v2 numeric score (some older records)
    for entry in &vuln.severity {
        if entry.severity_type.starts_with("CVSS_V2")
            && let Ok(score) = entry.score.parse::<f64>()
        {
            // v2 thresholds: High ≥ 7, Medium ≥ 4, Low > 0
            return if score >= 7.0 {
                VulnSeverity::High
            } else if score >= 4.0 {
                VulnSeverity::Medium
            } else {
                VulnSeverity::Low
            };
        }
    }

    // 3. database_specific.severity label (GitHub Advisory / GHSA format)
    if let Some(label) = vuln
        .database_specific
        .as_ref()
        .and_then(|d| d.get("severity"))
        .and_then(|s| s.as_str())
    {
        return label_severity(label);
    }

    // 4. affected[].database_specific.severity
    for affected in &vuln.affected {
        if let Some(label) = affected
            .database_specific
            .as_ref()
            .and_then(|d| d.get("severity"))
            .and_then(|s| s.as_str())
        {
            return label_severity(label);
        }
    }

    VulnSeverity::Unknown
}

fn extract_fixed_versions(vuln: &RawVuln) -> Vec<String> {
    let mut fixed = Vec::new();
    for affected in &vuln.affected {
        for range in &affected.ranges {
            for event in &range.events {
                if let Some(ver) = event.get("fixed")
                    && !ver.is_empty()
                    && !fixed.contains(ver)
                {
                    fixed.push(ver.clone());
                }
            }
        }
    }
    fixed
}

fn best_url(vuln: &RawVuln) -> String {
    // Prefer ADVISORY or WEB links; fall back to the OSV canonical URL
    for preferred in ["ADVISORY", "WEB"] {
        if let Some(r) = vuln.references.iter().find(|r| r.ref_type == preferred) {
            return r.url.clone();
        }
    }
    format!("https://osv.dev/vulnerability/{}", vuln.id)
}

fn convert(raw: &RawVuln) -> Vulnerability {
    Vulnerability {
        id: raw.id.clone(),
        summary: raw
            .summary
            .clone()
            .unwrap_or_else(|| "No summary available".to_string()),
        severity: parse_severity(raw),
        fixed_versions: extract_fixed_versions(raw),
        details_url: best_url(raw),
    }
}

// ── Cache helpers ─────────────────────────────────────────────────────────────

fn audit_cache_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".cache")
        .join("3va")
        .join("audit")
}

fn cache_filename(name: &str, version: &str) -> String {
    // @scope/pkg → scope__pkg  (avoid directory separators in filenames)
    let safe = name.trim_start_matches('@').replace('/', "__");
    format!("{}@{}.json", safe, version)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn read_cache(name: &str, version: &str, force: bool) -> Option<Vec<RawVuln>> {
    if force {
        return None;
    }
    let path = audit_cache_dir().join(cache_filename(name, version));
    let content = std::fs::read_to_string(&path).ok()?;
    let entry: CacheEntry = serde_json::from_str(&content).ok()?;
    let age = now_unix().saturating_sub(entry.fetched_at_unix);
    if age <= CACHE_TTL_SECS {
        Some(entry.vulns)
    } else {
        None
    }
}

fn read_stale_cache(name: &str, version: &str) -> Vec<RawVuln> {
    let path = audit_cache_dir().join(cache_filename(name, version));
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<CacheEntry>(&s).ok())
        .map(|e| e.vulns)
        .unwrap_or_default()
}

fn write_cache(name: &str, version: &str, vulns: &[RawVuln]) {
    let dir = audit_cache_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let entry = CacheEntry {
        fetched_at_unix: now_unix(),
        vulns: vulns.to_vec(),
    };
    if let Ok(json) = serde_json::to_string(&entry) {
        let _ = std::fs::write(dir.join(cache_filename(name, version)), json);
    }
}

// ── OSV HTTP client ───────────────────────────────────────────────────────────

async fn query_osv_batch(
    client: &reqwest::Client,
    chunk: &[(String, String)],
) -> anyhow::Result<Vec<Vec<RawVuln>>> {
    let queries: Vec<OsvQuery> = chunk
        .iter()
        .map(|(name, version)| OsvQuery {
            version: version.clone(),
            package: OsvPackageRef {
                name: name.clone(),
                ecosystem: "npm".to_string(),
            },
        })
        .collect();

    let body = OsvBatchRequest { queries };

    let send = |b: &OsvBatchRequest| {
        client
            .post(OSV_BATCH_URL)
            .json(b)
            .timeout(std::time::Duration::from_secs(30))
            .send()
    };

    let resp = send(&body).await?;

    let resp = if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // Single retry after a short back-off
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        send(&body).await?
    } else {
        resp
    };

    if !resp.status().is_success() {
        anyhow::bail!("OSV API returned HTTP {}", resp.status());
    }

    let parsed: OsvBatchResponse = resp.json().await?;
    Ok(parsed.results.into_iter().map(|r| r.vulns).collect())
}

// ── Node modules fallback ─────────────────────────────────────────────────────

fn version_from_pkg_json(path: &Path) -> String {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["version"].as_str().map(String::from))
        .unwrap_or_else(|| "unknown".to_string())
}

fn packages_from_node_modules(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if name.starts_with('@') {
            let Ok(subs) = std::fs::read_dir(&p) else {
                continue;
            };
            for sub in subs.flatten() {
                let sp = sub.path();
                if sp.is_dir() {
                    let full = format!("{}/{}", name, sub.file_name().to_string_lossy());
                    let ver = version_from_pkg_json(&sp.join("package.json"));
                    out.push((full, ver));
                }
            }
        } else {
            let ver = version_from_pkg_json(&p.join("package.json"));
            out.push((name, ver));
        }
    }
    out
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Runs the full OSV vulnerability audit against installed packages.
///
/// Reads `3va-lock.json` (or falls back to scanning `node_modules/`), queries
/// the OSV batch API for every package@version, caches results locally, and
/// returns a structured report.
///
/// - `force_refresh`: bypass the 24-hour cache TTL and fetch fresh data.
pub async fn run_audit(force_refresh: bool) -> anyhow::Result<AuditReport> {
    let lockfile_path = PathBuf::from("3va-lock.json");
    let node_modules = PathBuf::from("node_modules");

    let packages: Vec<(String, String)> = if lockfile_path.exists() {
        let lock = Lockfile::load(&lockfile_path)?;
        lock.dependencies
            .iter()
            .filter(|(_, dep)| dep.version != "unknown")
            .map(|(name, dep)| (name.clone(), dep.version.clone()))
            .collect()
    } else if node_modules.exists() {
        packages_from_node_modules(&node_modules)
            .into_iter()
            .filter(|(_, v)| v != "unknown")
            .collect()
    } else {
        anyhow::bail!(
            "No 3va-lock.json or node_modules/ found. Run '3va install <pkg> --allow-net=...' first."
        );
    };

    if packages.is_empty() {
        return Ok(AuditReport {
            findings: vec![],
            total_packages: 0,
            packages_with_vulns: 0,
            total_vulns: 0,
            critical_count: 0,
            high_count: 0,
            cache_hits: 0,
            network_errors: vec![],
        });
    }

    let total_packages = packages.len();
    let mut cache_hits = 0usize;
    let mut network_errors: Vec<String> = Vec::new();
    let mut raw_results: HashMap<(String, String), Vec<RawVuln>> = HashMap::new();
    let mut to_fetch: Vec<(String, String)> = Vec::new();

    for (name, version) in &packages {
        if let Some(cached) = read_cache(name, version, force_refresh) {
            cache_hits += 1;
            raw_results.insert((name.clone(), version.clone()), cached);
        } else {
            to_fetch.push((name.clone(), version.clone()));
        }
    }

    if !to_fetch.is_empty() {
        let client = reqwest::Client::builder()
            .user_agent("3va-audit/0.1 (+https://github.com/sophava/3va)")
            .build()?;

        for chunk in to_fetch.chunks(BATCH_CHUNK_SIZE) {
            match query_osv_batch(&client, chunk).await {
                Ok(results) => {
                    for (i, vulns) in results.into_iter().enumerate() {
                        let (name, version) = &chunk[i];
                        write_cache(name, version, &vulns);
                        raw_results.insert((name.clone(), version.clone()), vulns);
                    }
                }
                Err(e) => {
                    let msg = format!("{}", e);
                    tracing::warn!("OSV API unavailable: {}. Falling back to stale cache.", msg);
                    network_errors.push(msg);
                    for (name, version) in chunk {
                        let stale = read_stale_cache(name, version);
                        raw_results.insert((name.clone(), version.clone()), stale);
                    }
                }
            }
        }
    }

    let mut findings = Vec::new();
    let mut packages_with_vulns = 0usize;
    let mut total_vulns = 0usize;
    let mut critical_count = 0usize;
    let mut high_count = 0usize;

    for (name, version) in &packages {
        let key = (name.clone(), version.clone());
        let Some(raw) = raw_results.get(&key) else {
            continue;
        };
        if raw.is_empty() {
            continue;
        }

        let vulns: Vec<Vulnerability> = raw.iter().map(convert).collect();

        for v in &vulns {
            match v.severity {
                VulnSeverity::Critical => critical_count += 1,
                VulnSeverity::High => high_count += 1,
                _ => {}
            }
        }

        total_vulns += vulns.len();
        packages_with_vulns += 1;
        findings.push(VulnFinding {
            pkg_name: name.clone(),
            pkg_version: version.clone(),
            vulns,
        });
    }

    Ok(AuditReport {
        findings,
        total_packages,
        packages_with_vulns,
        total_vulns,
        critical_count,
        high_count,
        cache_hits,
        network_errors,
    })
}

/// Prints the OSV audit report and returns `true` if the audit passed.
///
/// When `deny` is `true` the function returns `false` (and prints an error) if
/// any CRITICAL or HIGH vulnerability was found — useful as a CI gate.
pub fn print_audit_report(report: &AuditReport, deny: bool) -> bool {
    println!();
    println!(
        "Scanning {} package(s) for known vulnerabilities (OSV)...",
        report.total_packages
    );

    if report.total_packages > 0 {
        let fresh = report.total_packages.saturating_sub(report.cache_hits);
        if report.cache_hits > 0 && fresh > 0 {
            println!(
                "  {} fresh from OSV, {} from local cache (TTL 24h)",
                fresh, report.cache_hits
            );
        } else if report.cache_hits == report.total_packages {
            println!(
                "  All {} result(s) served from local cache (TTL 24h).",
                report.cache_hits
            );
            println!("  Run '3va audit --update-cache' to force a fresh fetch.");
        }
    }

    for err in &report.network_errors {
        eprintln!("  ! Network warning: {}", err);
    }
    if !report.network_errors.is_empty() {
        eprintln!("  ! Results may be incomplete. Check your internet connection.");
    }

    println!();

    if report.findings.is_empty() {
        println!(
            "✓ No known vulnerabilities found in {} package(s).",
            report.total_packages
        );
        println!();
        return true;
    }

    // Sort findings: worst severity first
    let mut findings = report.findings.clone();
    findings.sort_by(|a, b| {
        let worst = |f: &VulnFinding| {
            f.vulns
                .iter()
                .map(|v| &v.severity)
                .max()
                .cloned()
                .unwrap_or(VulnSeverity::Unknown)
        };
        worst(b).cmp(&worst(a))
    });

    for finding in &findings {
        let worst = finding
            .vulns
            .iter()
            .map(|v| &v.severity)
            .max()
            .cloned()
            .unwrap_or(VulnSeverity::Unknown);

        eprintln!(
            "  {} {}@{} — {} issue(s)",
            worst.as_str(),
            finding.pkg_name,
            finding.pkg_version,
            finding.vulns.len()
        );

        let mut sorted = finding.vulns.clone();
        sorted.sort_by(|a, b| b.severity.cmp(&a.severity));

        for v in &sorted {
            eprintln!("    [{}] {} — {}", v.severity.as_str(), v.id, v.summary);
            if !v.fixed_versions.is_empty() {
                eprintln!(
                    "           Fix: upgrade to {}",
                    v.fixed_versions.join(" or ")
                );
            }
            eprintln!("           See: {}", v.details_url);
        }
        eprintln!();
    }

    println!("  Packages scanned      : {}", report.total_packages);
    println!("  Packages with vulns   : {}", report.packages_with_vulns);
    println!("  Total vulnerabilities : {}", report.total_vulns);
    if report.critical_count > 0 {
        eprintln!("  Critical              : {}", report.critical_count);
    }
    if report.high_count > 0 {
        eprintln!("  High                  : {}", report.high_count);
    }
    println!();

    let has_severe = report.critical_count > 0 || report.high_count > 0;

    if has_severe && deny {
        eprintln!("✗ Audit failed: CRITICAL or HIGH vulnerabilities detected (--deny).");
        false
    } else if has_severe {
        eprintln!(
            "! {} CRITICAL/HIGH issue(s) found. Review the findings above.",
            report.critical_count + report.high_count
        );
        eprintln!("  Use '3va audit --deny' in CI/CD pipelines to enforce a hard block.");
        true
    } else {
        println!(
            "! {} vulnerability(s) found (none CRITICAL or HIGH).",
            report.total_vulns
        );
        true
    }
}
