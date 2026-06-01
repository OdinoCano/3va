# Architecture

3va is organized as a Cargo workspace. Each crate has a single, well-defined responsibility.

## Crate overview

| Crate | Responsibility |
|-------|----------------|
| `vvva_core` | Tokio async event loop and task scheduler |
| `vvva_cli` | `clap`-based CLI entrypoint |
| `vvva_permissions` | Capability-based deny-by-default permission engine |
| `vvva_js` | QuickJS engine via `rquickjs`; ESM loader, TypeScript transpiler, async/await, Promise microtask loop |
| `vvva_pm` | Package manager, malware scanner, secrets scanner, OSV auditor |
| `vvva_bundler` | Bundler with tree shaking, code splitting, and watch mode |
| `vvva_test` | Test runner, matchers, snapshot engine, and coverage reporting |
| `vvva_crypto` | Cryptographic utilities (post-quantum preparation) |

## JavaScript engine

`vvva_js` embeds [QuickJS](https://bellard.org/quickjs/) via `rquickjs` and provides:

- Full ESM support: `import`/`export`, named and default exports, re-export chains
- TypeScript transpilation before execution (no separate compile step)
- `async`/`await` and Promise chains driven by a pending-jobs microtask loop
- Built-in modules: `fs`, `net`, `http`, `crypto`, `buffer`, `child_process`, `timers`, `zlib`, `fetch`, `WebSocket`, and more

## Cryptography (`vvva_crypto`)

Built with post-quantum primitives:

- **ML-KEM-768** — Key Encapsulation Mechanism (NIST PQC standard)
- **ML-DSA** — Digital Signature Algorithm (NIST PQC standard)
- **HKDF** — Key derivation
- **Lamport signatures** — One-time signatures

The `__pqTlsConnect` global in the JS runtime establishes a classical TLS connection with an additional ML-KEM-768 key exchange on top, producing a hybrid shared secret.

## Bundler (`vvva_bundler`)

- Dependency resolution and graph construction
- Tree shaking (dead code elimination)
- Code splitting
- Minification and source map generation

## Permission engine (`vvva_permissions`)

All permission checks go through a single crate. The engine:

- Evaluates capability flags at startup
- Enforces scope restrictions (path prefix matching, host matching)
- Is consulted by every builtin before performing a sensitive operation

## Workspace layout

```
crates/
  cli/          # CLI entrypoint
  core/         # Async event loop
  js/           # JS engine + builtins
  permissions/  # Permission engine
  bundler/      # Bundler
  crypto/       # Cryptography
  pm/           # Package manager
  test/         # Test runner
  wasm/         # WASM sandbox
```
