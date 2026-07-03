# 05 - INTERACTIVE PERMISSION PROMPTS

## 5.1 Overview

When a script attempts an operation that was not granted via CLI flags, 3va can ask the user at the point of first access instead of silently denying it. This mode is called **interactive permission prompts** and is enabled by default in `3va run` when stderr is a TTY.

---

## 5.2 How It Works

The prompt fires inside `PermissionState::check()`, implemented in `crates/permissions/src/capability.rs`. The decision algorithm is:

```
check(capability)
       │
       ├── deny_all_<category>?  → DENY (silent)
       ├── in denied list?       → DENY (silent)
       ├── in granted list?      → ALLOW (silent)
       ├── interactive = true?   → PROMPT USER
       └── default               → DENY (silent)
```

The prompt is only shown once per unique capability. If the user chooses **Allow Always** (`A`), the capability is added to the granted list and subsequent checks pass silently. If the user chooses **Deny** (`N` or Enter), the capability is added to the denied list and subsequent checks fail silently.

---

## 5.3 TTY Detection

Interactive prompts require a real terminal. In non-interactive environments (CI, pipes, redirected input), `stdin` is not a TTY, so the runtime detects this and denies automatically:

```rust
// crates/permissions/src/capability.rs
if !std::io::stdin().is_terminal() {
    self.deny(required.clone());
    return false;
}
```

This prevents indefinite hangs in automated pipelines where no one is there to answer.

---

## 5.4 Prompt Format

```
[!] El script está intentando conectarse a la red 'api.example.com'.
¿Permitir? [y (Sí una vez) / N (Denegar) / A (Permitir Siempre)]
```

| Input | Effect |
|-------|--------|
| `y` | Allow this request only; next check for the same capability prompts again |
| `A` | Allow always; capability added to `granted` list for the rest of the session |
| `N` or Enter | Deny; capability added to `denied` list for the rest of the session |

The message is localized in Spanish; the prompt text varies by capability type:

| Capability | Message |
|-----------|---------|
| `FileRead(path)` | `leer el archivo '<path>'` |
| `FileWrite(path)` | `escribir el archivo '<path>'` |
| `Network(host)` | `conectarse a la red '<host>'` |
| `SpawnProcess` | `crear procesos hijos` |
| `EnvAccess` | `acceder a todas las variables de entorno` |
| `EnvVar(name)` | `acceder a la variable de entorno '<name>'` |
| `FFI(path)` | `llamar a librería nativa '<path>'` |

---

## 5.5 Enabling and Disabling

### Default behavior

Interactive prompts are active when `3va run` is called without explicit permission flags and stderr is a TTY:

```bash
3va run app.ts          # prompts on first access to any ungranted capability
```

### Pre-granting permissions (no prompts)

Pass explicit flags to avoid all prompts:

```bash
3va run app.ts --allow-net=api.example.com --allow-read=./data
```

Wildcards suppress all prompts for an entire category:

```bash
3va run app.ts --allow-read= --allow-net=  # unrestricted; no prompts ever
```

### Disabling prompts entirely (strict mode)

Pass `--no-prompt`, set `"3va": { "no-prompt": true }` in `package.json` (see
`docs/06-permissions/06-package-json-permissions.md`), or redirect stdin from
`/dev/null` to force deny-only behavior in an attended terminal:

```bash
3va run app.ts --no-prompt   # interactive = false; all ungranted → silent deny
3va run app.ts < /dev/null   # same effect, via non-TTY stdin
```

---

## 5.6 CI/CD Behavior

In CI environments stdin is not a TTY, so interactive mode is automatically off. Any ungranted capability is denied without output. Scripts that require network or filesystem access must declare permissions explicitly:

```yaml
# .github/workflows/ci.yml
- run: 3va run deploy.ts --allow-net=api.example.com --allow-read=./dist
```

Omitting the flags in CI causes silent denials that may appear as unexpected failures. Use `--audit-log` to diagnose:

```bash
3va run app.ts --audit-log=audit.json
cat audit.json  # shows every denied check with timestamp
```

---

## 5.7 Session Scope

Granted and denied capabilities from prompts are scoped to the current process lifetime. They are not written to `3va-lock.json`, `package.json`, or any file. Each new invocation of `3va run` starts with the capabilities provided via CLI flags plus whatever `package.json["3va"].permissions` declares (see `docs/06-permissions/06-package-json-permissions.md`).

---

*Implemented in `crates/permissions/src/capability.rs` — `PermissionState::prompt_user()`.*
