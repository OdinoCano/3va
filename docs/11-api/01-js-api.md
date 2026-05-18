# 01 - API JAVASCRIPT PÚBLICA

## 1.1 APIs Disponibles

El runtime 3va expone APIs compatibles con Node.js y navegador.

## 1.2 Módulos Integrados

| Módulo | Descripcion |
|--------|-------------|
| 3va | Información del runtime |
| buffer | Buffer de datos |
| console | Consola con auditoría |
| crypto | Criptografía |
| events | EventEmitter |
| fs | Sistema de archivos |
| http | Cliente HTTP |
| net | TCP/UDP sockets |
| os | Información del sistema |
| path | Utilidades de rutas |
| process | Proceso actual |
| stream | Streams |
| url | Parseo de URLs |
| util | Utilidades |

## 1.3 3va API

```javascript
// Información de versión
3va.version          // "1.0.0"
3va.versions.node    // Versión de Node.js
3va.versions.v8      // Versión de V8

// Runtime
3va.gc()            // Force garbage collection

// Security
3va.security.checkPermission("fs", "read", "/path")
3va.security.getAuditLog()

// Performance
3va.performance.now()
```

## 1.4 Global APIs

| API | Descripcion |
|-----|-------------|
| fetch | HTTP requests |
| WebSocket | WebSockets |
| AbortController | Control de abortos |
| Performance | Medición |
| TextEncoder/Decoder | Codificación |

---

*API conforme a Node.js y WHATWG standards.*