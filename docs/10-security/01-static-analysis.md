# 01 - ANÁLISIS ESTÁTICO

## 1.1 Análisis de Código

El módulo de análisis estático examina código JavaScript/TypeScript en busca de vulnerabilidades.

## 1.2 Detección de Vulnerabilidades

| Tipo | Descripcion | Severidad |
|------|-------------|-----------|
| XSS | Cross-site scripting | Critical |
| SQLi | SQL injection | Critical |
| RCE | Remote code execution | Critical |
| Path Traversal | Path traversal | High |
| Command Injection | Inyección de comandos | Critical |
| XXE | XML external entity | High |
| Insecure Deserialization | Deserialización insegura | High |
| Weak Crypto | Criptografía débil | Medium |

## 1.3 Reglas

```javascript
// Detectado: eval() con entrada de usuario
eval(userInput); // ALERT: Code injection

// Detectado: innerHTML con entrada
element.innerHTML = userData; // ALERT: XSS

// Detectado: concatenación en SQL
query("SELECT * FROM " + table); // ALERT: SQLi
```

## 1.4 Integración

```bash
# Ejecutar análisis
3va analyze

# En build
3va build --security-scan

# Configuración
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

*Análisis estático basado en OWASP y ESLint security.*