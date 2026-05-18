# 04 - FORMATO DE LOCKFILE

## 4.1 Estructura del Lockfile

El lockfile de 3va garantiza instalaciones reproducibles y auditorías de seguridad.

## 4.2 Formato JSON

```json
{
  "lockfileVersion": 3,
  "name": "my-project",
  "version": "1.0.0",
  "requires": true,
  "packages": {
    "": {
      "name": "my-project",
      "version": "1.0.0",
      "dependencies": {
        "lodash": "^4.17.21"
      }
    },
    "node_modules/lodash": {
      "version": "4.17.21",
      "resolved": "https://registry.npmjs.org/lodash/-/lodash-4.17.21.tgz",
      "integrity": "sha512-v2kDEe57lecTulaDIuNPH8kBW8ZjxJZI4E3L7I4Bi0S6MjsZ4zT9BtC2N3bQ6ZFW3Vfrq9XtC5Q=="
    }
  },
  "dependencies": {
    "lodash": {
      "version": "4.17.21",
      "resolved": "https://registry.npmjs.org/lodash/-/lodash-4.17.21.tgz",
      "integrity": "sha512-v2kDEe57lcTulaDIuNPH8kBW8ZjxJZI4E3L7I4Bi0S6MjsZ4zT9BtC2N3bQ6ZFW3Vfrq9XtC5Q==",
      "dependencies": {}
    }
  },
  "3va": {
    "audit": {
      "lastChecked": "2026-05-18T10:00:00Z"
    },
    "security": {
      "verified": true,
      "signatures": ["..."]
    }
  }
}
```

## 4.3 Campos

| Campo | Tipo | Descripcion |
|-------|------|-------------|
| lockfileVersion | number | Version del formato (3) |
| name | string | Nombre del proyecto |
| version | string | Version del proyecto |
| packages | object | Mapa de paquetes instalados |
| dependencies | object | Dependencias resueltas |
| 3va | object | Metadatos de 3va |

## 4.4 Entrada de Paquete

```json
"node_modules/lodash": {
  "version": "4.17.21",
  "resolved": "https://registry.npmjs.org/lodash/-/lodash-4.17.21.tgz",
  "integrity": "sha512-v2kDEe57lcTulaDIuNPH8kBW8ZjxJZI4E3L7I4Bi0S6MjsZ4zT9BtC2N3bQ6ZFW3Vfrq9XtC5Q==",
  "dependencies": {},
  "devDependencies": {},
  "optionalDependencies": {},
  "engines": {},
  "bin": {}
}
```

## 4.5 Integridad y Seguridad

### 4.5.1 Hash de Integridad

```
integrity: sha384-<SHA384-hash-base64>
```

### 4.5.2 Metadatos de Seguridad

```json
"3va": {
  "security": {
    "verified": true,
    "signatures": [
      {
        "keyid": "...",
        "signature": "..."
      }
    ],
    "malwareScan": {
      "scannedAt": "2026-05-18T10:00:00Z",
      "result": "clean"
    }
  }
}
```

## 4.6 Operaciones

### 4.6.1 Generar Lockfile

```bash
3va install
# Genera o actualiza 3va-lock.json
```

### 4.6.2 Instalar desde Lockfile

```bash
3va install --frozen-lockfile
# Falla si lockfile no coincide con package.json
```

### 4.6.3 Actualizar Lockfile

```bash
3va install --update
# Actualiza lockfile a últimas versiones permitidas
```

---

*Lockfile conforme a npm lockfile spec con extensiones de seguridad.*