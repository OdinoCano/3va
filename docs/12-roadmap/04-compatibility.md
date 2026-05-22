# 04 - BACKWARD COMPATIBILITY

## 4.1 Node.js Compatibility

3va prioritizes compatibility with the Node.js ecosystem.

## 4.2 Compatible APIs

| Module | Compatibility | Notes |
|--------|----------------|-------|
| fs | 95% | No support for some edge cases |
| http | 99% | Complete |
| https | 95% | Partial TLS |
| net | 95% | Partial Unix sockets |
| crypto | 90% | Modern algorithms |
| stream | 90% | Streams2 partial |
| process | 99% | Complete |
| buffer | 100% | Complete |
| events | 100% | Complete |
| url | 100% | Complete |
| querystring | 100% | Complete |
| path | 100% | Complete |
| os | 100% | Complete |
| util | 95% | Complete |

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
