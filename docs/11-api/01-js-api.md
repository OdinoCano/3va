# 01 - PUBLIC JAVASCRIPT API

## 1.1 Available APIs

The 3va runtime exposes APIs compatible with Node.js and the browser.

## 1.2 Built-in Modules

| Module | Description |
|--------|-------------|
| 3va | Runtime information |
| buffer | Data buffer |
| console | Console with auditing |
| crypto | Cryptography |
| events | EventEmitter |
| fs | File system |
| http | HTTP client |
| net | TCP/UDP sockets |
| os | System information |
| path | Path utilities |
| process | Current process |
| stream | Streams |
| url | URL parsing |
| util | Utilities |

## 1.3 3va API

```javascript
// Version information
3va.version          // "1.0.0"
3va.versions.node    // Node.js version
3va.versions.v8      // V8 version

// Runtime
3va.gc()            // Force garbage collection

// Security
3va.security.checkPermission("fs", "read", "/path")
3va.security.getAuditLog()

// Performance
3va.performance.now()
```

## 1.4 Global APIs

| API | Description |
|-----|-------------|
| fetch | HTTP requests |
| WebSocket | WebSockets |
| AbortController | Abort control |
| Performance | Measurement |
| TextEncoder/Decoder | Encoding |

---

*API compliant with Node.js and WHATWG standards.*
