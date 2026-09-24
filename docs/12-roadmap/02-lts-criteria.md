# 02 - LTS CRITERIA

## 2.1 LTS Definition

Long Term Support means extended support and guaranteed stability.

## 2.2 Requirements for LTS

### 2.2.1 Stability

| Criterion | Target |
|----------|--------|
| Uptime | 99.9% |
| Crash rate | < 0.1% |
| Memory leaks | 0 detectable |
| API stability | 100% backward compatible |

### 2.2.2 Compatibility

| Criterion | Target |
|----------|--------|
| Node.js API | 99.9% |
| npm packages | 95% |
| ESM/CJS | 100% |
| TypeScript | 100% |

### 2.2.3 Security

| Criterion | Target |
|----------|--------|
| Critical CVEs | 0 |
| High CVEs | < 5 |
| Security audit | Passed |
| Penetration test | Passed |

### 2.2.4 Performance

| Criterion | Target |
|----------|--------|
| Cold start | < 100ms |
| Throughput | Comparable to Bun |
| Memory usage | < 50MB base |
| Bundle time | < 2s |

## 2.3 LTS Support

There is currently **no LTS release line**. Only the latest minor release is
supported; see [SECURITY.md § Supported Versions](../../SECURITY.md#supported-versions)
for the authoritative policy. The criteria in § 2.2 are what a future LTS line
would have to meet before one is announced.

## 2.4 Quality Process

```
Code Review → Tests → Security Scan → Benchmark → Release
```

### Release Gate

- [ ] All tests pass
- [ ] Coverage > 80%
- [ ] No critical vulnerabilities
- [ ] Stable benchmark
- [ ] Updated documentation

---

*LTS compliant with Node.js and enterprise standards.*
