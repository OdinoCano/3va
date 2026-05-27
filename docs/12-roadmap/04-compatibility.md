# 04 - BACKWARD COMPATIBILITY

## 4.1 Node.js Compatibility

3va prioritizes compatibility with the Node.js ecosystem.

## 4.2 Compatible APIs

| Module | Compatibility | Notes |
|--------|----------------|-------|
| `fs` | 97% | FD API completo (`open/close/read/write/fstat`), `opendir`, `mkdtemp`; falta `watch` real con inotify |
| `http` | 99% | Complete |
| `https` | 95% | Partial TLS |
| `net` | 95% | Partial Unix sockets |
| `crypto` | 90% | Modern algorithms; faltan `createSign/Verify`, `generateKeyPair`, `DiffieHellman` |
| `stream` | 85% | Streams2 partial; falta backpressure real (`highWaterMark`) |
| `process` | 99% | `memoryUsage`/`cpuUsage` reales en Linux; EventEmitter completo |
| `buffer` | 100% | Complete |
| `events` | 100% | API completa: `prependListener`, `rawListeners`, `eventNames`, `getMaxListeners` |
| `url` | 100% | Complete |
| `querystring` | 100% | Complete |
| `path` | 99% | `relative`, `normalize`, `posix`, `win32` correctos; falta `matchesGlob` real |
| `os` | 95% | `hostname`, `totalmem`, `freemem`, `uptime` reales en Linux; faltan `cpus()` con datos reales, `networkInterfaces()` |
| `util` | 95% | `util.types` con 30+ métodos; faltan `parseArgs`, `styleText` |
| `zlib` | 95% | Async callbacks + sync + Transform streams reales; brotli usa gzip como fallback |
| `child_process` | 85% | `exec`/`spawn` reales; faltan `execSync`/`spawnSync`, stdin piping |

## 4.3 Compatibility Flags

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

| Version | Change | Migration |
|---------|--------|-----------|
| 1.0 | Removed legacy API | Use new namespace |
| 0.9 | Changed default security | --legacy-security |

## 4.6 Compatibility Testing

```bash
# Compatibility test suite
3va test --compat

# Run npm test of packages
3va test-compat express
3va test-compat lodash
```

---

*Compatibility targeting 99% Node.js API parity.*
