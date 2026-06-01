# 04 - BACKWARD COMPATIBILITY

## 4.1 Node.js Compatibility

3va prioritizes compatibility with the Node.js ecosystem.

## 4.2 Compatible APIs

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
| `zlib` | 95% | Async callbacks + sync + Transform streams reales; brotli usa gzip como fallback |
| `child_process` | 95% | `exec`/`spawn`/`execSync`/`spawnSync` reales; stdin piping vía `stdin.write()`/`stdin.end()` y `spawnSync({input})` |

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
