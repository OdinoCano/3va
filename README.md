# 3va

> *Veni, Vidi, Vici, Abiit — He came, he saw, he conquered, he left.*

**3va** is a secure-by-default JavaScript and TypeScript runtime written in Rust. The name is a tribute to the philosophy of Satoshi Nakamoto: build something that matters, ship it to the world, and step away.

---

## Philosophy

The JavaScript ecosystem is broken from a supply chain security perspective. `3va` reimagines the runtime from the ground up, taking inspiration from QubesOS, WASI, and the Chrome sandbox rather than from Node.js.

- **Deny by default.** No access to the filesystem, network, environment variables, or child processes unless you explicitly grant it.
- **Capability-based permissions.** Every sensitive operation requires a flag. Scopes can be narrowed to specific hosts, paths, or variables.
- **Untrusted dependencies.** The package manager refuses to execute post-install scripts. Packages are treated as untrusted code, not trusted collaborators.
- **Post-quantum ready.** The `vvva_crypto` crate is built with post-quantum cryptography primitives in mind.

---

## Quick Start

### Prerequisites

- [Rust toolchain](https://rustup.rs/) (edition 2021 or later)

### Build

```bash
git clone https://github.com/yourusername/3va.git
cd 3va
cargo build --release
```

### Install the binary

```bash
# Temporary (current session only)
export PATH="$PWD/target/release:$PATH"

# Permanent
sudo cp target/release/3va /usr/local/bin/
```

### Run a script

```bash
3va run app.ts
3va run app.ts --allow-read=/app/config --allow-net=api.example.com
```

---

## CLI Reference

### Global flags

| Flag | Description |
|------|-------------|
| `--accessible` | Enable accessible output mode |

---

### `3va run <file>`

Run a JavaScript or TypeScript file inside a sandboxed environment.

```bash
3va run app.ts
3va run app.ts --allow-net=api.example.com --allow-read=/data --allow-env=HOME
```

| Flag | Description |
|------|-------------|
| `--allow-read[=<path>]` | Grant read access (optionally scoped to a path) |
| `--allow-write[=<path>]` | Grant write access (optionally scoped to a path) |
| `--allow-net[=<host>]` | Grant network access (optionally scoped to a host) |
| `--allow-env[=<var>]` | Grant environment variable access (optionally scoped) |
| `--allow-child-process` | Allow spawning child processes |
| `--interactive` | Start an interactive session after running the file |

All permissions are deny-by-default. Omitting a flag means the capability is blocked.

---

### `3va install <package>[@version]`

Install a package from npm, Yarn, or JSR. The registry is determined by the `--allow-net` host — no separate `--registry` flag is needed.

```bash
3va install axios --allow-net=registry.npmjs.org
3va install react@18 --allow-net=registry.yarnpkg.com
3va install @std/path --allow-net=jsr.io
```

| Supported registry | Host value |
|--------------------|------------|
| npm | `registry.npmjs.org` |
| Yarn | `registry.yarnpkg.com` |
| JSR | `jsr.io` |

Post-install scripts are never executed.

---

### `3va reinstall`

Reinstall all packages listed in the lockfile.

```bash
3va reinstall
```

---

### `3va update`

Update installed packages to their latest compatible versions.

```bash
3va update
```

---

### `3va bundle <input>`

Bundle a JavaScript or TypeScript application into a single output file.

```bash
3va bundle src/index.ts
3va bundle src/index.ts -o dist/bundle.js
3va bundle src/index.ts --minify --source-map
3va bundle src/index.ts --split
```

| Flag | Description |
|------|-------------|
| `-o <path>` | Output file path (default: derived from input) |
| `--split` | Enable code splitting |
| `--minify` | Minify output |
| `--source-map` | Emit a source map |

---

### `3va test [paths...]`

Run tests using the built-in Jest-compatible test runner. Supports `describe`, `test`, `expect`, all common matchers, and snapshots.

```bash
3va test
3va test tests/unit
3va test --watch
3va test --coverage
3va test --update-snapshots
```

| Flag | Description |
|------|-------------|
| `--watch` | Re-run tests on file change |
| `--coverage` | Collect and report code coverage |
| `--update-snapshots` | Overwrite existing snapshots with current output |

---

### `3va audit`

Audit installed packages in three phases:

1. **Malware scan** — static analysis of `node_modules` for known malicious patterns
2. **OSV CVE scan** — queries [api.osv.dev](https://api.osv.dev/v1/querybatch) for known vulnerabilities (24-hour local cache)
3. **Secrets detection** — scans for leaked credentials and tokens (opt-in)

```bash
3va audit
3va audit --secrets
3va audit --deny
3va audit --update-cache
3va audit --json
```

| Flag | Description |
|------|-------------|
| `--secrets` | Enable phase 3: secrets detection |
| `--deny` | Exit with non-zero status on CRITICAL or HIGH findings |
| `--update-cache` | Bypass the 24-hour OSV cache and re-fetch |
| `--json` | Output results as machine-readable JSON |

---

### `3va sandbox`

Start an interactive JavaScript REPL with a sandboxed environment. Permissions can be granted dynamically inside the session.

```bash
3va sandbox
```

REPL commands available inside the sandbox:

| Command | Description |
|---------|-------------|
| `.help` | Show available REPL commands |
| `.exit` | Exit the REPL |
| `.clear` | Clear the current input buffer |
| `.allow-read <path>` | Grant read permission for a path |
| `.allow-net <host>` | Grant network permission for a host |
| `.permissions` | List currently granted permissions |

---

### `3va dev`

Start a development server with hot module replacement (HMR) via Server-Sent Events.

```bash
3va dev
3va dev --port 3000 --host 0.0.0.0
3va dev --public-dir ./static --open
```

| Flag | Description |
|------|-------------|
| `--port <N>` | Port to listen on (default: varies) |
| `--host <H>` | Host address to bind |
| `--open` | Open the browser automatically on start |
| `--public-dir <D>` | Directory to serve static files from (default: `public`) |

**HMR details:**
- File changes trigger a rebuild with a 300 ms debounce; rebuild time is printed to the console.
- The `/__hmr` endpoint is an SSE stream that browsers subscribe to.
- An HMR client script is automatically injected before `</body>` in HTML responses.
- Unknown routes fall back to `public/index.html` (SPA mode).
- Static files are served with correct MIME types.

---

### `3va doctor`

Run a system health check to verify the runtime environment.

```bash
3va doctor
```

---

## Architecture

`3va` is organized as a Cargo workspace. Each crate has a single, well-defined responsibility.

| Crate | Responsibility |
|-------|----------------|
| `vvva_core` | Tokio async event loop and scheduler |
| `vvva_cli` | `clap`-based CLI entrypoint |
| `vvva_permissions` | Capability-based deny-by-default permission engine |
| `vvva_js` | QuickJS engine via `rquickjs`; ESM loader/resolver, TypeScript transpiler, async/await, Promise microtask loop |
| `vvva_pm` | Package manager, malware scanner, secrets scanner, OSV auditor |
| `vvva_bundler` | Bundler with tree shaking, code splitting, and watch mode |
| `vvva_test` | Test runner, matchers, snapshot engine, and coverage reporting |
| `vvva_crypto` | Cryptographic utilities (post-quantum preparation) |

### JavaScript engine

`vvva_js` embeds [QuickJS](https://bellard.org/quickjs/) via `rquickjs` and provides:

- Full ESM support: `import`/`export`, named and default exports, re-export chains
- TypeScript transpilation before execution
- `async`/`await` and Promise chains driven by a pending-jobs microtask loop

---

## License

This project is licensed under the [MIT License](LICENSE).
