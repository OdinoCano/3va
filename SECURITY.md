# Security Policy

## Supported Versions

3va does not have a formally documented version support policy outside of the roadmap.
The current stable version is **2.5** (all workspace crates: `vvva_permissions`, `vvva_crypto`,
`vvva_pm`, `vvva_js`, etc., all version 2.5.0).

| Version | Status      | Notes |
|---------|-------------|-------|
| 2.5.x   | Current     | Stable release; receives security patches |
| 2.1.x   | Historical  | Released 2026-06; no longer patched |
| < 2.1   | Unsupported | |

## Reporting a Vulnerability

**Do not open a public GitHub issue for security bugs.**

The canonical reporting channel is [GitHub Security Advisories](https://github.com/OdinoCano/3va/security/advisories/new).
This is the mechanism referenced in the README ([source](README.md#reporting-security-vulnerabilities)).

When reporting, include:
- Description of the vulnerability
- Reproduction steps
- Affected versions (if known)
- Any mitigations already applied

Response times are a future goal, not a committed SLA at this time.

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

3va follows coordinated disclosure. If you discover a vulnerability, please give
maintainers reasonable time to address it before public disclosure. No fixed
timeline has been committed to at this time.

## Advisory History

No security advisories have been published for 3va as of this date.

Past accepted risks documented in `docs/SECURITY.md` (internal security documentation):

| Advisory | Crate | Status | Notes |
|----------|-------|--------|-------|
| RUSTSEC-2023-0071 (Marvin Attack, CVE-2023-49092) | `rsa` | Accepted | `rsa` 0.9 used only for signing/verification; `RsaPrivateKey::decrypt()` is unreachable in 3va's code paths. `deny.toml` ignore with documented rationale. |
| RUSTSEC-2023-0051, RUSTSEC-2024-0370 | `wasmtime` transitive | Accepted | No active exploitation path for 3va's WASM usage. |
| RUSTSEC-2025-0057 | `fxhash` (transitive) | Accepted | No active CVE; dropped if `wasmtime` drops it. |

> Note: `docs/SECURITY.md` is internal developer documentation (Rust-specific security
> hardening, fuzzing, accepted risk register). This file (`SECURITY.md` at repo root)
> is the public-facing security policy.

---

## Verification Pending Maintainer Input

All fields have been confirmed by Edgar Cano (2026-08-19):
- Version support: latest only
- Reporting channel: GitHub Security Advisories
- SLA: future goal, not committed
- Disclosure timeline: "reasonable time", no fixed deadline
