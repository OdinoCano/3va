# CLI Reference

## Global flags

Global flags are placed immediately after `3va`, before the subcommand. Both `3va --flag cmd` and `3va cmd --flag` are accepted.

### `--verbose` / `-v`

Show runtime status messages (hidden by default when running scripts).

```bash
3va run app.ts          # only script output — no runtime noise
3va -v run app.ts       # also shows: Running …, initialized, Execution finished
```

### `--accessible`

Enable accessible / screen reader / Braille mode. Must come immediately after `3va`, before the subcommand.

- Disables all ANSI colors and escape sequences
- Removes animations, spinners, and progress bars
- Produces plain line-by-line text suitable for screen readers
- Complies with EN 301 549

```bash
3va --accessible run app.ts
3va --accessible audit --json
```

---

## Command aliases

| Full | Short |
|------|-------|
| `3va run` | `3va r` |
| `3va install` | `3va i`, `3va add` |
| `3va test` | `3va t`, `3va spec` |
| `3va dev` | `3va d` |
| `3va bundle` | `3va b` |
| `3va workspace` | `3va ws` |
| `3va sandbox` | `3va sh`, `3va shell` |

---

## `3va run <file>`

Run a JavaScript or TypeScript file inside a sandboxed environment.

```bash
3va run app.ts
3va run app.ts --allow-net=api.example.com --allow-read=/data --allow-env=HOME
```

| Flag | Short | Description |
|------|-------|-------------|
| `--allow-read[=<path>]` | | Grant read access (optionally scoped to a path) |
| `--allow-write[=<path>]` | | Grant write access (optionally scoped to a path) |
| `--allow-net[=<host>]` | | Grant network access (optionally scoped to a host) |
| `--allow-env[=<var>]` | | Grant environment variable access (optionally scoped) |
| `--allow-child-process` | | Allow spawning child processes |
| `--allow-ffi[=<path>]` | | Allow loading native `.node` addons (NAPI) |
| `--inspect[=<host:port>]` | `-I` | Enable Chrome DevTools Protocol debugger (default: `127.0.0.1:9229`) |
| `--audit-log=<path>` | | Write a JSON audit log of permission checks after execution |
| `--audit-level=<level>` | | `deny` (default): log only denied checks; `all`: log every check |
| `--prof` | | Enable CPU sampling profiler |
| `--prof-out=<path>` | | Output path for the CPU profile (default: `profile.cpuprofile`) |
| `--prof-interval=<ms>` | | Sampling interval in milliseconds (default: `10`) |
| `--flamegraph=<path>` | | Also emit an Inferno-style SVG flamegraph |

### Debugging with `--inspect`

```bash
3va run app.ts --inspect          # or: 3va run app.ts -I
3va run app.ts --inspect=0.0.0.0:9230
```

Open `chrome://inspect` in Chrome and click *Open dedicated DevTools for Node*.

### CPU Profiling

```bash
3va run app.ts --prof
3va run app.ts --prof --prof-out=my.cpuprofile --flamegraph=flame.svg
```

Use `console.profile` / `console.profileEnd` to annotate regions in your script.

### Post-quantum TLS

```js
const { connId, pqSharedSecret } = await __pqTlsConnect('example.com', 443);
```

`__pqTlsConnect` is a global injected by the runtime. Requires `--allow-net=<host>`. Returns a hybrid shared secret derived via ML-KEM-768.

---

## `3va prof <file>`

Analyze a `.cpuprofile` file and print a top-N function breakdown.

```bash
3va prof profile.cpuprofile
3va prof profile.cpuprofile --top 10
3va prof profile.cpuprofile --format=flamegraph --out=flame.svg
```

| Flag | Default | Description |
|------|---------|-------------|
| `--top <N>` | `20` | Number of hot functions to show |
| `--format <fmt>` | `text` | Output format: `text` or `flamegraph` |
| `--out <path>` | `flamegraph.svg` | SVG output path |

---

## Package Management

### `3va install <package>[@version]`

```bash
3va install axios --allow-net=registry.npmjs.org
3va install react@18 --allow-net=registry.yarnpkg.com
3va install @std/path --allow-net=jsr.io
```

| Registry | Host |
|----------|------|
| npm | `registry.npmjs.org` |
| Yarn | `registry.yarnpkg.com` |
| JSR | `jsr.io` |

Post-install scripts are **never** executed.

### `3va reinstall <package>[@version]`

Force-reinstall a single package, even if already installed.

```bash
3va reinstall axios --allow-net=registry.npmjs.org
```

### `3va update [packages...]`

Update installed packages (all, or the ones listed) to their latest versions, preserving each package's original registry.

```bash
3va update --allow-net=registry.npmjs.org,jsr.io
```

---

## `3va bundle <input>`

Bundle a JS/TS application into a single output file.

```bash
3va bundle src/index.ts
3va bundle src/index.ts -o dist/bundle.js --minify --source-map
3va bundle src/index.ts --split
```

| Flag | Description |
|------|-------------|
| `-o <path>` | Output file path |
| `--split` | Enable code splitting |
| `--minify` | Minify output |
| `--source-map` | Emit a source map |

---

## `3va test [paths...]`

Run tests using the built-in Jest-compatible test runner.

```bash
3va test
3va test tests/unit
3va test --watch
3va test --coverage
3va test --update-snapshots
```

Supports `describe`, `test`, `expect`, all common matchers, and snapshots.

---

## `3va audit`

Audit installed packages in three phases:

1. **Malware scan** — static analysis of `node_modules`
2. **OSV CVE scan** — queries [api.osv.dev](https://api.osv.dev/v1/querybatch) (24-hour local cache)
3. **Secrets detection** — scans for leaked credentials (opt-in)

```bash
3va audit
3va audit --secrets
3va audit --deny
3va audit --json
```

| Flag | Description |
|------|-------------|
| `--secrets` | Enable secrets detection (scans the current project's source files) |
| `--deny` | Exit non-zero on CRITICAL or HIGH OSV vulnerabilities (secrets fail only on Critical) |
| `--update-cache` | Bypass the 24-hour OSV cache |
| `--json` | Machine-readable JSON output |

---

## `3va sandbox`

Start an interactive JavaScript REPL with a sandboxed environment.

```bash
3va sandbox
```

| REPL command | Description |
|--------------|-------------|
| `.help` | Show available commands |
| `.permissions` | List currently granted permissions |
| `.allow-read=PATH` | Grant read permission |
| `.allow-write=PATH` | Grant write permission |
| `.allow-net=HOST` | Grant network permission |
| `.allow-env` | Grant environment variable access |
| `.clear` | Reset the JS context |
| `exit` / `quit` / `^D` | Exit the REPL (no leading dot) |

---

## `3va dev`

Start a development server with hot module replacement (HMR).

Automatically detects Astro, Next.js, Nuxt, SvelteKit, Remix, Gatsby, SolidStart, and Qwik.

```bash
3va dev
3va dev --port 3000 --host 0.0.0.0 --open
```

| Flag | Description |
|------|-------------|
| `--port <N>` | Port to listen on |
| `--host <H>` | Host address to bind |
| `--open` | Open the browser on start |
| `--public-dir <D>` | Static files directory (default: `public`) |
| `--no-csp` | Disable the Content-Security-Policy header |

---

## Process Manager

| Command | Description |
|---------|-------------|
| `3va start <file>` | Start a managed background daemon |
| `3va stop <name>` | Stop a managed process (SIGTERM → SIGKILL after 1.5s) |
| `3va restart <name>` | Restart a managed process |
| `3va status [name]` | Show status of one or all processes |
| `3va logs <name>` | Show logs (`--lines <N>`, default: 50) |
| `3va delete <name>` | Stop and permanently remove a process |

```bash
3va start app.js --name my-api
3va status
3va logs my-api --lines 200
3va stop my-api
```

---

## `3va doctor`

Run a system health check to verify the runtime environment.

```bash
3va doctor
```
