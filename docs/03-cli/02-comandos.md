# 02 - AVAILABLE COMMANDS

## 2.1 Command Catalog

This document describes all available commands in the 3va CLI.

---

## 2.2 Execution Commands

### 2.2.1 `run`

Executes a JavaScript or TypeScript file in a sandboxed environment. Permissions are denied by default.

**Signature:**
```
3va run [OPTIONS] <FILE> [-- <SCRIPT_ARGS>...]
```

**Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `FILE` | `path` (required) | Path to the `.js`, `.ts`, `.jsx`, or `.tsx` file to execute |
| `SCRIPT_ARGS` | `string...` (optional) | Arguments forwarded to the script via `process.argv[2+]`. Must appear after `--`. |

**Options:**
| Option | Type | Description |
|--------|------|-------------|
| `--allow-read=<paths>` | `path,...` | Grants read permission. Comma-separate multiple paths. Omit value (`--allow-read=`) to allow all paths. |
| `--allow-write=<paths>` | `path,...` | Grants write permission. Comma-separate multiple paths. Omit value to allow all paths. |
| `--allow-net=<hosts>` | `string,...` | Grants network access. Comma-separate multiple hosts. Supports `*.domain.com` wildcards. Omit value to allow all hosts. |
| `--allow-env` | `bool` | Allows access to environment variables via `process.env`. |
| `--allow-child-process` | `bool` | Allows spawning child processes via `child_process`. |
| `--audit-log=<path>` | `path` | Writes a JSON audit log of permission checks to the specified file after execution. |
| `--audit-level=<level>` | `"deny"\|"all"` | `deny` (default): log only denied checks. `all`: log every check. |
| `--interactive` | `bool` | Enables the interactive permission prompt at runtime. |

**`process.argv` layout:**
```
process.argv[0]   →  path to the 3va binary
process.argv[1]   →  absolute path to the script
process.argv[2+]  →  arguments passed after --
```

**Transpilation rules (applied automatically):**
| Extension | Treatment |
|-----------|-----------|
| `.ts`, `.mts`, `.cts` | TypeScript type-strip via Oxc |
| `.tsx`, `.jsx` | TypeScript strip + JSX → `React.createElement` |
| `.js`, `.mjs`, others | JSX transform applied automatically if `<Tag` syntax is detected |

**Behavior:**
1. Loads and validates the input file.
2. Initializes `PermissionState` with the granted permissions.
3. Transpiles the source according to the rules above.
4. Sets `process.argv[1]` to the absolute script path; appends `SCRIPT_ARGS` to `process.argv`.
5. Executes the file in the QuickJS engine.
6. Runs the event loop until pending timers, microtasks, and callbacks complete.

**Examples:**
```bash
# Run without permissions
3va run app.ts

# Read permission to a specific directory
3va run app.ts --allow-read=/app/data

# Allow all network access (wildcard)
3va run app.ts --allow-net=

# Specific hosts, comma-separated
3va run app.ts --allow-net=api.example.com,cdn.example.com

# All permissions open (development only)
3va run app.ts --allow-read= --allow-write= --allow-net= --allow-env --allow-child-process

# Pass arguments to the script
3va run server.ts -- --port 3000 --dev
# process.argv → ['3va', '/abs/server.ts', '--port', '3000', '--dev']

# JSX file
3va run component.jsx --allow-read=/src

# Audit log
3va run app.ts --allow-net=api.example.com --audit-log=audit.json --audit-level=all
```

---

## 2.3 Package Manager Commands

### 2.3.1 `install`

Installs one or more packages from a registry. Requires `--allow-net` with the registry host. Never executes post-install scripts.

**Signature:**
```
3va install [<PACKAGE>[@<VERSION>]...] [--allow-net=<registry-host>]
```

**Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `PACKAGE[@VERSION]...` | `string...` (optional) | One or more packages to install. Multiple packages can be specified in a single invocation. If omitted, installs all dependencies from `package.json`. |

**Options:**
| Option | Type | Description |
|--------|------|-------------|
| `--allow-net=<host>` | `string` | Registry host. Determines which registry is used. Use `--allow-net=` to allow all registries. |

**The registry is derived from the host:**
| `--allow-net` | Registry used |
|---------------|---------------|
| `registry.npmjs.org` | npm |
| `registry.yarnpkg.com` | Yarn |
| `jsr.io` | JSR (scoped packages only `@scope/name`) |
| Any other host | Custom npm-compatible registry |

> There is no separate `--registry` flag — the registry is determined exclusively by the host authorized in `--allow-net`, consistent with 3va's capability model.

**Version resolution:**
- If no version is specified, `dist-tags.latest` is used.
- If the requested version does not exist, the 5 closest versions by semver distance are shown.

**After a successful installation:**
- Updates `package.json` with the dependency.
- Writes or updates `3va-lock.json` with the exact version and source registry.

**Examples:**
```bash
# Single package from npm
3va install axios --allow-net=registry.npmjs.org

# Multiple packages in one command
3va install next react react-dom --allow-net=registry.npmjs.org

# Exact version
3va install axios@1.7.2 --allow-net=registry.npmjs.org

# From JSR (requires @scope/name)
3va install @std/path --allow-net=jsr.io

# Install all deps from package.json
3va install --allow-net=registry.npmjs.org

# Without --allow-net: explanatory error with the correct command
3va install axios
# ✗ Network access denied.
#   3va install axios --allow-net=registry.npmjs.org
```

---

### 2.3.2 `reinstall`

Forces reinstallation of a package even if already installed. Useful for repairing a corrupted installation or changing version.

**Signature:**
```
3va reinstall <PACKAGE>[@<VERSION>] --allow-net=<registry-host>
```

**Examples:**
```bash
3va reinstall axios --allow-net=registry.npmjs.org
3va reinstall axios@1.6.0 --allow-net=registry.npmjs.org
3va reinstall @std/path@0.196.0 --allow-net=jsr.io
```

---

### 2.3.3 `update`

Updates installed packages to their latest version, respecting the source registry recorded in `3va-lock.json`.

**Signature:**
```
3va update [<PACKAGE>...] --allow-net=<hosts>
```

**Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `PACKAGE` | `string[]` (optional) | Packages to update. If omitted, updates all. |

**Options:**
| Option | Type | Description |
|--------|------|-------------|
| `--allow-net=<hosts>` | `string` | Authorized hosts, comma-separated. Must cover all required registries. |

**Behavior:**
1. Reads `3va-lock.json` and the `registry` field of each dependency.
2. Groups packages to update by registry.
3. Verifies that `--allow-net` includes all required hosts.
4. If a host is missing, shows the exact command the user should run.
5. Updates each package from its original registry.

**If `--allow-net` is missing:**
```
✗ Update requires network access to:

    registry.npmjs.org        (axios, express)
    jsr.io                    (@std/path)

Run: 3va update --allow-net=registry.npmjs.org,jsr.io
```

**Examples:**
```bash
# Update all packages
3va update --allow-net=registry.npmjs.org,jsr.io

# Update a specific package
3va update axios --allow-net=registry.npmjs.org

# Update multiple packages from different registries
3va update axios @std/path --allow-net=registry.npmjs.org,jsr.io
```

---

## 2.4 Testing Commands

### 2.4.1 `test`

Runs the project's test suite.

**Signature:**
```
3va test [<PATHS>...] [OPTIONS]
```

**Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `PATHS` | `path[]` (optional) | Files or directories to search for tests. By default, searches recursively in `.` |

**Options:**
| Option | Abbreviation | Description |
|--------|-------------|-------------|
| `--watch` | | Runs tests in watch mode: they are re-executed automatically when file changes are detected. |
| `--coverage` | | Generates line and branch coverage report upon completion. |
| `--update-snapshots` | `-u` | Overwrites existing snapshots with current values. |

**Auto-discovery:**

The runner detects files with the following extensions:
- `*.test.js`
- `*.test.ts`
- `*.spec.js`
- `*.spec.ts`

**Examples:**
```bash
# Run all project tests
3va test

# Run tests in a specific directory
3va test tests/

# Run a specific file
3va test tests/auth.test.ts

# Watch mode
3va test --watch

# With coverage
3va test --coverage

# Update outdated snapshots
3va test --update-snapshots
3va test -u

# Combine flags
3va test tests/ --coverage --watch
```

---

## 2.5 Build Commands

### 2.5.1 `bundle`

Bundles an application from a single entry point, resolving imports and applying tree-shaking.

**Signature:**
```
3va bundle <INPUT> [-o <OUTPUT>] [OPTIONS]
```

**Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `INPUT` | `path` (required) | Entry file (application entry point) |

**Options:**
| Option | Abbreviation | Default | Description |
|--------|-------------|---------|-------------|
| `--output <path>` | `-o` | `dist/bundle.js` | Output bundle file path |
| `--split` | | | Generates separate chunks (code splitting) |
| `--minify` | | | Minifies output code |
| `--source-map` | | | Generates `.map` file alongside the bundle |

**Examples:**
```bash
# Basic bundle (output in dist/bundle.js)
3va bundle src/index.ts

# Bundle with custom output
3va bundle src/index.ts -o dist/app.js

# Production bundle
3va bundle src/index.ts --minify --source-map

# Bundle with code splitting
3va bundle src/index.ts --split -o dist/
```

---

## 2.6 Development Commands

### 2.6.1 `dev`

Starts the development server with hot module replacement (HMR) and static file serving.

**Signature:**
```
3va dev [OPTIONS]
```

**Options:**
| Option | Default | Description |
|--------|---------|-------------|
| `--port <N>` | `3000` | Port the server listens on |
| `--host <H>` | `127.0.0.1` | Network address to bind to |
| `--open` | | Opens the browser automatically on start |
| `--public-dir <D>` | `public/` | Static files directory to serve |

**Behavior:**
1. Performs an initial compilation on startup.
2. Watches for changes in `.js`, `.ts`, `.jsx`, `.tsx` files with a 300 ms debounce.
3. On detecting changes, rebuilds and notifies connected clients via HMR.
4. Serves static files from `--public-dir` with correct MIME types.
5. Unmatched routes return `public/index.html` (SPA fallback).
6. If `public/index.html` does not exist, serves a built-in development page.

**HMR (Hot Module Replacement):**
- The SSE endpoint is `/__hmr`.
- 3va automatically injects the HMR client script just before `</body>` in all served HTML files.
- Connected clients receive rebuild notifications without needing to manually reload.

**Examples:**
```bash
# Development server with default configuration
3va dev

# Custom port and host
3va dev --port 8080 --host 0.0.0.0

# Open browser automatically
3va dev --open

# Alternative public directory
3va dev --public-dir www/

# Full configuration
3va dev --port 4000 --host 0.0.0.0 --open --public-dir static/
```

---

## 2.7 Diagnostic Commands

### 2.7.1 `audit`

Audits installed dependencies in up to **three phases**. All three phases run independently: an error in one phase does not cancel the following ones.

**Signature:**
```
3va audit [OPTIONS]
```

**Options:**
| Option | Description |
|--------|-------------|
| `--deny` | Exits with a non-zero error code if any finding of **CRITICAL** or **HIGH** severity is detected. Recommended as a gate in CI/CD pipelines. |
| `--update-cache` | Ignores the local cache (TTL 24 h) and downloads fresh data from the OSV API. |
| `--secrets` | Enables Phase 3: detection of hardcoded secrets in dependency code. |
| `--json` | Outputs machine-readable JSON instead of human-readable format. |

**Audit phases:**

**Phase 1 — Static malware analysis**
Scans extracted code in `node_modules/` for known malicious patterns:
- Data exfiltration (sending environment variables, system credentials)
- Command injection in installation scripts
- Suspicious obfuscation (`eval`, `Function`, base64-encoded strings)
- Cryptocurrency miners (stratum/pool fingerprints)

**Phase 2 — Known vulnerabilities (OSV)**
- Queries `https://api.osv.dev/v1/querybatch` in batches of up to 100 packages.
- Reads `3va-lock.json` for exact versions; if it does not exist, falls back to walking `node_modules/`.
- Caches results in `~/.cache/3va/audit/` for 24 hours.
- If the API is unavailable, uses stale cache and warns the user — never fails due to connectivity issues.
- Only `{ name, version, ecosystem }` is transmitted to OSV. No source code or file paths are sent.

**Phase 3 — Secret detection (requires `--secrets`)**
Scans dependencies for hardcoded secrets using `SecretsScanner`:
- AWS keys (`AKIA...`)
- GitHub tokens (`ghp_`, `ghs_`, `gho_`)
- Private PEM keys (`-----BEGIN ... PRIVATE KEY-----`)
- JWT tokens (`eyJ...`)
- Stripe API keys (`sk_live_...`)
- Other common secret patterns

**OSV Severity:**
| CVSS v3 Range | Label |
|---------------|-------|
| ≥ 9.0 | CRITICAL |
| ≥ 7.0 | HIGH |
| ≥ 4.0 | MEDIUM |
| < 4.0 | LOW |

Severity is determined in this order: CVSS v3 vector → CVSS v2 score → `database_specific.severity` field (GitHub Advisory format).

**JSON format (`--json`):**
```json
{
  "passed": true,
  "phases": {
    "malware": {
      "findings": []
    },
    "osv": {
      "packages_scanned": 12,
      "vulnerable": 0,
      "findings": []
    },
    "secrets": {
      "findings": []
    }
  }
}
```
> When using `--json`, all human-readable output is suppressed and only JSON is emitted to stdout.

**Local cache:**
```
~/.cache/3va/audit/
  lodash@4.17.20.json
  axios@1.7.9.json
  @scope__pkg@2.0.0.json    ← @ and / sanitized in filename
```

**Examples:**
```bash
# Standard audit (malware + CVEs)
3va audit

# CI/CD: fail on HIGH or CRITICAL
3va audit --deny

# Include secret detection
3va audit --secrets

# Force fresh OSV data
3va audit --update-cache

# JSON output (for integration with other tools)
3va audit --json

# Full audit with JSON output for CI
3va audit --secrets --deny --json
```

---

### 2.7.2 `sandbox`

Opens an isolated JavaScript REPL in a sandbox.

**Signature:**
```
3va sandbox
```

**Behavior:**
- In TTY: opens the interactive REPL with multi-line support. The bracket matcher tracks parentheses, brackets, and braces to determine when an expression is complete.
- In pipe / CI (non-TTY stdin): exits immediately without blocking the process.
- Objects are displayed in Node.js-style JSON format.
- Statements that produce `undefined` display it explicitly.

**REPL session commands:**
| Command | Description |
|---------|-------------|
| `.help` | Shows the list of available commands |
| `.exit` | Exits the REPL |
| `.clear` | Clears the current session context |
| `.allow-read <path>` | Grants read permission to the specified path in the session |
| `.allow-net <host>` | Grants network access to the specified host in the session |
| `.permissions` | Lists all currently granted permissions in the session |

**Session example:**
```
3va sandbox
> 1 + 1
2
> const obj = { a: 1, b: [2, 3] }
undefined
> obj
{ "a": 1, "b": [2, 3] }
> .allow-net api.example.com
Granted: net → api.example.com
> .permissions
Granted permissions:
  net: api.example.com
> .exit
```

---

### 2.7.3 `doctor`

Checks the health of the runtime and system environment.

**Signature:**
```
3va doctor
```

Verifies the binary installation, environment configuration, lock files, and other system requirements. Useful for diagnosing installation or configuration issues.

---

## 2.8 Global Option

### `--accessible`

Enables accessible mode: disables colors, animations, and special characters in output. Compliant with EN 301 549 for screen readers and Braille terminals.

Can be combined with any subcommand:

```bash
3va --accessible run app.ts
3va --accessible install axios --allow-net=registry.npmjs.org
3va --accessible audit --deny
3va --accessible test --coverage
```

---

*Commands compliant with IEEE 829 and secure-by-default CLI design.*
