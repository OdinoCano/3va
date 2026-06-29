# 02 - AVAILABLE COMMANDS

## 2.1 Command Catalog

This document describes all available commands in the 3va CLI.

### Quick aliases

All main commands have short aliases for faster use:

| Full command | Alias(es) |
|---|---|
| `3va run` | `3va r` |
| `3va install` | `3va i`, `3va add` |
| `3va test` | `3va t`, `3va spec` |
| `3va dev` | `3va d` |
| `3va bundle` | `3va b` |
| `3va workspace` | `3va ws` |
| `3va sandbox` | `3va sh`, `3va shell` |

```bash
3va r app.ts --allow-net=api.example.com
3va i axios --allow-net=registry.npmjs.org
3va t --watch
3va d --port 8080
```

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
| `--port <N>` / `-p <N>` | `u16` | Port to listen on. Sets `PORT` env var so the script can read `process.env.PORT`. Can also be set via `3va.config.ts` (`run.port`) or env var `3VA_RUN_PORT`. |
| `--allow-read=<paths>` | `path,...` | Grants read permission. Comma-separate multiple paths. Omit value (`--allow-read=`) to allow all paths. |
| `--allow-write=<paths>` | `path,...` | Grants write permission. Comma-separate multiple paths. Omit value to allow all paths. |
| `--allow-net=<hosts>` | `string,...` | Grants network access. Comma-separate multiple hosts. Supports `*.domain.com` wildcards. Omit value to allow all hosts. |
| `--allow-env[=VARS]` | `string,...` | Allows access to environment variables via `process.env`. Omit value to expose all; `--allow-env=NODE_ENV,PATH` scopes access to the listed variables. |
| `--allow-child-process` | `bool` | Allows spawning child processes via `child_process`. |
| `--allow-ffi[=paths]` | `path,...` | Allows loading native libraries (NAPI/FFI). Omit value to allow all, or restrict to specific library paths. |
| `--inspect[=HOST:PORT]` | `string` | Activates the Chrome DevTools Protocol (CDP) inspector (default `127.0.0.1:9229`). `debugger;` statements pause execution. |
| `--audit-log=<path>` | `path` | Writes a JSON audit log of permission checks to the specified file after execution. |
| `--audit-level=<level>` | `"deny"\|"all"` | `deny` (default): log only denied checks. `all`: log every check. |
| `--prof` | `bool` | Enables the CPU sampling profiler. |
| `--prof-out=<path>` | `path` | Output path for the profile (default: `profile.cpuprofile`). |
| `--prof-interval=<ms>` | `integer` | Sampling interval in milliseconds (default: 10). |
| `--flamegraph=<path>` | `path` | Also emit a flamegraph SVG (requires `--prof`). |

> There is no `--interactive` flag. Interactive permission prompts activate
> automatically when stderr is a TTY; see
> [06-permissions/05-interactive-prompts.md](../06-permissions/05-interactive-prompts.md).

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

# Run with a specific port (sets process.env.PORT)
3va run server.ts --port 8080
3va run server.ts -p 8080

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
| `--watch` | `-w` | Runs tests in watch mode: they are re-executed automatically when file changes are detected. |
| `--coverage` | | Generates statement/line coverage report upon completion (branch coverage is not tracked; see [09-testing/03-coverage.md](../09-testing/03-coverage.md)). |
| `--update-snapshots` | `-u` | Overwrites existing snapshots with current values. |
| `--concurrency` | | Maximum concurrent test files (`0` = CPU count, default `0`) |
| `--reporter` | | Output format: `terminal` \| `json` \| `junit` \| `tap` \| `dot` (default: `terminal`) |
| `--reporter-file` | | Write reporter output to a file instead of stdout (e.g. `--reporter-file=junit.xml`) |

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
| `--no-csp` | | Disables the Content-Security-Policy header (enabled by default since v2.0.0) |

**Framework detection:**

3va automatically detects the project framework when running `3va dev`. If any of the following frameworks is detected, 3va delegates to the framework's own dev server instead of using the built-in bundler:

| Framework | Detection file | Delegates to |
|-----------|---------------|--------------|
| Astro | `astro.config.*` | `astro dev` |
| Next.js | `next.config.*` | `next dev` |
| Nuxt | `nuxt.config.*` | `nuxi dev` |
| SvelteKit | `svelte.config.*` + `@sveltejs/kit` | `vite dev` |
| Remix | `remix.config.*` | `remix dev` |
| Gatsby | `gatsby-config.*` | `gatsby develop` |
| SolidStart | `app.config.*` | `vinxi dev` |
| Qwik | `qwik.config.*` | `qwik dev` |

When a framework is detected, the flags `--port`, `--host`, and `--open` are forwarded to the framework's CLI. The `--public-dir` flag is ignored in delegation mode.

**Behavior (without framework detection):**
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
| `--deny` | Exits with a non-zero error code if any OSV vulnerability of **CRITICAL** or **HIGH** severity is detected. Recommended as a gate in CI/CD pipelines. (Secrets behave differently: only **Critical** secrets fail the audit, with or without `--deny`; High/Medium secrets produce a warning.) |
| `--update-cache` | Ignores the local cache (TTL 24 h) and downloads fresh data from the OSV API. |
| `--secrets` | Enables Phase 3: detection of hardcoded secrets in the **current project's source files**. |
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
Recursively scans the current working directory's source files for hardcoded
secrets using `SecretsScanner`:
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
      "clean": true
    },
    "osv": {
      "total_packages": 12,
      "packages_with_vulns": 0,
      "total_vulns": 0,
      "critical": 0,
      "high": 0,
      "findings": []
    },
    "secrets": {
      "scanned": true,
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
3va sandbox [--plugin <name|path>,...]
```

**Options:**
| Option | Description |
|--------|-------------|
| `--plugin <p>` | Loads a REPL plugin. Built-ins: `inspect`, `history`; or a `.js`/`.ts` file path. Comma-separate multiple plugins. |

**Behavior:**
- In TTY: opens the interactive REPL with multi-line support. The bracket matcher tracks parentheses, brackets, and braces to determine when an expression is complete.
- In pipe / CI (non-TTY stdin): evaluates piped input, then exits without blocking the process.
- Objects are displayed in Node.js-style JSON format.
- Statements that produce `undefined` display it explicitly.

**REPL session commands:**
| Command | Description |
|---------|-------------|
| `.help` | Shows the list of available commands |
| `.permissions` | Lists all currently granted permissions in the session |
| `.allow-read=PATH` | Grants read permission to the specified path in the session |
| `.allow-write=PATH` | Grants write permission to the specified path in the session |
| `.allow-net=HOST` | Grants network access to the specified host in the session |
| `.allow-env` | Grants environment variable access in the session |
| `.clear` | Resets the JS context (re-creates the engine) |
| `exit` / `quit` / `^D` | Leaves the sandbox (no leading dot) |

**Session example:**
```
3va sandbox
3va> 1 + 1
2
3va> const obj = { a: 1, b: [2, 3] }
undefined
3va> obj
{ "a": 1, "b": [2, 3] }
3va> .allow-net=api.example.com
  ✓ Network granted: api.example.com
3va> .permissions
  ✓ Network("api.example.com")
3va> exit
Leaving sandbox...
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

### 2.7.4 `prof`

Analyzes a `.cpuprofile` file produced by `3va run --prof` and prints the top hot functions or generates a flamegraph SVG.

**Signature:**
```
3va prof <FILE> [--top <N>] [--format <text|flamegraph>] [--out <PATH>]
```

**Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `FILE` | `path` (required) | Path to the `.cpuprofile` JSON file |
| `--top <N>` | `integer` | Number of hottest functions to display (default: 20) |
| `--format <fmt>` | `"text"\|"flamegraph"` | Output format: `text` (default) prints a ranked table; `flamegraph` writes an SVG file |
| `--out <PATH>` | `path` | Output path for the flamegraph SVG (only used with `--format=flamegraph`, default: `flamegraph.svg`) |

**Collecting a profile:**
```bash
# Record a profile while running a script
3va run app.ts --prof --prof-out profile.cpuprofile --prof-interval 10
```

**Examples:**
```bash
# Print top 20 hot functions
3va prof profile.cpuprofile

# Print top 5 hot functions
3va prof profile.cpuprofile --top 5

# Generate flamegraph SVG
3va prof profile.cpuprofile --format flamegraph --out flame.svg
```

**Sample output (`--format=text`):**
```
Self%  Function
--------------------------------------------------
 34%  processRequest (server.ts:42)
 18%  parseJSON (utils.ts:11)
  9%  validateSchema (middleware.ts:87)
```

---

## 2.8 Process Manager Commands

3va includes a built-in process manager for production deployments, similar to PM2. Managed processes run as daemons with log capture, PID tracking, and graceful shutdown.

Process metadata is stored in `~/.3va/processes/<name>.json`. Logs are written to `~/.3va/processes/<name>.log`.

### 2.8.1 `start`

Starts an entry file as a managed background process (daemon). The process runs in a new session (`setsid`) and continues running after the CLI exits.

**Signature:**
```
3va start [--name <NAME>] <ENTRY> [-- <ARGS>...]
```

**Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `ENTRY` | `path` (required) | Entry file (`.js`, `.ts`, `.jsx`, `.tsx`) to execute |
| `--name <NAME>` / `-n` | `string` | Custom process name (default: derived from the entry filename stem) |
| `--port <N>` / `-p <N>` | `u16` | Port to listen on. Sets `PORT` env var for the managed process. |
| `ARGS` | `string...` | Arguments forwarded to the script. Must appear after `--`. |

**Behavior:**
1. Resolves the entry file path relative to the current directory.
2. Spawns `3va run <ENTRY>` in a new process group via `setsid()`.
3. Captures stdout and stderr to `~/.3va/processes/<name>.log`.
4. Writes process metadata (PID, CWD, arguments, timestamps) to `~/.3va/processes/<name>.json`.
5. Returns immediately — the daemon continues running independently.

**Examples:**
```bash
# Start with auto-naming (process name: "app")
3va start app.js

# Start with a custom name
3va start --name my-api server.js

# Start on a specific port (sets process.env.PORT)
3va start --port 8080 server.js
3va start -p 8080 server.js

# Start with arguments forwarded to the script
3va start --name worker worker.js -- --queue emails --concurrency 5

# Start in a project subdirectory
cd /opt/myapp && 3va start dist/bundle.js
```

---

### 2.8.2 `stop`

Stops a managed process gracefully (SIGTERM), then forcibly (SIGKILL) if it does not exit within 1.5 seconds.

**Signature:**
```
3va stop <NAME>
```

**Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `NAME` | `string` (required) | Name of the process to stop |

**Behavior:**
1. Loads the process metadata from `~/.3va/processes/<name>.json`.
2. Sends `SIGTERM` to the process.
3. Waits 1.5 seconds.
4. If the process is still alive, sends `SIGKILL`.
5. Updates the process status to `stopped`.

> On Windows there is no SIGTERM equivalent; the runtime first attempts a
> graceful `taskkill` (WM_CLOSE), waits 1.5 seconds, and forces termination
> with `taskkill /F` if the process is still alive.

**Examples:**
```bash
3va stop my-api
3va stop worker
```

---

### 2.8.3 `restart`

Restarts a managed process by stopping it and starting it again with the same entry, arguments, and working directory.

**Signature:**
```
3va restart <NAME>
```

**Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `NAME` | `string` (required) | Name of the process to restart |

**Behavior:**
1. Loads the process metadata (entry, CWD, arguments).
2. Stops the existing process (if running).
3. Starts a new process with the same configuration.
4. The new PID is different from the previous one.

**Examples:**
```bash
3va restart my-api
3va restart worker
```

---

### 2.8.4 `status`

Displays the status of one or all managed processes. Status is colour-coded in terminals
(green = `running`, yellow = `stopped`, red = `error`). If `NAME` is omitted, all processes are listed.

**Signature:**
```
3va status [<NAME>]
```

**Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `NAME` | `string` (optional) | Process name to inspect. Lists all processes if omitted. |

**Status values:**
| Status | Meaning |
|--------|---------|
| `running` | Process is alive according to PID check |
| `stopped` | Process was deliberately stopped |
| `error` | Process exited unexpectedly |

**Examples:**
```bash
# List all processes
3va status

# Inspect a specific process
3va status my-api
```

**Sample output:**
```
  Name                 PID      Status   Restarts   Entry
  -------------------- -------- -------- ---------- --------------------
  my-api               574912   running  0          /opt/myapp/server.js
  worker               574988   stopped  1          /opt/myapp/worker.js
```

The `Restarts` column counts how many times the process has been restarted via
`3va restart` since it was first started.

---

### 2.8.5 `logs`

Displays the log file of a managed process.

**Signature:**
```
3va logs <NAME> [--lines <N>]
```

**Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `NAME` | `string` (required) | Process name to view logs for |
| `--lines <N>` / `-l` | `integer` | Number of lines to show from the tail (default: 50) |

**Behavior:**
1. Loads the process metadata to locate the log file at `~/.3va/processes/<name>.log`.
2. Reads the last N lines from the log file.
3. Prints them to stdout.

**Examples:**
```bash
# Show last 50 lines
3va logs my-api

# Show last 200 lines
3va logs worker --lines 200

# Follow log (via tail)
tail -f $(3va logs my-api)
```

---

### 2.8.6 `delete`

Stops (if running) and permanently removes a process from 3va's management, including its log file and metadata.

**Signature:**
```
3va delete <NAME>
```

**Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `NAME` | `string` (required) | Process name to delete |

**Behavior:**
1. If the process is running, stops it (SIGTERM → SIGKILL).
2. Removes `~/.3va/processes/<name>.json`.
3. Removes `~/.3va/processes/<name>.log`.
4. The process cannot be managed again until `start` is called.

**Examples:**
```bash
3va delete my-api
3va delete worker
```

---

## 2.9 Workspace Commands

Commands for managing monorepo workspaces. A workspace is detected automatically when `package.json` contains a `workspaces` field.

### 2.9.1 `workspace install`

Installs all dependencies across every workspace package.

**Signature:**
```
3va workspace install [--allow-net=<hosts>]
```

**Examples:**
```bash
3va workspace install --allow-net=registry.npmjs.org
```

---

### 2.9.2 `workspace list`

Lists all packages discovered in the workspace.

**Signature:**
```
3va workspace list
```

**Examples:**
```bash
3va workspace list
# packages/core  (1.0.0)
# packages/ui    (1.0.0)
# apps/web       (0.1.0)
```

---

### 2.9.3 `workspace info`

Displays workspace root, package count, and global content-store statistics.

**Signature:**
```
3va workspace info
```

---

### 2.9.4 `workspace run`

Runs a script (defined in `package.json` `"scripts"`) in every workspace package that defines it, in topological dependency order.

**Signature:**
```
3va workspace run <SCRIPT> [--affected] [--base <BRANCH>] [--parallel] [--concurrency <N>]
```

**Parameters:**
| Parameter | Type | Description |
|-----------|------|-------------|
| `SCRIPT` | `string` (required) | Script name as defined in each package's `"scripts"` field |
| `--affected` | `bool` | Only run in packages affected since the base branch |
| `--base <BRANCH>` | `string` | Base branch for affected detection (default: `main`) |
| `--parallel` | `bool` | Run in parallel, ignoring topological ordering |
| `--concurrency <N>` | `integer` | Max concurrent packages (default from config, or 4) |

**Examples:**
```bash
3va workspace run build
3va workspace run test --affected --base main
3va workspace run lint --parallel --concurrency 8
```

---

### 2.9.5 `workspace graph`

Visualizes the workspace dependency graph.

**Signature:**
```
3va workspace graph
```

---

## 2.10 Store Commands

Commands for managing the global content-addressable package cache stored at `~/.3va/store/`.

### 2.10.1 `store status`

Displays statistics for the global package store (entry count, disk usage).

**Signature:**
```
3va store status
```

---

### 2.10.2 `store repair`

Removes corrupt or incomplete cache entries left by a prior crash or interrupted download.

**Signature:**
```
3va store repair
```

---

### 2.10.3 `store prune`

Removes packages from the global store that are not referenced by any lockfile in the current project.

**Signature:**
```
3va store prune
```

---

### 2.10.4 `store verify`

Verifies that every cached package has a complete extraction (no missing files).

**Signature:**
```
3va store verify
```

---

## 2.11 Global Option (`--accessible`)

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

## 2.12 Publishing Commands

### 2.12.1 `pack`

Creates a tarball (`.tgz`) of the current package, respecting the `files` field in `package.json` and default excludes (`node_modules`, `.git`, `*.lock`, etc.).

**Signature:**
```
3va pack [--output=<path>] [--dry-run]
```

**Options:**
| Option | Type | Description |
|--------|------|-------------|
| `--output=<path>` | `path` (optional) | Destination path for the `.tgz`. Defaults to `<name>-<version>.tgz` in the current directory. |
| `--dry-run` | `bool` | Lists files that would be packed without writing the archive. |

**Behavior:**
1. Reads `name` and `version` from `package.json`.
2. Collects all files under the project root, excluding `node_modules`, `.git`, `*.tgz`, and lock files.
3. If the `files` field is set in `package.json`, only matching paths are included (plus `package.json` and `README` unconditionally).
4. Writes a gzip-compressed tar archive with all entries under the `package/` prefix (npm convention).

**Examples:**
```bash
# Create <name>-<version>.tgz in the current directory
3va pack

# Specify output path
3va pack --output=dist/my-lib.tgz

# Preview what would be packed
3va pack --dry-run
```

---

### 2.12.2 `publish`

Publishes the current package to a registry. Packs the package into a temporary tarball and uploads it via the npm CouchDB PUT API.

**Signature:**
```
3va publish [--registry=<url>] [--dry-run] [--access=<public|restricted>]
```

**Options:**
| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `--registry=<url>` | `string` | `https://registry.npmjs.org` | Registry base URL. |
| `--dry-run` | `bool` | `false` | Pack to a temp file but do not upload. |
| `--access=<scope>` | `"public"\|"restricted"` | — | Access level for scoped packages. |

**Authentication:** reads `_authToken` for the registry host from `~/.npmrc`. Run `3va login` first.

**Examples:**
```bash
# Publish to npm
3va publish --allow-net=registry.npmjs.org

# Publish to a private registry
3va publish --registry=https://registry.corp.internal --allow-net=registry.corp.internal

# Dry run (pack only, no upload)
3va publish --dry-run
```

---

### 2.12.3 `login`

Authenticates with a registry and saves the bearer token to `~/.npmrc`.

**Signature:**
```
3va login [--registry=<url>]
```

**Options:**
| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `--registry=<url>` | `string` | `https://registry.npmjs.org` | Registry to authenticate against. |

**Behavior:** prompts for `Username` and `Password`, sends a CouchDB PUT to `/-/user/org.couchdb.user:<name>`, and writes `//host/:_authToken=<token>` to `~/.npmrc`.

**Examples:**
```bash
3va login
3va login --registry=https://registry.corp.internal
```

---

### 2.12.4 `logout`

Removes the stored auth token for a registry from `~/.npmrc`.

**Signature:**
```
3va logout [--registry=<url>]
```

**Examples:**
```bash
3va logout
3va logout --registry=https://registry.corp.internal
```

---

### 2.12.5 `link`

Creates a symlink between a local package and `node_modules/`, enabling development of inter-dependent packages without publishing.

**Signature:**
```
3va link [<package>]
```

**Behavior:**
- **No argument:** registers the current package globally in `~/.3va/linked/<name>`.
- **With `<package>`:** creates `node_modules/<package>` → `~/.3va/linked/<package>`. The target must have been globally linked first.

**Examples:**
```bash
# In your library directory — register it globally
cd my-lib && 3va link

# In the consuming project — link the library into node_modules
cd my-app && 3va link my-lib
```

---

### 2.12.6 `unlink`

Removes a symlink created by `3va link`.

**Signature:**
```
3va unlink [<package>]
```

**Behavior:**
- **No argument:** removes the global registration of the current package.
- **With `<package>`:** removes `node_modules/<package>` if it is a symlink.

**Examples:**
```bash
3va unlink            # remove global registration of current package
3va unlink my-lib     # remove node_modules/my-lib symlink
```

---

### 2.12.7 `init`

Interactively creates a `package.json` in the current directory.

**Signature:**
```
3va init [--yes]
```

**Options:**
| Option | Description |
|--------|-------------|
| `--yes` | Skip all prompts; accept all defaults. |

**Prompted fields:** `name` (default: directory name), `version` (1.0.0), `description`, `main` (index.js), `author`, `license` (MIT).

**Examples:**
```bash
3va init          # interactive
3va init --yes    # all defaults, no prompts
```

---

### 2.12.8 `why`

Explains why a package is installed by tracing direct and transitive dependency paths.

**Signature:**
```
3va why <package>
```

**Sources checked (in order):**
1. `dependencies`, `devDependencies`, `peerDependencies`, `optionalDependencies` in `package.json`.
2. `dependencies` of every entry in `3va-lock.json`.
3. `dependencies` / `peerDependencies` fields of every package in `node_modules/`.

**Examples:**
```bash
3va why lodash
3va why typescript
```

**Sample output:**
```
  Why is lodash installed?

  Direct dependencies (^4.17.21): my-app
  Transitive: required by express
  Transitive: webpack → lodash (^4.17.11)
```

---

*Commands compliant with IEEE 829 and secure-by-default CLI design.*
