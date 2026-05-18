# 03 - CÓDIGOS DE ERROR

## 3.1 Códigos de Error

3va usa códigos de error estructurados para facilitar debugging.

## 3.2 Errores del Runtime

| Código | Descripcion |
|--------|-------------|
| ERR_RUNTIME_FAILURE | Fallo interno del runtime |
| ERR_OUT_OF_MEMORY | Memoria agotada |
| ERR_STACK_OVERFLOW | Stack overflow |
| ERR_TIMEOUT | Timeout de ejecución |

## 3.3 Errores de Permisos

| Código | Descripcion |
|--------|-------------|
| ERR_PERMISSION_DENIED | Permiso denegado |
| ERR_CAPABILITY_MISSING | Capability no otorgada |
| ERR_DENY_BY_DEFAULT | Denegado por defecto |

## 3.4 Errores de Módulos

| Código | Descripcion |
|--------|-------------|
| ERR_MODULE_NOT_FOUND | Módulo no encontrado |
| ERR_MODULE_PARSE | Error de parseo |
| ERR_REQUIRE_CYCLE | Require circular |
| ERR_INVALID_EXPORT | Export inválido |

## 3.5 Errores de Red

| Código | Descripcion |
|--------|-------------|
| ERR_HOST_NOT_ALLOWED | Host no permitido |
| ERR_DNS_RESOLVE | Error de DNS |
| ERR_CONNECTION_REFUSED | Conexión rechazada |
| ERR_TLS_ERROR | Error de TLS |

## 3.6 Errores de Sistema de Archivos

| Código | Descripcion |
|--------|-------------|
| ERR_FILE_NOT_FOUND | Archivo no encontrado |
| ERR_PERMISSION_READ | Lectura denegada |
| ERR_PERMISSION_WRITE | Escritura denegada |
| ERR_PATH_TRAVERSAL | Path traversal detectado |

## 3.7 Errores de Seguridad

| Código | Descripcion |
|--------|-------------|
| ERR_MALWARE_DETECTED | Malware detectado |
| ERR_INVALID_SIGNATURE | Firma inválida |
| ERR_SECRETS_DETECTED | Secretos en código |

## 3.8 Formato de Error

```javascript
{
  "code": "ERR_PERMISSION_DENIED",
  "message": "Permission denied: FileRead(/etc/passwd)",
  "stack": "Error at ...\n    at ...",
  "metadata": {
    "capability": "FileRead",
    "path": "/etc/passwd"
  }
}
```

---

*Error codes conforme a Node.js error system.*