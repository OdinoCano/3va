# 06 - `package.json["3va"].permissions`

## 6.1 Overview

`3va run` reads permission grants declared under the `"3va"` key of the nearest
`package.json` (same directory as the entry file), in addition to `--allow-*`
CLI flags. This lets a project commit its required permissions instead of
requiring every invocation to repeat long `--allow-read=...` flags.

CLI flags and `package.json` grants are **merged, not replaced** — CLI flags
only add on top of what `package.json` already grants; they cannot revoke a
`package.json` grant. `deny-*` entries (see §6.4) are the only way to carve an
exception out of a grant.

Implemented in `crates/cli/src/main.rs`:
`read_package_json_permissions()`, `resolve_pkg_path()`, `expand_env_vars()`,
wired into `build_permissions()`.

---

## 6.2 Schema

```json
{
  "3va": {
    "no-prompt": true,
    "permissions": {
      ".": {
        "allow-read": ["db.js", "${NODE_MODULES_ROOT}/express@4.22.2"],
        "allow-env": ["SHELL", "SESSION_MANAGER"],
        "deny-read": ["${NODE_MODULES_ROOT}/express@4.22.2/node_modules/express/lib/express.js"]
      },
      "express": {
        "allow-net": ["*"]
      }
    }
  }
}
```

Each key under `permissions` is a **scope** (conventionally `"."` for the
project root, or a package name for readability). Scopes are cosmetic only —
see §6.3 for why.

Per-scope fields, one per capability category, mirroring the CLI flags:

| Field | Type | Equivalent CLI flag |
|---|---|---|
| `allow-read` | `string[]` (paths) | `--allow-read` |
| `allow-write` | `string[]` (paths) | `--allow-write` |
| `allow-net` | `string[]` (hosts, `*` wildcard) | `--allow-net` |
| `allow-env` | `string[]` (var names) | `--allow-env` |
| `allow-ffi` | `string[]` (paths) | `--allow-ffi` |
| `allow-child-process` | `bool` | `--allow-child-process` |
| `deny-read` / `deny-write` / `deny-net` / `deny-env` / `deny-ffi` | `string[]` | none (package.json-only) |
| `deny-child-process` | `bool` | none (package.json-only) |

Top-level (outside `permissions`):

| Field | Type | Equivalent CLI flag |
|---|---|---|
| `no-prompt` | `bool` | `--no-prompt` |

---

## 6.3 Scopes Are Merged, Not Isolated

`vvva_permissions::PermissionState` has no notion of "which module is
currently executing" — every `Capability` check is global to the process.
Because of that, `read_package_json_permissions()` unions every scope's
`allow-*`/`deny-*` into one flat set; it does **not** restrict `express`'s
`allow-net` to code running inside the `express` package. Naming a scope
`"express"` is documentation for humans, not an enforcement boundary.

True per-package isolation would require threading the calling module through
every `PermissionState::check()` call site — a much larger change than
reading a config file. If that's a hard requirement, treat it as a separate
feature request.

---

## 6.4 `deny-*`: Carving Exceptions Out Of A Broader Grant

`PermissionState::check()` consults the deny-list before the granted-list
(`crates/permissions/src/capability.rs`), so a `deny-*` entry always wins over
a broader `allow-*`, regardless of declaration order. This is the supported
way to grant a whole vendored directory by prefix while excluding one file —
e.g. a dependency with a known CVE in a specific file:

```json
{
  "3va": {
    "permissions": {
      ".": {
        "allow-read": ["node_modules/.pnpm/express@4.22.2"],
        "deny-read": ["node_modules/.pnpm/express@4.22.2/node_modules/express/lib/express.js"]
      }
    }
  }
}
```

`FileRead`/`FileWrite`/`FFI` grants match by path **prefix** (component-wise,
via `Path::starts_with`, including a symlink-resolved re-check) — see
`docs/06-permissions/01-capability-model.md`. Granting a directory once
already covers every file beneath it; you don't need to list each file
individually unless you need a `deny-*` exception.

---

## 6.5 Relative Paths Resolve Against `package.json`, Not `cwd`

Any `allow-read`/`allow-write`/`allow-ffi`/`deny-read`/`deny-write`/`deny-ffi`
entry that is not an absolute path is resolved against the directory
containing `package.json` (the project root), regardless of the directory
`3va run` was invoked from. This keeps the file portable: `node_modules/foo`
means the same thing whether you run `3va run index.js` from the project root
or `3va run backend/index.js` from a parent directory.

Already-absolute paths (`/...`) are left as-is.

---

## 6.6 `${VAR}` Expansion For Per-Environment Absolute Roots

Path fields also expand `${VAR}` placeholders against **the environment of
the host process launching `3va run`** — this happens before any
`PermissionState` exists, so it is unrelated to what the sandboxed script
itself can read via `process.env` (that's still gated by `allow-env`
separately).

This solves the case where an absolute root genuinely differs per
machine/team (`/var/node_module` on one server, `/local/bin/node_modules` on
another, `/tmp/node_modules` in CI) without hand-editing every path in
`package.json` per environment:

```json
{
  "3va": {
    "permissions": {
      ".": {
        "allow-read": ["${NODE_MODULES_ROOT}/express@4.22.2"]
      }
    }
  }
}
```

```bash
# server A
NODE_MODULES_ROOT=/var/node_module 3va run index.js

# server B — same package.json, nothing edited
NODE_MODULES_ROOT=/local/bin/node_modules 3va run index.js
```

If the variable is undefined, the `${VAR}` placeholder is left **literal**
rather than collapsing to an empty string. This fails closed: an unresolved
placeholder produces a path that cannot exist, instead of silently widening
the grant (e.g. `${UNSET}/x` must not become `/x`).

Multiple `${VAR}` placeholders per string, and plain strings with no
placeholder at all, both work — `expand_env_vars()` is a no-op pass-through
when there's nothing to expand.

---

## 6.7 `no-prompt`: Disabling Interactive Prompts

By default, `3va run` prompts on stderr/stdin for any capability that isn't
covered by `allow-*` (CLI or `package.json`) when both are a TTY — see
`docs/06-permissions/05-interactive-prompts.md`. In an attended terminal you
may want everything *not* explicitly listed to be denied silently instead of
interrupting execution with a prompt per ungranted capability.

Two equivalent ways to get that:

```bash
3va run index.js --no-prompt
```

```json
{
  "3va": {
    "no-prompt": true
  }
}
```

Either one sets `interactive = false` regardless of TTY state. Ungranted
capabilities are denied immediately (no blocking read from stdin), same as
already happened automatically in non-TTY contexts (CI, pipes).

---

## 6.8 Precedence Summary

For a single capability check:

1. Category-wide `deny_all_*` (only settable via CLI flags today, e.g. future `--deny-all-net`) → **DENY**
2. Explicit `deny-*` (from `package.json`) → **DENY**
3. Explicit `allow-*` (CLI flag **or** `package.json`, unioned) → **ALLOW**
4. `interactive` (TTY **and** no `--no-prompt` **and** no `"no-prompt": true`) → **PROMPT**
5. Otherwise → **DENY**

---

*See also: `docs/06-permissions/01-capability-model.md` (Capability enum,
prefix matching), `docs/06-permissions/05-interactive-prompts.md` (prompt
mechanics and `--no-prompt`).*
