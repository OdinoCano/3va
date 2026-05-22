# 01 - STATIC ANALYSIS

## 1.1 Code Analysis

The static analysis module examines JavaScript/TypeScript code for vulnerabilities.

## 1.2 Vulnerability Detection

| Type | Description | Severity |
|------|-------------|-----------|
| XSS | Cross-site scripting | Critical |
| SQLi | SQL injection | Critical |
| RCE | Remote code execution | Critical |
| Path Traversal | Path traversal | High |
| Command Injection | Command injection | Critical |
| XXE | XML external entity | High |
| Insecure Deserialization | Insecure deserialization | High |
| Weak Crypto | Weak cryptography | Medium |

## 1.3 Rules

```javascript
// Detected: eval() with user input
eval(userInput); // ALERT: Code injection

// Detected: innerHTML with input
element.innerHTML = userData; // ALERT: XSS

// Detected: SQL concatenation
query("SELECT * FROM " + table); // ALERT: SQLi
```

## 1.4 Integration

```bash
# Run analysis
3va analyze

# In build
3va build --security-scan

# Configuration
// 3va.config.js
module.exports = {
  security: {
    scan: true,
    rules: ["xss", "sqli", "rce"],
    severityThreshold: "medium"
  }
};
```

## 1.5 Output

```json
{
  "file": "src/user.js",
  "line": 10,
  "rule": "xss",
  "severity": "critical",
  "message": "Potential XSS: innerHTML",
  "suggestion": "Use textContent instead"
}
```

---

*Static analysis based on OWASP and ESLint security.*
