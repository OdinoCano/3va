# 05 - `3va.config.ts` Configuration File

## 5.1 Overview

v2.0.0 introduces a project-level configuration file that sets defaults for all CLI commands. CLI flags always override config file values. The file is optional — 3va continues to work without it.

---

## 5.2 File Location

3va searches for the config file in the current directory and then walks up to the filesystem root, stopping at the first match:

1. `3va.config.ts`
2. `3va.config.js`
3. `3va.config.json`

The first file found is used. Multiple files at different directory levels are not merged.

---

## 5.3 Schema

```ts
// 3va.config.ts
import type { Config } from '3va/config';

export default {
  // Defaults for `3va run`
  run: {
    permissions: {
      net: ['api.example.com'],          // equivalent to --allow-net=api.example.com
      read: ['/app/data', '/app/config'],
      write: ['/tmp'],
      env: ['HOME', 'NODE_ENV'],
      childProcess: false,               // equivalent to --allow-child-process
      ffi: [],                           // equivalent to --allow-ffi=<path>
    },
    inspect: false,       // default inspect address when --inspect is passed bare
    port: 8080,           // default port for --port (sets process.env.PORT)
  },

  // Defaults for `3va dev`
  dev: {
    port: 3000,
    host: '0.0.0.0',
    publicDir: './public',
    open: false,
  },

  // Defaults for `3va test`
  test: {
    paths: ['tests/', 'src/'],
    watch: false,
    coverage: false,
    updateSnapshots: false,
  },

  // Defaults for `3va audit`
  audit: {
    deny: true,
    secrets: false,
    updateCache: false,
  },

  // Defaults for `3va bundle`
  bundle: {
    outDir: './dist',
    minify: false,
    sourceMap: true,
    split: false,
  },

  // Workspace settings
  workspace: {
    hoisting: true,
    parallelism: 4,      // max concurrent packages during workspace run
  },

  // HTTP server firewall (applied to all http.createServer() instances)
  firewall: {
    enabled: true,
    rateLimitRps: 100,           // max sustained req/s per IP
    rateLimitBurst: 200,         // burst capacity before throttling
    autoBlockThreshold: 10,      // violations before auto-block
    blockDurationSecs: 300,      // block duration in seconds
    maxConnectionsPerIp: 50,     // simultaneous connections per IP
    maxConnectionsTotal: 10_000, // total simultaneous connections
    headerTimeoutMs: 10_000,     // Slowloris protection: header read deadline
    bodyTimeoutMs: 30_000,       // RUDY protection: body read deadline
    maxHeaderCount: 100,         // max HTTP headers per request
    maxHeaderBytes: 16_384,      // max combined header size (bytes)
    maxBodyBytes: 0,             // 0 = internal 100 MiB cap
  },
} satisfies Config;
```

---

## 5.4 Loading Order

For each invocation:

1. Locate config file (directory walk as above).
2. If `.ts`, transpile via the existing TypeScript transpiler (`vvva_js::transpiler`).
3. Evaluate the default export.
4. Merge config values as defaults into the CLI flag parser.
5. CLI flags override all config values.

### 5.4.1 Security Sandbox

To prevent arbitrary code execution attacks (e.g. from malicious repositories or dependencies), the config file is loaded and evaluated inside a zero-privilege sandboxed JavaScript context:
- No network access (`--allow-net` is ignored/denied during evaluation).
- No filesystem write access.
- Filesystem read access is restricted strictly to the config file path itself (no directory traversal or secret reading allowed).
- No child process spawning (`child_process` and FFI are disabled).

---

## 5.5 Environment Variable Overrides

Config file values can be overridden with environment variables using the pattern `3VA_<SECTION>_<KEY>` (uppercase, underscores for camelCase):

| Variable | Overrides | Type |
|----------|-----------|------|
| `3VA_RUN_PORT` | `config.run.port` | `u16` |
| `3VA_DEV_PORT` | `config.dev.port` | `u16` |
| `3VA_DEV_HOST` | `config.dev.host` | string |
| `3VA_DEV_PUBLIC_DIR` | `config.dev.public_dir` | string |
| `3VA_DEV_OPEN` | `config.dev.open` | bool |
| `3VA_DEV_CSP` | `config.dev.csp.enabled` | bool |
| `3VA_TEST_COVERAGE` | `config.test.coverage` | bool |
| `3VA_TEST_WATCH` | `config.test.watch` | bool |
| `3VA_TEST_UPDATE_SNAPSHOTS` | `config.test.update_snapshots` | bool |
| `3VA_TEST_CONCURRENCY` | `config.test.concurrency` | `usize` |
| `3VA_AUDIT_DENY` | `config.audit.deny` | bool |
| `3VA_AUDIT_SECRETS` | `config.audit.secrets` | bool |
| `3VA_AUDIT_UPDATE_CACHE` | `config.audit.update_cache` | bool |
| `3VA_BUNDLE_OUT_DIR` | `config.bundle.out_dir` | string |
| `3VA_BUNDLE_MINIFY` | `config.bundle.minify` | bool |
| `3VA_BUNDLE_SOURCE_MAP` | `config.bundle.source_map` | bool |
| `3VA_BUNDLE_SPLIT` | `config.bundle.split` | bool |
| `3VA_WORKSPACE_HOISTING` | `config.workspace.hoisting` | bool |
| `3VA_WORKSPACE_PARALLELISM` | `config.workspace.parallelism` | `usize` |

Bool values accept: `1`, `true`, `yes`, `on` (case-insensitive).

```bash
3VA_RUN_PORT=8080 3va run app.ts   # overrides config.run.port
3VA_DEV_PORT=8080 3va dev          # overrides config.dev.port
3VA_TEST_CONCURRENCY=8 3va test    # overrides config.test.concurrency
3VA_BUNDLE_MINIFY=true 3va bundle  # overrides config.bundle.minify
```

Priority: CLI flags > environment variables > config file > built-in defaults.

---

## 5.6 `3va config` Subcommand

```bash
# Show the resolved config (merged from file + env vars + defaults)
3va config

# Show a specific key
3va config dev.port

# Validate the config file without running any command
3va config --check
```
