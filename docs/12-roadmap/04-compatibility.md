# 04 - COMPATIBILIDAD RETROACTIVA

## 4.1 Compatibilidad con Node.js

3va prioriza compatibilidad con el ecosistema Node.js.

## 4.2 APIs Compatibles

| Módulo | Compatibilidad | Notas |
|--------|----------------|-------|
| fs | 95% | Sin soporte para algunos edge cases |
| http | 99% | Completo |
| https | 95% | TLS parcial |
| net | 95% | Unix sockets parcial |
| crypto | 90% | Algoritmos modernos |
| stream | 90% | Streams2 partial |
| process | 99% | Completo |
| buffer | 100% | Completo |
| events | 100% | Completo |
| url | 100% | Completo |
| querystring | 100% | Completo |
| path | 100% | Completo |
| os | 100% | Completo |
| util | 95% | Completo |

## 4.3 Flags de Compatibilidad

| Flag | Descripcion |
|------|-------------|
| --compat | Modo compatibilidad máxima |
| --preset=node | Simular Node.js |

## 4.4 Polyfills Automáticos

3va polyfill automáticamente APIs no disponibles:

```javascript
// Automático
fetch
AbortController
TextEncoder
Performance

// Requiere flag
crypto (some algorithms)
```

## 4.5 Breaking Changes

| Version | Cambio | Migration |
|---------|--------|-----------|
| 1.0 | Removed legacy API | Usar nuevo namespace |
| 0.9 | Changed default security | --legacy-security |

## 4.6 Testing de Compatibilidad

```bash
# Test suite de compatibilidad
3va test --compat

# Run npm test de paquetes
3va test-compat express
3va test-compat lodash
```

---

*Compatibilidad targeting 99% Node.js API parity.*