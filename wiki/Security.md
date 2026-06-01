# Security

## Reporting vulnerabilities

**Do not open a public issue.**

Email `security@sophava.com` with:
- A description of the vulnerability
- Reproduction steps
- Affected versions (if known)

We will acknowledge receipt within 48 hours and aim to provide a fix or mitigation within 90 days depending on severity.

## Security model

3va's security model is built around three principles:

### 1. Deny by default

No access to the filesystem, network, environment variables, or child processes unless explicitly granted via a flag. See [[Permissions]] for details.

### 2. Untrusted dependencies

The package manager never executes post-install scripts. Packages are treated as untrusted code. The `3va audit` command scans for:

- Known malicious patterns (static analysis)
- CVEs via the [OSV database](https://osv.dev)
- Leaked secrets and credentials (opt-in with `--secrets`)

### 3. Post-quantum cryptography

The `vvva_crypto` crate uses NIST-standardized post-quantum primitives:

- **ML-KEM-768** for key encapsulation
- **ML-DSA** for digital signatures

The `__pqTlsConnect` runtime global establishes hybrid classical + post-quantum TLS connections.

## Automated security checks

Every commit and PR is scanned by:

- **Semgrep SAST** — custom rules in `.semgrep/rules/` covering unsafe Rust patterns, filesystem access, and 3va-specific security properties
- **CodeQL** — GitHub's semantic code analysis
- **Gitleaks** — secret scanning
- **cargo-audit** — Rust dependency CVE scanning
- **cargo-deny** — dependency license and ban enforcement

## Responsible disclosure

We follow coordinated disclosure. If you discover a vulnerability, please give us reasonable time to address it before public disclosure.
