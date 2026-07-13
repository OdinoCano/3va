# 04 - BACKWARD COMPATIBILITY

## 4.1 Node.js Compatibility

3va prioritizes compatibility with the Node.js ecosystem.

## 4.2 Compatible APIs

The percentages below are self-reported estimates, not measured against an
exhaustive Node.js API conformance suite — treat them as directional, not
exact. The specific factual sub-claims in the Notes column (which named
functions exist/are missing) were spot-checked against current code on
2026-07-13: `ECDH` is still accurate as "missing" in practice — `crypto.subtle.generateKey({name:'ECDH'})`
generates an EC keypair but `deriveBits`/`deriveKey` (the actual key-exchange
step) are unimplemented stubs, and there is no classic `crypto.createECDH()`;
`parseArgs`/`styleText`/`matchesGlob` are confirmed still missing.

| Module | Compatibility | Notes |
|--------|----------------|-------|
| `fs` | 98% | FD API completo, `opendir`, `mkdtemp`; `watch` real con inotify vía crate `notify` |
| `http` | 99% | Complete |
| `https` | 95% | Partial TLS |
| `net` | 95% | Partial Unix sockets |
| `crypto` | 97% | Modern algorithms; `createSign/Verify`, `generateKeyPair`, `DiffieHellman` (modp2/5/14/15/16); falta ECDH |
| `stream` | 92% | Streams2 con backpressure real (`highWaterMark`, `drain` event) |
| `process` | 99% | `memoryUsage`/`cpuUsage` reales en Linux; EventEmitter completo |
| `buffer` | 100% | Complete |
| `events` | 100% | API completa: `prependListener`, `rawListeners`, `eventNames`, `getMaxListeners` |
| `url` | 100% | Complete |
| `querystring` | 100% | Complete |
| `path` | 99% | `relative`, `normalize`, `posix`, `win32` correctos; falta `matchesGlob` real |
| `os` | 99% | `hostname`, `totalmem`, `freemem`, `uptime`, `cpus()` con model/speed/times reales, `networkInterfaces()` vía inotify/`ip addr` |
| `util` | 95% | `util.types` con 30+ métodos; faltan `parseArgs`, `styleText` |
| `zlib` | 98% | Async callbacks + sync + Transform streams reales; brotli real vía crate `brotli 7` |
| `child_process` | 95% | `exec`/`spawn`/`execSync`/`spawnSync` reales; stdin piping vía `stdin.write()`/`stdin.end()` y `spawnSync({input})` |

## 4.3 Compatibility Flags — NOT IMPLEMENTED

**None of the flags below exist in `crates/cli/src/main.rs`.** This section
was written as an aspirational placeholder and never built or removed —
`3va run --compat` / `--preset=node` are not real flags today. Left here as a
possible future feature, not a claim about current behavior.

| Flag | Description |
|------|-------------|
| --compat | Maximum compatibility mode |
| --preset=node | Simulate Node.js |

## 4.4 Automatic Polyfills

3va automatically polyfills unavailable APIs:

```javascript
// Automatic
fetch
AbortController
TextEncoder
Performance

// Requires flag
crypto (some algorithms)
```

## 4.5 Breaking Changes

**Same caveat as §4.3** — `--legacy-security` does not exist as a flag; this
table predates any real 1.0/0.9 release and doesn't correspond to an actual
migration path. For the real, current breaking-change history see
`docs/CHANGELOG.md` and `3va codemod --from=1 --to=2` (which is a real,
implemented command — see `crates/cli/src/main.rs::run_codemod`).

| Version | Change | Migration |
|---------|--------|-----------|
| 1.0 | Removed legacy API | Use new namespace |
| 0.9 | Changed default security | --legacy-security |

## 4.6 Expo / React Native Compatibility

3va runs Expo packages in a web/server context without a bundler or device.

### Supported packages

| Package | Version | Status | Notes |
|---------|---------|--------|-------|
| `expo-modules-core` | 56.0.14 | ✅ | Polyfill + real npm package loadable |
| `expo-constants` | 56.0.16 | ✅ | Web variant via `ExponentConstants.web.js` |
| `expo-asset` | 56.0.15 | ✅ | `@react-native/assets-registry` polyfilled |
| `expo-font` | 56.0.5 | ✅ | Server path (`isServer = true`); `fontfaceobserver` available |
| `expo-file-system` | 56.0.7 | ✅ | Web stubs via `ExpoFileSystem.web.ts` |
| `react-native` | 0.79.0 | ✅ | Pre-cached polyfill (Platform, NativeModules, …) |

### How it works

1. **Platform extension priority** — `.web.ts/.web.js` files are resolved before `.ts/.js` so Expo packages load their server-safe implementations automatically.
2. **`process.env.EXPO_OS = 'web'`** — Expo packages branch on this to skip `registerWebModule()` calls that require a DOM.
3. **NativeModules** — only explicitly registered modules are exposed; unknown names return `undefined` (prevents truthy-proxy issues with conditional `JSON.parse` calls).
4. **ESM→CJS converter** — handles all TypeScript/ESM patterns used by Expo (enums, destructuring exports, `export {}` markers, circular re-exports).

### Known limitations

- Native device APIs (camera, location, etc.) return stubs or throw `UnavailabilityError`.
- `requireOptionalNativeModule` always returns `null` — intended for server environments.
- `registerWebModule` returns the factory result directly (no DOM/web worker context).

## 4.7 Compatibility Testing

`3va test --compat` and `3va test-compat <pkg>` below are **not real
subcommands** — `3va test` has no `--compat` flag and there is no
`test-compat` command in `crates/cli/src/main.rs`. The last two lines are
real and are how Expo/framework compatibility is actually verified today:

```bash
# NOT IMPLEMENTED — aspirational, do not run
3va test --compat
3va test-compat express
3va test-compat lodash

# Expo / ESM→CJS integration tests (real, implemented)
cargo test -p vvva_js --test pipeline
cargo test -p vvva_js --test framework_compat
```

---

*Compatibility targeting 99% Node.js API parity + Expo web/server package support.*
