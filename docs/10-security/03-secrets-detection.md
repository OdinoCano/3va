# 03 - Secrets Detection

The secrets detection module scans source files for hardcoded credentials: API keys, access tokens, passwords, private certificates, and database connection strings. It is implemented in `crates/pm/src/secrets.rs` and is activated as an optional phase of the `3va audit` command.

---

## Usage

Secrets detection is activated with the `--secrets` flag on the `audit` command:

```bash
# Scan with human-readable output
3va audit --secrets

# JSON output (includes the secrets phase)
3va audit --json --secrets

# Combined with --deny to fail on severe OSV vulnerabilities
3va audit --deny --secrets
```

The scanner recursively analyzes the current working directory. Findings are printed to `stderr`; the process exits with a non-zero exit code **only if Critical severity secrets are found**. Lower severity findings (High, Medium, Low) produce a warning but do not interrupt the pipeline.

### Human-readable output

Each finding is reported in the format:

```
  [CRITICAL] src/config.js:12 — aws_access_key — const key = "AKIA...[REDACTED]...xyz"
        Fix: Store in AWS_ACCESS_KEY_ID env var or use IAM roles
```

A summary is shown at the end of the scan:

```
  Secrets found: 3 (1 critical, 2 high)
✗ Critical secrets detected. Remove them immediately.
```

If no critical secrets are found but lower severity ones are:

```
  Secrets found: 2 (0 critical, 2 high)
! Secrets detected. Review and rotate affected credentials.
```

---

## Detected Patterns

The table shows the 20 registered patterns, in the order they are evaluated. When multiple patterns match on the same line, **only one finding is generated** (the highest priority pattern, i.e. the first one in the list that matches).

| Pattern Name | Severity | Description |
|---|---|---|
| `aws_access_key` | Critical | AWS access keys (`AKIA[0-9A-Z]{16}`) |
| `aws_secret_key` | Critical | AWS secret keys in assignments (`aws*secret*key = "..."`, 40 chars) |
| `gcp_service_account` | Critical | GCP service account JSON (`"type": "service_account"`) |
| `github_token` | Critical | GitHub user tokens (`ghp_[A-Za-z0-9]{36}`) |
| `github_oauth` | Critical | GitHub OAuth tokens (`gho_[A-Za-z0-9]{36}`) |
| `github_app_token` | Critical | GitHub App tokens (`ghs_[A-Za-z0-9]{36}`) |
| `gitlab_token` | Critical | GitLab personal access tokens (`glpat-[A-Za-z0-9-_]{20}`) |
| `stripe_secret_key` | Critical | Stripe production secret keys (`sk_live_[A-Za-z0-9]{24,}`) |
| `stripe_restricted_key` | High | Stripe restricted keys (`rk_live_[A-Za-z0-9]{24,}`) |
| `slack_token` | High | Slack tokens (`xox[baprs]-[A-Za-z0-9-]{10,}`) |
| `sendgrid_api_key` | High | SendGrid API keys (`SG.<22+ chars>.<43+ chars>`) |
| `twilio_account_sid` | High | Twilio account SIDs (`AC[0-9a-fA-F]{32}`) |
| `private_key_pem` | Critical | PEM private keys (RSA, EC, DSA, OpenSSH) |
| `private_key_pkcs8` | Critical | Encrypted PKCS8 private keys |
| `jwt` | High | Hardcoded JSON Web Tokens (3 base64url segments starting with `eyJ`) |
| `npm_token` | Critical | NPM publish tokens (`npm_[A-Za-z0-9]{36}`) |
| `password_assignment` | High | Passwords in code assignments (`password = '...'`, 8+ chars) |
| `api_key_assignment` | High | Generic API keys (`api_key = '...'`, 20+ alphanumeric chars) |
| `secret_assignment` | Medium | `secret` or `token` variables with literal values (16+ chars) |
| `db_connection_string` | High | Connection URIs with credentials (mongodb, postgres, mysql, redis, amqp) |
| `sensitive_env_var` | Medium | Sensitive environment variable names assigned literally in code (`AWS_SECRET_ACCESS_KEY = '...'`, etc.) |

### Severities

| Level | Description | CI Behavior |
|---|---|---|
| **Critical** | Credential with direct access to production systems or infrastructure | Fails the process (exit ≠ 0) |
| **High** | Third-party service credential or token with elevated permissions | Warning; process continues |
| **Medium** | Suspicious generic assignment that may contain a real secret | Warning; process continues |
| **Low** | Weak indication, uncertain context | Warning; process continues |

---

## Scanned and Excluded Files

### Analyzed extensions

The scanner only reads files with the following extensions:

`.js` `.ts` `.mjs` `.cjs` `.jsx` `.tsx` `.json` `.env` `.yaml` `.yml` `.toml` `.sh` `.bash` `.zsh` `.py` `.rb` `.go` `.rs`

Any other file type (including binaries) is silently skipped.

### Excluded directories

Recursive scanning automatically skips the following directories:

- `.git/`
- `node_modules/`
- `dist/`
- `target/`
- `.cache/`

### Excluded comment lines

Lines starting with `//`, `#`, `*` or `/*` (after trimming leading whitespace) are not evaluated. This avoids false positives in documentation and code examples within comments.

---

## Finding Structure (`SecretFinding`)

```rust
pub struct SecretFinding {
    pub file: PathBuf,        // Path to the file where the secret was found
    pub line: usize,          // Line number (1-based)
    pub secret_type: String,  // Pattern name (e.g. "aws_access_key")
    pub severity: Severity,   // Critical | High | Medium | Low
    pub snippet: String,      // Redacted snippet of the line
    pub suggestion: String,   // Remediation recommendation
}
```

The `snippet` field never exposes the full secret value: the scanner redacts most of the line content before including it in the finding.

---

## JSON Output (`3va audit --json --secrets`)

When using `--json`, the output object includes the `secrets` phase inside `phases`:

```json
{
  "passed": false,
  "phases": {
    "malware": {
      "clean": true
    },
    "osv": {
      "total_packages": 42,
      "packages_with_vulns": 1,
      "total_vulns": 2,
      "critical": 0,
      "high": 1,
      "findings": []
    },
    "secrets": {
      "scanned": true,
      "findings": [
        {
          "file": "src/config.js",
          "line": 12,
          "type": "aws_access_key",
          "severity": "Critical",
          "suggestion": "Store in AWS_ACCESS_KEY_ID env var or use IAM roles"
        },
        {
          "file": "src/db.js",
          "line": 3,
          "type": "db_connection_string",
          "severity": "High",
          "suggestion": "Use process.env.DATABASE_URL instead; never hardcode credentials in URIs"
        }
      ]
    }
  }
}
```

If `--secrets` is not specified, `phases.secrets` is always `{ "scanned": false, "findings": [] }`.

The `passed` field is `false` if any finding has `"Critical"` severity, regardless of the result of the other phases.

---

## One finding per line rule

When multiple patterns match on the same line, **only one finding is emitted**. The scanner evaluates patterns in the order of the table above and uses the first one that matches (priority by position in the list, not by severity). This avoids duplicate reports on lines with multiple signals.

---

## Remediation

The canonical fix is to move the value to an environment variable and access it at runtime:

```javascript
// Incorrect — exposes the credential in the repository
const stripe = new Stripe("YOUR_STRIPE_SECRET_KEY");

// Correct — the value only exists in the runtime environment
const stripe = new Stripe(process.env.STRIPE_SECRET_KEY);
```

For infrastructure secrets (PEM keys, database credentials), consider using a secrets manager (AWS Secrets Manager, HashiCorp Vault, GCP Secret Manager) instead of plain environment variables.

If a credential has already been exposed in Git history, **rotate the credential immediately** — rewriting history is not sufficient if the repository has been cloned or accessed by third parties.
