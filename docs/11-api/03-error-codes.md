# 03 - ERROR CODES

> **Status: PLANNED — not implemented.** The structured `ERR_*` error-code system
> described below is a design goal for a future version. As of v2.0.x the runtime
> reports errors as plain `Error` objects with descriptive messages (e.g.
> `PermissionError::FileReadDenied` surfaces as a thrown JS error with a
> human-readable message), without machine-readable codes or `metadata`.

## 3.1 Error Codes (planned)

3va will use structured error codes to facilitate debugging.

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

## 3.8 Error Format (planned)

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

## 3.9 Current Behavior (v2.0.x)

Today, errors surface as standard JS exceptions whose messages come from the
Rust error types:

| Area | Source type | Example message |
|------|-------------|-----------------|
| Permissions | `PermissionError` (`crates/permissions/src/enforcement.rs`) | `File read access denied for "/etc/passwd"` |
| Modules | loader errors (`crates/js`) | `Cannot find module 'x'` |
| Network | `fetch`/socket errors | `Network access denied: host 'evil.com'` |

Match on the message text, not on a code, until this specification is
implemented.

---

*Planned error codes follow the Node.js error system conventions.*
