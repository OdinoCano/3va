# 03 - DETECCIÓN DE SECRETOS

## 3.1 Secrets Detection

Detecta claves API, tokens, contraseñas y otros secretos en el código.

## 3.2 Tipos de Secretos

| Tipo | Pattern |
|------|---------|
| AWS Key | AKIA[0-9A-Z]{16} |
| GitHub Token | ghp_[a-zA-Z0-9]{36} |
| Private Key | -----BEGIN PRIVATE KEY----- |
| JWT | eyJ[a-zA-Z0-9_-]*\.eyJ[a-zA-Z0-9_-]* |
| Password | password\s*=\s*["\'][^"\']{8,}["\'] |
| API Key | api[_-]?key\s*=\s*["\'][a-zA-Z0-9]{20,}["\'] |

## 3.3 Entornos Detectados

| Entorno | Variables |
|---------|-----------|
| AWS | AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY |
| GitHub | GITHUB_TOKEN, GH_TOKEN |
| GitLab | GITLAB_TOKEN |
| NPM | NPM_TOKEN |
| Stripe | STRIPE_SECRET_KEY |
| SendGrid | SENDGRID_API_KEY |

## 3.4 Uso

```bash
# Scan de secretos
3va scan-secrets

# En build
3va build --scan-secrets

# Hook pre-commit
3va hook install --secret-scan
```

## 3.5 Output

```json
{
  "file": "config.js",
  "line": 5,
  "type": "aws_key",
  "severity": "critical",
  "suggestion": "Usar variable de entorno"
}
```

## 3.6 Remediación

```javascript
// Malo
const apiKey = "ghp_xxxxxxxxxxxxxxx";

// Bueno
const apiKey = process.env.GITHUB_TOKEN;
```

---

*Secrets detection basado en truffleHog y git-secrets.*