# 03 - Detección de Secretos

El módulo de detección de secretos escanea archivos fuente en busca de credenciales hardcodeadas: claves API, tokens de acceso, contraseñas, certificados privados y cadenas de conexión a bases de datos. Está implementado en `crates/pm/src/secrets.rs` y se activa como fase opcional del comando `3va audit`.

---

## Uso

La detección de secretos se activa con la bandera `--secrets` en el comando `audit`:

```bash
# Escaneo con salida legible para humanos
3va audit --secrets

# Salida en JSON (incluye la fase de secretos)
3va audit --json --secrets

# Combinado con --deny para fallar en vulnerabilidades OSV severas
3va audit --deny --secrets
```

El escáner analiza recursivamente el directorio de trabajo actual. Los hallazgos se imprimen en `stderr`; el proceso termina con código de salida distinto de cero **únicamente si se encuentran secretos de severidad Critical**. Los hallazgos de severidad menor (High, Medium, Low) producen una advertencia pero no interrumpen el pipeline.

### Salida legible para humanos

Cada hallazgo se reporta en el formato:

```
  [CRITICAL] src/config.js:12 — aws_access_key — const key = "AKIA...[REDACTED]...xyz"
        Fix: Store in AWS_ACCESS_KEY_ID env var or use IAM roles
```

Al finalizar el escaneo se muestra un resumen:

```
  Secrets found: 3 (1 critical, 2 high)
✗ Critical secrets detected. Remove them immediately.
```

Si no se encuentran secretos críticos pero sí de menor severidad:

```
  Secrets found: 2 (0 critical, 2 high)
! Secrets detected. Review and rotate affected credentials.
```

---

## Patrones detectados

La tabla muestra los 20 patrones registrados, en el orden en que se evalúan. Cuando múltiples patrones coinciden en la misma línea, **solo se genera un hallazgo** (el patrón de mayor prioridad, es decir el primero en la lista que coincida).

| Nombre del patrón | Severidad | Descripción |
|---|---|---|
| `aws_access_key` | Critical | Claves de acceso AWS (`AKIA[0-9A-Z]{16}`) |
| `aws_secret_key` | Critical | Claves secretas AWS en asignaciones (`aws*secret*key = "..."`, 40 chars) |
| `gcp_service_account` | Critical | JSON de cuenta de servicio GCP (`"type": "service_account"`) |
| `github_token` | Critical | Tokens de usuario GitHub (`ghp_[A-Za-z0-9]{36}`) |
| `github_oauth` | Critical | Tokens OAuth de GitHub (`gho_[A-Za-z0-9]{36}`) |
| `github_app_token` | Critical | Tokens de GitHub Apps (`ghs_[A-Za-z0-9]{36}`) |
| `gitlab_token` | Critical | Tokens de acceso personal GitLab (`glpat-[A-Za-z0-9-_]{20}`) |
| `stripe_secret_key` | Critical | Claves secretas de Stripe producción (`sk_live_[A-Za-z0-9]{24,}`) |
| `stripe_restricted_key` | High | Claves restringidas de Stripe (`rk_live_[A-Za-z0-9]{24,}`) |
| `slack_token` | High | Tokens de Slack (`xox[baprs]-[A-Za-z0-9-]{10,}`) |
| `sendgrid_api_key` | High | Claves API de SendGrid (`SG.<22+ chars>.<43+ chars>`) |
| `twilio_account_sid` | High | SID de cuentas Twilio (`AC[0-9a-fA-F]{32}`) |
| `private_key_pem` | Critical | Claves privadas PEM (RSA, EC, DSA, OpenSSH) |
| `private_key_pkcs8` | Critical | Claves privadas PKCS8 cifradas |
| `jwt` | High | JSON Web Tokens hardcodeados (3 segmentos base64url comenzando con `eyJ`) |
| `npm_token` | Critical | Tokens de publicación NPM (`npm_[A-Za-z0-9]{36}`) |
| `password_assignment` | High | Contraseñas en asignaciones de código (`password = '...'`, 8+ chars) |
| `api_key_assignment` | High | Claves API genéricas (`api_key = '...'`, 20+ chars alfanuméricos) |
| `secret_assignment` | Medium | Variables `secret` o `token` con valores literales (16+ chars) |
| `db_connection_string` | High | URIs de conexión con credenciales (mongodb, postgres, mysql, redis, amqp) |
| `sensitive_env_var` | Medium | Nombres de variables de entorno sensibles asignados literalmente en código (`AWS_SECRET_ACCESS_KEY = '...'`, etc.) |

### Severidades

| Nivel | Descripción | Comportamiento en CI |
|---|---|---|
| **Critical** | Credencial con acceso directo a sistemas de producción o infraestructura | Falla el proceso (salida ≠ 0) |
| **High** | Credencial de servicio de terceros o token con permisos elevados | Advertencia; proceso continúa |
| **Medium** | Asignación genérica sospechosa que puede contener un secreto real | Advertencia; proceso continúa |
| **Low** | Indicación débil, contexto incierto | Advertencia; proceso continúa |

---

## Archivos escaneados y excluidos

### Extensiones analizadas

El escáner solo lee archivos con las siguientes extensiones:

`.js` `.ts` `.mjs` `.cjs` `.jsx` `.tsx` `.json` `.env` `.yaml` `.yml` `.toml` `.sh` `.bash` `.zsh` `.py` `.rb` `.go` `.rs`

Cualquier otro tipo de archivo (incluyendo binarios) se omite silenciosamente.

### Directorios excluidos

El escaneo recursivo omite automáticamente los siguientes directorios:

- `.git/`
- `node_modules/`
- `dist/`
- `target/`
- `.cache/`

### Líneas de comentario excluidas

Las líneas que comienzan con `//`, `#`, `*` o `/*` (tras eliminar espacios iniciales) no se evalúan. Esto evita falsos positivos en documentación y ejemplos de código dentro de comentarios.

---

## Estructura de un hallazgo (`SecretFinding`)

```rust
pub struct SecretFinding {
    pub file: PathBuf,        // Ruta del archivo donde se encontró el secreto
    pub line: usize,          // Número de línea (base 1)
    pub secret_type: String,  // Nombre del patrón (p. ej. "aws_access_key")
    pub severity: Severity,   // Critical | High | Medium | Low
    pub snippet: String,      // Fragmento redactado de la línea
    pub suggestion: String,   // Recomendación de remediación
}
```

El campo `snippet` nunca expone el valor completo del secreto: el escáner redacta la mayor parte del contenido de la línea antes de incluirla en el hallazgo.

---

## Salida JSON (`3va audit --json --secrets`)

Cuando se usa `--json`, el objeto de salida incluye la fase `secrets` dentro de `phases`:

```json
{
  "passed": false,
  "phases": {
    "malware": {
      "clean": true
    },
    "osv": {
      "total_packages": 42,
      "packages_with_vulns": 1,
      "total_vulns": 2,
      "critical": 0,
      "high": 1,
      "findings": []
    },
    "secrets": {
      "scanned": true,
      "findings": [
        {
          "file": "src/config.js",
          "line": 12,
          "type": "aws_access_key",
          "severity": "Critical",
          "suggestion": "Store in AWS_ACCESS_KEY_ID env var or use IAM roles"
        },
        {
          "file": "src/db.js",
          "line": 3,
          "type": "db_connection_string",
          "severity": "High",
          "suggestion": "Use process.env.DATABASE_URL instead; never hardcode credentials in URIs"
        }
      ]
    }
  }
}
```

Si `--secrets` no se indica, `phases.secrets` siempre es `{ "scanned": false, "findings": [] }`.

El campo `passed` es `false` si algún hallazgo tiene severidad `"Critical"`, independientemente del resultado de las otras fases.

---

## Regla de un hallazgo por línea

Cuando múltiples patrones coinciden en la misma línea, **solo se emite un hallazgo**. El escáner evalúa los patrones en el orden de la tabla anterior y usa el primero que coincida (prioridad por posición en la lista, no por severidad). Esto evita reportes duplicados en líneas con varias señales.

---

## Remediación

La corrección canónica es mover el valor a una variable de entorno y accederla en tiempo de ejecución:

```javascript
// Incorrecto — expone la credencial en el repositorio
const stripe = new Stripe("YOUR_STRIPE_SECRET_KEY");

// Correcto — el valor solo existe en el entorno de ejecución
const stripe = new Stripe(process.env.STRIPE_SECRET_KEY);
```

Para secretos de infraestructura (claves PEM, credenciales de base de datos), considerar un gestor de secretos (AWS Secrets Manager, HashiCorp Vault, GCP Secret Manager) en lugar de variables de entorno planas.

Si una credencial ya fue expuesta en el historial de Git, **rotar la credencial de inmediato** — reescribir el historial no es suficiente si el repositorio fue clonado o accedido por terceros.
