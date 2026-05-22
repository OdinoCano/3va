# 01 - PACKAGE MANAGER SPECIFICATION

## 1.1 Overview

3va's Package Manager (PM) is a dependency manager that is secure by default. It prioritizes supply chain security over convenience: no network call happens without explicit user permission.

## 1.2 Design Philosophy

### 1.2.1 The Registry is Defined by `--allow-net`

Unlike npm/yarn/pnpm, 3va **does not have a `--registry` flag**. The host the user authorizes in `--allow-net` *is* the registry. This is consistent with the runtime's capability model:

```bash
# El host autorizado determina el registry
3va install axios --allow-net=registry.npmjs.org      # → npm
3va install axios --allow-net=registry.yarnpkg.com    # → Yarn
3va install @std/path --allow-net=jsr.io              # → JSR
```

Having a separate `--registry` flag would duplicate authorization and break the security model.

### 1.2.2 Network Denied by Default

```bash
3va install axios
# ✗ Network access denied.
#   3va install axios --allow-net=registry.npmjs.org
#   3va install axios --allow-net=registry.yarnpkg.com
#   3va install axios --allow-net=jsr.io
```

### 1.2.3 Comparison with Traditional Managers

| Feature | npm | yarn | 3va PM |
|---------|-----|------|--------|
| Network by default | Yes | Yes | **No** |
| Registry flag | `--registry` | `--registry` | `--allow-net` |
| Post-install scripts | Default | Default | **Disabled** |
| Signature verification | Optional | Optional | **Mandatory** |
| Malware analysis | No | No | **Yes** |
| CVE audit (OSV) | No | No | **Yes** |
| Multi-registry per project | No | No | **Yes** |
| Per-package origin in lockfile | No | No | **Yes** |

---

## 1.3 Supported Registries

### 1.3.1 npm (`registry.npmjs.org`)

API compatible with npm registry. Returns JSON with `versions` and `dist-tags.latest` fields.

```bash
3va install axios --allow-net=registry.npmjs.org
3va install axios@1.7.2 --allow-net=registry.npmjs.org
```

### 1.3.2 Yarn (`registry.yarnpkg.com`)

Same protocol as npm. The authorized host determines that Yarn is used as the source.

```bash
3va install react --allow-net=registry.yarnpkg.com
```

### 1.3.3 JSR (`jsr.io`)

Only accepts scoped packages (`@scope/name`). Uses the endpoint:
`GET https://jsr.io/api/scopes/{scope}/packages/{name}/versions`

Response: `{ "items": [{ "version": "..." }] }`

```bash
3va install @std/path --allow-net=jsr.io
3va install @std/path@0.196.0 --allow-net=jsr.io

# Error: paquete sin scope no válido en JSR
3va install axios --allow-net=jsr.io
# ✗ JSR only supports scoped packages (e.g. @scope/name)
```

### 1.3.4 Registry Custom

Any host that does not match the three above is treated as an npm-compatible registry:

```bash
3va install my-pkg --allow-net=registry.mycompany.com
```

---

## 1.4 Subcommands

### 1.4.1 `install`

```bash
3va install <package>[@<version>] --allow-net=<registry-host>
```

**Flow:**
1. Validate package name and version.
2. Check `--allow-net` — if missing, error with command suggestions.
3. Derive registry from authorized host.
4. Query the registry (verify package existence).
5. Resolve version: uses `latest` if not specified; if the version does not exist, shows the 5 closest.
6. Verify package signature.
7. Update `package.json`.
8. Regenerate `3va-lock.json` preserving previous registries and recording the new package's registry.

**Already installed package detection:**
```bash
3va install axios --allow-net=registry.npmjs.org
# ✓ axios@1.7.2 is already installed.
#   Use 'reinstall' to force reinstall.
```

**Version not found — nearby version suggestions:**
```bash
3va install axios@99.0.0 --allow-net=registry.npmjs.org
# ✗ Version axios@99.0.0 not found in registry.
#
#   Versions available near 99.0.0:
#     axios@1.7.9
#     axios@1.7.8
#     axios@1.7.7
#     axios@1.7.6
#     axios@1.7.5
```

### 1.4.2 `reinstall`

Forces reinstallation even if the package is already installed.

```bash
3va reinstall <package>[@<version>] --allow-net=<registry-host>
```

### 1.4.3 `update`

Updates packages to their latest version, respecting the source registry recorded in the lockfile.

```bash
# Update all packages
3va update --allow-net=<all-necessary-hosts>

# Update specific packages
3va update axios @std/path --allow-net=registry.npmjs.org,jsr.io
```

**If `--allow-net` does not cover all required registries:**
```bash
3va update
# ✗ Update requires network access to:
#
#     registry.npmjs.org        (axios, express)
#     jsr.io                    (@std/path)
#
# Run: 3va update --allow-net=registry.npmjs.org,jsr.io
```

**Internal flow:**
1. Read `3va-lock.json`.
2. Determine which packages to update (all or specified).
3. Read the `registry` field of each package in the lockfile.
4. Verify that `--allow-net` covers all required registries.
5. For each package, reinstall from its original registry.

**Note:** `update` never changes a package's registry. To migrate to another registry, use `install` explicitly.

---

## 1.5 Multi-Registry per Project

A project can have dependencies from different registries simultaneously. The lockfile records the origin of each:

```json
{
  "dependencies": {
    "axios":     { "version": "1.7.2",   "registry": "registry.npmjs.org" },
    "react":     { "version": "18.3.1",  "registry": "registry.yarnpkg.com" },
    "@std/path": { "version": "0.196.0", "registry": "jsr.io" }
  }
}
```

To update this project:
```bash
3va update --allow-net=registry.npmjs.org,registry.yarnpkg.com,jsr.io
```

---

## 1.6 Version Resolution

### 1.6.1 Unspecified Version

Uses `dist-tags.latest` from the registry (npm/Yarn) or the last entry in `items[]` (JSR).

### 1.6.2 Specified and Existing Version

```bash
3va install axios@1.7.2 --allow-net=registry.npmjs.org
# ✓ Version axios@1.7.2 exists
```

### 1.6.3 Specified and Non-Existent Version

Calculates the 5 closest versions by numeric semver distance:
`score = major × 1_000_000 + minor × 1_000 + patch`

Suggestions always follow the `name@version` format.

### 1.6.4 Package Specification Format

| Format | Example | Result |
|--------|---------|--------|
| Name only | `axios` | Installs `latest` |
| Name + version | `axios@1.7.2` | Installs exact version |
| Scoped | `@std/path` | Installs `latest` from scope |
| Scoped + version | `@std/path@0.196.0` | Installs exact version |

---

## 1.7 `package.json` Format

```json
{
  "name": "my-package",
  "version": "1.0.0",
  "description": "",
  "main": "index.js",
  "type": "module",
  "dependencies": {
    "axios": "1.7.2",
    "@std/path": "0.196.0"
  }
}
```

3va writes exact versions (without `^` or `~`) when installing to ensure reproducibility.

---

## 1.8 Security

### 1.8.1 Post-install Scripts

Disabled by default. The `postinstall`, `install`, `preinstall` scripts defined in dependency `package.json` files **are not executed**.

### 1.8.2 Signature Verification

Each package goes through `SignatureVerifier` (SHA-256/SHA-512) before being registered in the lockfile.

### 1.8.3 Malware Scanner

`MalwareScanner` analyzes the package content before installing it.

### 1.8.4 Regulatory Compliance

- **NIS2**: static code verification and restriction of third-party binary execution.
- **eIDAS**: cryptographic signature verification mechanisms for packages.

---

*Implemented in `crates/pm/src/` (`lib.rs`, `lockfile.rs`, `fetcher.rs`, `resolver.rs`, `signature_verifier.rs`, `malware_scanner.rs`).*
