# 05 - 3VA v2.0.0 ROADMAP

**Released:** 2026-06-10 · **Base:** v1.0.0 LTS (2026-06-01)

---

## 2.1 Vision for v2.0.0

v1.0.0 established the secure-by-default runtime foundation: permissions, post-quantum crypto, CDP inspector, NAPI, WASM, and a full toolchain (PM + bundler + test runner + dev server). v2.0.0 closes the remaining performance observability gap and deepens Node.js compatibility to the point where the majority of the npm ecosystem works without modification.

---

## 2.2 Feature Goals

### 2.2.1 Performance Profiling and Flamegraph (`--prof`) — ✅ Implemented in v1.x

**Motivation:** There is no way to identify CPU hot paths or memory allocations in running 3va scripts today.

**Scope:**

| Feature | Detail |
|---------|--------|
| CPU sampling profiler | Sampling-based profiler emitting V8-compatible `.cpuprofile` JSON |
| Flamegraph export | `--prof-out flamegraph.svg` — Inferno-style SVG output |
| Memory snapshot | `--heap-snapshot` — planned; not yet implemented in v2.0.0 |
| `console.profile()` / `console.profileEnd()` | JS-side profiler start/stop markers |
| `3va prof <file>` subcommand | Post-hoc analysis: load a `.cpuprofile` and print top-N hot functions |

**CLI:**

```bash
3va run app.ts --prof --prof-out=profile.cpuprofile
3va prof profile.cpuprofile --top 20
```

---

### 2.2.2 Node.js Compatibility Layer v2

**Motivation:** Several popular npm packages fail because they use Node.js APIs not yet implemented in v1.0.0. The target for v2.0.0 is compatibility sufficient to run Express, Fastify, Koa, and their common middleware ecosystem without shimming.

**New modules / improvements:**

| Module | v1.0.0 state | v2.0.0 target |
|--------|-------------|---------------|
| `cluster` | ✅ Single-process emulation: `isPrimary: true`, `fork()` returns mock workers | |
| `worker_threads` | Not implemented | `Worker`, `parentPort`, `workerData`, `MessageChannel` |
| `dgram` | Not implemented | UDP send/receive |
| `dns` | Stub only | `dns.resolve`, `dns.lookup`, `dns.promises.*` |
| `readline` | Partial | Full `Interface`, `createInterface`, async iterator |
| `node:crypto` `webcrypto` | Missing | `globalThis.crypto` (SubtleCrypto) backed by existing `vvva_crypto` |
| `events` `EventEmitter` | Implemented | Add `EventEmitter.once`, `EventEmitter.on` static helpers (Node 16+) |
| `stream/web` | Not implemented | `ReadableStream`, `WritableStream`, `TransformStream` (WHATWG Streams) |
| `timers/promises` | Not implemented | `setTimeout`, `setInterval`, `setImmediate` as promises |

---

### 2.2.3 REPL Plugins

**Motivation:** The sandbox REPL has no extension mechanism. Power users and tooling authors need a way to add custom commands, syntax highlighting, or auto-completion.

**Scope:**

- Plugin API: `3va sandbox --plugin ./my-plugin.ts`
- Plugin interface:
  ```ts
  export interface ReplPlugin {
    commands?: Record<string, (args: string, repl: ReplContext) => void>;
    completers?: Array<(line: string) => string[]>;
    banner?: string;
  }
  ```
- Built-in plugins bundled with 3va:
  - `inspect` — pretty-print objects with color
  - `history` — persistent REPL history saved to `~/.3va/repl_history`

---

### 2.2.4 Workspace v2 — ✅ Implemented

**Motivation:** The current workspace implementation (`3va workspace`) covers basic install and list operations but lacks the dependency graph and script execution coordination needed for monorepos.

**Scope:**

| Feature | Detail | Status |
|---------|--------|--------|
| Topological script execution | `3va workspace run build` — runs `build` in dependency order | ✅ `workspace_v2.rs::topological_order` (Kahn's algorithm) |
| Affected-only mode | `--affected` — detects changed packages via `git diff` and runs scripts only in affected packages and their dependents | ✅ `workspace_v2.rs::affected_packages` |
| Workspace graph | `3va workspace graph` — prints the inter-package dependency graph as ASCII or DOT | ✅ `workspace_v2.rs` |
| Shared dependency hoisting | Deduplicate compatible versions into a workspace-root `node_modules` | ✅ `workspace::merged_deps` picks the highest compatible version per name across all packages; `install_workspace` (`crates/pm/src/lib.rs:2100`) installs the deduplicated set once into the workspace root |
| Per-package permission overrides | Each package can declare its own permission scopes in `package.json#3va.permissions` | ✅ **Isolated for real** (2026-07-13). Previously cosmetic — every scope merged into one flat, process-wide grant set (`ponytail:` comment in `read_package_json_permissions` admitted this). Fixed without V8 stack introspection: `vvva_permissions::scope` tracks "which package's code is executing" in a thread-local, set by the `require()` wrapper in `crates/js/src/builtins/modules.rs` right before handing a capability-gated builtin (`fs`, `fs/promises`, `net`, `tls`, `dgram`, `child_process`) to the requesting module — derived from the requesting module's own `node_modules/<pkg>` path, no code-execution-flow changes needed at any of the ~40 `perms().check(...)` call sites in `fs.rs`/`tcp.rs`/etc. `PermissionState` (`crates/permissions/src/capability.rs`) now stores `scoped_granted`/`scoped_denied` maps alongside the existing global sets; `check()` consults both automatically. `crates/cli/src/main.rs`'s `read_package_json_permissions`/`build_permissions` route non-`"."` scopes through `grant_scoped`/`deny_scoped` instead of merging them. **Known residual gap:** long-lived class instances obtained by requiring the class directly (`new require('net').Socket()` instead of calling `net.connect(...)`) bypass the wrapper, since wrapping a constructor with a plain closure would break `instanceof` — the common functional API (`net.connect`, `createServer`, `dgram.createSocket`, `child_process.exec/spawn`) is covered, prototype methods on manually-constructed instances are not. `fs.createReadStream`/`createWriteStream`'s deferred native I/O (fires on a later tick, after the synchronous wrapper call already reverted) is explicitly patched to re-capture and re-apply the creator's scope — see the `__streamScope` comments in `fs.rs`. Verified with a real running binary: a scoped-grant dependency can read/connect where the app's own unscoped code is denied the identical capability. |

---

### 2.2.5 Security Hardening

**Motivation:** v1.0.0 accepted RUSTSEC-2023-0071 (RSA Marvin Attack) with an explicit rationale. v2.0.0 must resolve this and raise the security baseline.

**Scope:**

| Item | Detail |
|------|--------|
| Replace RSA with constant-time implementation | Migrate from `rsa 0.9` to `rsa 0.10+` when RUSTSEC-2023-0071 is patched upstream, or to a FIPS-validated alternative |
| Sandbox isolation audit | Formal review of the `vvva_permissions` enforcement layer against WASI capability boundaries |
| Supply-chain: SLSA level 2 | Sign release binaries with `cosign`; publish provenance attestations to GitHub Releases |
| Dependency audit automation | Weekly scheduled `cargo audit` + `cargo deny check` via GitHub Actions; auto-open issues on new advisories |
| Content Security Policy for `dev` server | Inject default CSP header in dev server responses; configurable via `3va.config.ts` |

---

### 2.2.6 `3va.config.ts` Configuration File

**Motivation:** Several flags (dev server port, permission defaults, test paths) are repeated in every invocation. A project-level config file eliminates the repetition.

**Scope:**

```ts
// 3va.config.ts
export default {
  run: {
    permissions: {
      net: ['api.example.com'],
      read: ['/app/data'],
    },
  },
  dev: {
    port: 3000,
    host: '0.0.0.0',
    publicDir: './static',
  },
  test: {
    paths: ['tests/'],
    coverage: true,
  },
  audit: {
    deny: true,
    secrets: true,
  },
};
```

The config file is loaded by the CLI before flag parsing. CLI flags override config file values.

---

## 2.3 Non-Goals for v2.0.0

The following items are explicitly **out of scope** for v2.0.0:

- **Multi-threaded JS execution** (shared-memory parallelism) — deferred to v3.0.0
- **V8 / SpiderMonkey engine swap** — V8 remains the JS engine
- **GUI debugger** — CDP support (v1.0.0) is the debugger surface; a GUI is a third-party concern
- **Paid / cloud features** — 3va remains fully open source and self-hosted

---

## 2.4 Milestones

| Milestone | Target | Content |
|-----------|--------|---------|
| v2.0.0-alpha.1 | Q1 2027 | `--prof` CPU profiler + `3va.config.ts` skeleton |
| v2.0.0-alpha.2 | Q2 2027 | `worker_threads` + `dgram` + `webcrypto` global |
| v2.0.0-beta.1 | Q3 2027 | Workspace v2 topological execution + affected mode |
| v2.0.0-beta.2 | Q3 2027 | REPL plugins + persistent history |
| v2.0.0-rc.1 | Q4 2027 | Security hardening: SLSA + RSA fix + CSP |
| v2.0.0 stable | Q4 2027 | All features complete; API frozen |

---

## 2.5 Breaking Changes

v2.0.0 will be a **compatible** major release. The only planned breaking changes are:

| Change | Migration |
|--------|-----------|
| `require('crypto').pq` API aligned with Web Crypto naming | Automated codemod provided (`3va codemod --from=1 --to=2`) |
| `3va workspace` sub-subcommands renamed for consistency | Old names kept as deprecated aliases until v3.0.0 |

---

*Roadmap subject to change. Priorities are revisited each quarter based on community feedback.*
