# Security Policy

## Supported Versions

Only the latest minor release receives security fixes. Upgrading to the latest
release is the supported remediation path.

| Version | Status      | Notes |
|---------|-------------|-------|
| 2.8.x   | Current     | Receives security patches |
| < 2.8   | Unsupported | Upgrade to 2.8.x |

## Reporting a Vulnerability

**Do not open a public GitHub issue for security bugs.**

The canonical reporting channel is [GitHub Security Advisories](https://github.com/OdinoCano/3va/security/advisories/new).
This is the mechanism referenced in the README ([source](README.md#reporting-security-vulnerabilities)).

When reporting, include:
- Description of the vulnerability
- Reproduction steps
- Affected versions (if known)
- Any mitigations already applied

## Response Times (SLA)

| Stage | Commitment |
|-------|------------|
| Acknowledge a report | 72 hours |
| Initial severity assessment (CVSS v4.0) | 5 days |
| Patched release for Critical/High severity, or a vulnerable dependency advisory | **8 days** |
| Patched release for Medium/Low severity | Next scheduled release, at most 90 days |

Clocks start when the report is received or when the dependency advisory is
published (RustSec/GHSA/OSV), whichever applies. The 8-day patch window is
derived from the project's measured history: 2.5 × the mean time-to-patch of
all dependency advisories fixed so far (2.93 days, see
[Patched Dependency Advisories](#patched-dependency-advisories)), rounded up.
It is the largest of 2.5 × mean, 2.5 × median and 2.5 × mode.

## Scope

### What this covers

- **3va runtime (`vvva_js`, `vvva_core`)**: V8-based JS execution with deny-by-default
  capability enforcement (`vvva_permissions`). All capabilities (filesystem, network,
  environment variables, child processes, native addons) are blocked by default and must
  be explicitly declared at invocation.
- **Package manager (`vvva_pm`)**: Post-install scripts (`preinstall`, `install`,
  `postinstall`, `prepare`, `prepublish`) are **never executed** — this is a
  enforced invariant, not a flag.
- **Permission system (`vvva_permissions`)**: Grants are scoped per-dependency, not
  per-process. A package not listed in a `package.json` `"3va"` grant gets no
  capability, even if other packages in the same process receive grants.
- **`3va audit`**: Three-phase audit:
  1. Malware scan (static analysis of `node_modules` for known malicious patterns)
  2. OSV CVE scan (`api.osv.dev`, 24-hour local cache)
  3. Secrets detection (opt-in, 21 patterns covering AWS keys, GitHub tokens,
     Stripe keys, private certificates, JWT secrets, database connection strings)

### What this does NOT cover

- **Vulnerabilities in third-party packages installed via `3va install`**: 3va
  audit can *detect* known CVEs via OSV and can *report* them — it is not a
  patching mechanism. Fixing an vulnerable dependency is the user's responsibility.
- **Malware in packages installed by other package managers** (npm, bun, pnpm, yarn)
  before they are imported into a 3va project.
- **OSV CVE data quality or completeness**: 3va queries `api.osv.dev` as-is; it does
  not maintain its own vulnerability database.
- **`3va run <script>` delegates to external package managers for `package.json`
  scripts** (see README §"package.json scripts fallback"): this delegation is **not
  sandboxed** — the delegated script runs as a real external process outside
  `vvva_permissions`' capability model. Running arbitrary package scripts requires
  explicit consent (`--yes`, TTY prompt, or `"3va": { "no-prompt": true }`).
- **`native-tls` TLS connections**: The default TLS path (`reqwest` + `native-tls`)
  does **not** include post-quantum key exchange. Only `__pqTlsConnect` (hybrid
  classical + ML-KEM-768) provides post-quantum forward secrecy. Applications
  requiring PQ security must use `__pqTlsConnect` explicitly (see README §
  "Post-Quantum Cryptography").

## Existing Security Guarantees

3va's security properties are documented in the README. This section summarizes them
without reformulating — refer to the README for authoritative details:

| Guarantee | Location in README |
|-----------|-------------------|
| Deny-by-default permissions, interactive prompts, CLI flags and `package.json` grants | [§Permissions](README.md#permissions) |
| Post-install scripts never executed | [§Package Manager](README.md#package-manager) ("Post-install scripts are never executed. There are no exceptions.") |
| Permission grants scoped per-dependency | [§Comparison table](README.md#comparison) row "Permission grants scoped per-dependency" |
| Malware + OSV + secrets audit via `3va audit` | [§Audit](README.md#audit) |
| Post-quantum TLS via `__pqTlsConnect` (ML-KEM-768 hybrid) | [§Post-Quantum Cryptography](README.md#post-quantum-cryptography) |
| OSV CVE scan with 24h cache | [§Audit](README.md#audit) |

The architecture table in the README lists each crate's responsibility, including the
capability engine (`vvva_permissions`), the package manager with audit
(`vvva_pm`), and the cryptography crate (`vvva_crypto`).

## Responsible Disclosure

3va follows coordinated vulnerability disclosure (ISO/IEC 29147 and ISO/IEC 30111):

1. The reporter submits privately via GitHub Security Advisories.
2. Maintainers acknowledge, triage and develop a fix in a private fork.
3. A CVE is requested through GitHub (CNA) for confirmed vulnerabilities.
4. The fix is released, and the advisory is published at the same time, crediting the reporter unless they ask not to be named.
5. Reporters are asked to keep the issue private for 90 days, or until the fix ships if that happens first.

## EU Cyber Resilience Act

For vulnerabilities in 3va that are actively exploited, the maintainer notifies
ENISA and the relevant CSIRT through the CRA single reporting platform. The
notification follows CRA Article 14: an early warning within 24 hours, a
notification within 72 hours and a final report within 14 days of a fix being
available. Downstream integrators should monitor the repository's security
advisories and the release SBOMs.

## Verifying Releases

Every release asset ships with:

- `*.sha256`: a SHA-256 checksum
- `*.bundle`: a keyless cosign (Sigstore) signature
- `*.intoto.jsonl`: SLSA build provenance
- `3va-<tag>.cdx.json`: a CycloneDX 1.5 SBOM, also signed with cosign

```sh
cosign verify-blob 3va-v2.8.0-x86_64-unknown-linux-gnu.tar.gz \
  --bundle 3va-v2.8.0-x86_64-unknown-linux-gnu.tar.gz.bundle \
  --certificate-identity-regexp 'https://github.com/OdinoCano/3va/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

### FIPS 140-3

Linux release assets ending in `-fips` are built with `--features fips`. They
route all runtime cryptography and TLS through the AWS-LC FIPS module (AWS-LC
FIPS 4.0, currently in the CMVP Modules In Process list) and reject
non-approved algorithms with `ERR_CRYPTO_FIPS_FORCED`. Scope, limits and
validation status are documented in
[docs/10-security/10-fips.md](docs/10-security/10-fips.md).

Secure-development practices are mapped to NIST SP 800-218 (SSDF) in
[docs/10-security/09-nist-ssdf.md](docs/10-security/09-nist-ssdf.md).

## Advisory History

No security advisories have been published for 3va as of this date.

Past accepted risks documented in `docs/SECURITY.md` (internal security documentation):

| Advisory | Crate | Status | Notes |
|----------|-------|--------|-------|
| RUSTSEC-2023-0071 (Marvin Attack, CVE-2023-49092) | `rsa` | Accepted | `rsa` 0.9 used only for signing/verification; `RsaPrivateKey::decrypt()` is unreachable in 3va's code paths. `deny.toml` ignore with documented rationale. |
| RUSTSEC-2023-0051, RUSTSEC-2024-0370 | `wasmtime` transitive | Accepted | No active exploitation path for 3va's WASM usage. |
| RUSTSEC-2025-0057 | `fxhash` (transitive) | Accepted | No active CVE; dropped if `wasmtime` drops it. |

### Patched Dependency Advisories

Time-to-patch runs from advisory publication (or from when the dependency was
adopted, if the advisory already existed) to the fix commit.

| Advisory | Crate | Fixed in | Days |
|----------|-------|----------|------|
| RUSTSEC-2025-0057, -2025-0118, -2026-0006, -2026-0020, -2026-0085 | `wasmtime` / `fxhash` | eef255b | 0.29 each |
| RUSTSEC-2026-0222 | `wasmtime` | fa910d0 | 18.10 |
| RUSTSEC-2026-0258 | `h2` | fa910d0 | 0.44 |
| RUSTSEC-2026-0285 (CVE-2025-61730) | `rustls` | 84a6394 | 3.41 |

Mean 2.93 days · median 0.29 · mode 0.29.

> Note: `docs/SECURITY.md` is internal developer documentation (Rust-specific security
> hardening, fuzzing, accepted risk register). This file (`SECURITY.md` at repo root)
> is the public-facing security policy.

---

## Policy Confirmation

Confirmed by Edgar Cano (2026-09-23):
- Version support: latest minor only
- Reporting channel: GitHub Security Advisories
- SLA: 8-day patch window for Critical/High (derived from measured history, see above)
- Disclosure: coordinated, 90-day embargo maximum
