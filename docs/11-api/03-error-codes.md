# 03 - ERROR CODES

## 3.1 Error Codes

3va uses structured error codes to facilitate debugging.

## 3.2 Runtime Errors

| Code | Description |
|--------|-------------|
| ERR_RUNTIME_FAILURE | Internal runtime failure |
| ERR_OUT_OF_MEMORY | Out of memory |
| ERR_STACK_OVERFLOW | Stack overflow |
| ERR_TIMEOUT | Execution timeout |

## 3.3 Permission Errors

| Code | Description |
|--------|-------------|
| ERR_PERMISSION_DENIED | Permission denied |
| ERR_CAPABILITY_MISSING | Capability not granted |
| ERR_DENY_BY_DEFAULT | Denied by default |

## 3.4 Module Errors

| Code | Description |
|--------|-------------|
| ERR_MODULE_NOT_FOUND | Module not found |
| ERR_MODULE_PARSE | Parse error |
| ERR_REQUIRE_CYCLE | Circular require |
| ERR_INVALID_EXPORT | Invalid export |

## 3.5 Network Errors

| Code | Description |
|--------|-------------|
| ERR_HOST_NOT_ALLOWED | Host not allowed |
| ERR_DNS_RESOLVE | DNS error |
| ERR_CONNECTION_REFUSED | Connection refused |
| ERR_TLS_ERROR | TLS error |

## 3.6 File System Errors

| Code | Description |
|--------|-------------|
| ERR_FILE_NOT_FOUND | File not found |
| ERR_PERMISSION_READ | Read denied |
| ERR_PERMISSION_WRITE | Write denied |
| ERR_PATH_TRAVERSAL | Path traversal detected |

## 3.7 Security Errors

| Code | Description |
|--------|-------------|
| ERR_MALWARE_DETECTED | Malware detected |
| ERR_INVALID_SIGNATURE | Invalid signature |
| ERR_SECRETS_DETECTED | Secrets in code |

## 3.8 Error Format

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

*Error codes compliant with Node.js error system.*
