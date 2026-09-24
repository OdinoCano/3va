# 11 - Threat Model and Assurance Case

This document covers five things:

- the threat model and attack surface of the 3va runtime and package manager;
- where its trust boundaries lie;
- how secure-design principles are applied;
- how common implementation weaknesses are countered;
- the known limits of all of the above.

It is the project's security assessment, and it is reviewed at every minor release.

Last reviewed: 2026-09-24 (v2.9.x) by Edgar Cano.

## 11.1 Security requirements

What users can rely on is stated in [SECURITY.md § Scope](../../SECURITY.md#scope). In short:

1. **R1. Deny by default.** A script has no filesystem, network, environment, child-process, or native-addon access unless the user grants it (CLI flags, `package.json` `"3va"` grants, or an interactive prompt).
2. **R2. Per-dependency scoping.** A grant to one package doesn't extend to other packages running in the same process.
3. **R3. No install-time code execution.** `3va install` never runs `preinstall`, `install`, `postinstall`, `prepare`, or `prepublish` scripts.
4. **R4. Encrypted network by default.** TLS 1.2 is the minimum, with certificate verification always on. Plaintext protocols to non-loopback hosts need `--allow-insecure`.
5. **R5. Supply-chain integrity.** Release artifacts are reproducibly identifiable (SHA-256), signed with Sigstore keyless signatures, carry SLSA provenance, and ship with an SBOM.

## 11.2 Actors and assets

| Actor | Trust | Goal |
|-------|-------|------|
| User running `3va` | Trusted. They choose the grants | Run their code safely |
| The user's own script | Semi-trusted: it can do only what the user granted | — |
| Third-party npm/JSR packages | **Untrusted** | May try to escape their grants (malware, typosquatting) |
| Remote servers (registries, APIs) | **Untrusted** | May send malicious responses or be impersonated (MITM) |
| Contributors | Semi-trusted: their changes are reviewed | May introduce bugs or malicious changes |
| CI / release pipeline | Trusted only for signed, pinned workflows | Build and sign releases |

Assets: the user's files and environment secrets, network reach from the user's machine, the integrity of installed packages, and the integrity of 3va releases.

## 11.3 Trust boundaries

```
 ┌──────────────── untrusted ────────────────┐
 │ JS/TS code + node_modules  ·  WASM modules │
 └──────────────────┬────────────────────────┘
        B1: JS builtins  (every privileged op checks vvva_permissions)
 ┌──────────────────▼────────────────────────┐
 │ 3va runtime (Rust): vvva_js, vvva_core,   │
 │ vvva_permissions, vvva_firewall           │
 └───┬──────────────┬──────────────┬─────────┘
  B2: OS (fs,      B3: network     B4: native code
  env, process)    (TLS, firewall)  (FFI / N-API, --allow-ffi)
```

- **B1, JS → Rust builtins.** This is the main boundary. Every builtin that touches the filesystem, network, environment, processes, or FFI calls `PermissionState::check` before acting. The capability model is in [06-permissions](../06-permissions/01-capability-model.md).
- **B2, runtime → OS.** Paths are canonicalized before the prefix check, so `..` and symlink tricks can't escape a granted directory.
- **B3, runtime → network.** Outbound hosts must match `--allow-net`, plaintext protocols need `--allow-insecure`, and the HTTP server applies the per-IP limits of the [firewall](08-firewall.md).
- **B4, runtime → native code.** Native addons run with the full privileges of the process. This is why `--allow-ffi` must name the library path explicitly.
- **B5, registry → package manager.** Tarballs are checked against SRI hashes and provenance signatures, scanned for malware, and never execute install scripts.
- **B6, contributor → release.** Branch rules, required CI gates, DCO, `CODEOWNERS` review, and keyless signing in CI (see [CONTRIBUTING.md](../../CONTRIBUTING.md)).

## 11.4 Attack surface and threats (STRIDE)

| Entry point | Threat | Mitigation |
|-------------|--------|------------|
| Builtins called by untrusted packages | Elevation of privilege: bypass a grant | Every builtin checks permissions (B1), grants are per scope (R2), and CI runs the permission-sandbox fuzz target (`fuzz_permission_sandbox`) and the capability-bypass test suite |
| `fs` paths | Tampering / information disclosure via traversal or symlinks | Canonicalization plus a prefix match against the granted paths |
| `fetch`, WebSocket, TCP, and the mail/chat protocols | Spoofing / MITM; SSRF to internal hosts | TLS with certificate verification and a TLS 1.2 floor; plaintext gated (R4); `--allow-net` matches the real destination host, so granting `api.example.com` never allows `127.0.0.1` |
| Response bodies and protocol parsers | Denial of service via oversized or malformed input | Response-size caps (`fetch` `maxResponseSize`), connect and IO timeouts, and fuzzing of parsers |
| `3va install` | Malicious package or dependency confusion | No install scripts (R3), SRI and provenance checks, malware scanner, OSV audit, and a lockfile with a `registry` field |
| HTTP server | Denial of service | Firewall: per-IP connection and rate limits |
| `3va run <package.json script>` | Unsandboxed delegation | Documented as out of scope; requires explicit consent (`--yes` or a prompt) |
| Release pipeline | Tampering with binaries | Pinned workflows, least-privilege tokens, validated inputs, cosign signatures, SLSA provenance |

## 11.5 Secure-design principles applied

| Principle | How 3va applies it |
|-----------|--------------------|
| Least privilege | Deny by default; grants scoped to a path, host, variable, or package; a read-only `GITHUB_TOKEN` in CI |
| Fail-safe defaults | No grant means denied. Without a TTY, prompts turn into denials. Plaintext and install scripts are off |
| Complete mediation | Permissions are checked on every call, not cached per handle |
| Economy of mechanism | A single `PermissionState` for every builtin; one TLS module (`builtins/tls.rs`) for every client |
| Separation of privilege | Release signing needs the CI OIDC identity plus a tag on protected `main`; security-sensitive paths need `CODEOWNERS` review |
| Open design | All of the code, this threat model, and the verification procedures are public |
| Psychological acceptability | Error messages name the exact flag to grant (`Run with --allow-net=host`), `3va permissions learn` records what a script needs, and interactive prompts are available |

## 11.6 Countering common weaknesses (CWE Top 25)

| Weakness | Countermeasure |
|----------|----------------|
| Memory safety (CWE-787, -125, -416) | Rust. `unsafe` is confined to the V8/N-API/FFI bindings, inventoried with `cargo-geiger`, and exercised under ASan and UBSan with fuzzing |
| Path traversal (CWE-22) | Canonicalized, prefix-matched filesystem grants |
| OS command injection (CWE-78) | Child processes need `--allow-child-process`. `spawn` and `execFile` pass arguments as argv, without a shell. `exec` goes through `sh -c`, as in Node, so callers must not interpolate untrusted input into it |
| Missing authorization (CWE-862) | A permission check in every privileged builtin, plus the capability-bypass tests |
| SSRF (CWE-918) | Outbound checks match the real destination host |
| Cleartext transmission (CWE-319) | R4 |
| Improper certificate validation (CWE-295) | Verification is always on; there is no insecure-TLS switch |
| Use of broken crypto (CWE-327) | SHA-2/SHA-3 and AEAD by default; a FIPS build that rejects non-approved algorithms |
| Uncontrolled resource consumption (CWE-400) | Size caps, timeouts, and firewall limits |
| Hard-coded or leaked credentials (CWE-798) | gitleaks in the pre-commit hook and CI, GitHub push protection, and the [secrets policy](../../SECURITY.md#secrets-and-credentials-policy) |
| Vulnerable dependencies (CWE-1395) | `cargo-deny`, `cargo-vet`, Dependabot, the SBOM, VEX, and the [findings policy](../../SECURITY.md#security-findings-policy) |

## 11.7 Known limits (residual risk)

- Native addons (`--allow-ffi`) and delegated `package.json` scripts run outside the sandbox.
- Raw `net`/`dgram` sockets and the HTTP *server* aren't covered by the plaintext rule. They are transports that the application chooses to use, and TLS is usually terminated in front of them.
- The permission model is enforced in-process. It isn't an OS sandbox (no seccomp or namespaces), so a memory-safety bug in V8 or in native code could bypass it.
- The project has one maintainer (see [GOVERNANCE.md](../../GOVERNANCE.md#continuity)).
