# 01 - STATIC SECURITY ANALYSIS

## 1.1 Overview

3va performs static security analysis on package code during installation and via the `audit` command. Analysis runs entirely in Rust — no external tools required.

Two analyzers are implemented:

| Analyzer | Crate | Command |
|----------|-------|---------|
| Malware scanner | `vvva_pm::MalwareScanner` | `3va audit` (Phase 1) |
| Secrets scanner | `vvva_pm::SecretsScanner` | `3va audit --secrets` (Phase 3) |

## 1.2 Malware Scanner

Scans installed code in `node_modules/` for known malicious patterns using both raw text matching and AST traversal (via Oxc).

### Detected patterns

| Pattern | Description | Severity |
|---------|-------------|----------|
| `rm -rf /` | Destructive recursive delete | Critical |
| `rm -rf $` | Recursive delete with variable | High |
| `> /etc/passwd` | Overwriting system files | Critical |
| `chmod 777` | World-writable permissions | High |
| `curl \| sh` | Pipe to shell execution | Critical |
| `wget \| sh` | Wget pipe to shell | Critical |
| `:(){ :\|:& };:` | Fork bomb | Critical |
| `exfiltrat` | Data exfiltration attempt | High |
| `eval(atob` | Base64-decode eval | High |
| `fromCharCode` | Character-code obfuscation | Medium |
| `cryptonight` / `coinhive` | Cryptocurrency mining | High |
| `backdoor` | Backdoor reference | Critical |
| `remote code execution` | RCE reference | High |

### Usage

The malware scanner runs **automatically during `3va install`** on every newly downloaded tarball, together with the secrets scanner (§1.3): packages are scanned after integrity verification and before they are written to the store or `node_modules/`. CRITICAL/HIGH findings abort the install of that package with a report in the audit output format. Skip it with `--no-scan` (e.g. CI flows that run `3va audit` separately).

```bash
3va install axios --allow-net=registry.npmjs.org  # scanned automatically; CRITICAL/HIGH aborts the install
3va install axios --no-scan                       # opt out; run audit yourself

# Explicit audit still available
3va audit
3va audit --json
```

Verified by the `security_scan_*` unit tests in `crates/pm/src/lib.rs` (`security_scan_passes_a_clean_package`, `security_scan_aborts_package_with_embedded_aws_key`, `security_scan_respects_skip_flag`).

## 1.3 Secrets Scanner

Scans source files for hardcoded secrets using 16 regex patterns.

### Detected secret types

| Pattern | Example match |
|---------|--------------|
| AWS access key | `AKIA[0-9A-Z]{16}` |
| GitHub PAT (classic) | `ghp_[A-Za-z0-9]{36}` |
| GitHub App token | `ghs_`, `gho_` |
| GitLab token | `glpat-` |
| Stripe secret key | `sk_live_` |
| Slack token | `xox[baprs]-` |
| SendGrid key | `SG\.` |
| Twilio token | `SK[0-9a-f]{32}` |
| PEM private key | `-----BEGIN ... PRIVATE KEY-----` |
| JWT | `eyJ[A-Za-z0-9_-]+\.eyJ` |
| npm token | `npm_[A-Za-z0-9]{36}` |
| Generic password | `password\s*=\s*"[^"]{8,}"` |
| Generic API key | `api[_-]?key\s*=\s*"[^"]{16,}"` |
| DB connection string | `(postgres\|mysql\|mongodb):\/\/` |

### Severity levels

| Severity | Examples |
|----------|---------|
| Critical | PEM private key, AWS key |
| High | GitHub token, Stripe key, JWT |
| Medium | Generic password, API key |
| Low | DB connection string (may be dev) |

### Usage

```bash
# Secrets scan is Phase 3 of audit (requires --secrets)
3va audit --secrets
3va audit --secrets --json
3va audit --secrets --deny   # exit 9 on findings
```

## 1.4 Scope

**What static analysis covers:**
- Installed packages in `node_modules/`
- Code extracted from tarballs during `install`

**What it does NOT cover:**
- General-purpose SAST for user code (XSS, SQLi, path traversal detection in application code)
- Runtime sandboxing (handled by the permission system — see `06-permissions/`)
- Binary analysis

Advanced SAST (XSS, SQLi, RCE detection in user application code) is planned for a future version.

---

*Malware scanner in `crates/pm/src/malware_scanner.rs`. Secrets scanner in `crates/pm/src/secrets.rs`.*
