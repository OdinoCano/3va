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

Each key under `permissions` is a **scope**: `"."` for the project root
(applies globally, no matter what code is executing), or a package name —
enforced, see §6.3.

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

## 6.3 Scopes Are Isolated Per Package

`vvva_permissions::scope` tracks "which package's code is currently
executing" in a thread-local (one `JsEngine` per thread, so this is safe).
`PermissionState` (`crates/permissions/src/capability.rs`) stores each
non-`"."` scope's `allow-*`/`deny-*` separately (`scoped_granted`/
`scoped_denied`) instead of merging it into the global set — `express`'s
`allow-net` only applies while `express`'s own code is running, never to any
other dependency or the app itself.

The scope is set by the `require()` wrapper in
`crates/js/src/builtins/modules.rs`: when a module does
`require('fs')`/`require('net')`/etc., it derives the requesting package's
name from the requesting module's own path (the innermost
`node_modules/<pkg>` segment; anything outside `node_modules` is `"."`) and
hands back a version of that builtin wrapped to bracket every call with
`__setCallerScope(pkgName)` / restore-on-exit — no changes needed at any of
the ~40 `perms().check(...)` call sites in `fs.rs`/`tcp.rs`/etc., since
`PermissionState::check()` reads the active scope internally.

Only the builtins that themselves perform capability-gated native calls are
wrapped: `fs`, `fs/promises`, `net`, `tls`, `dgram`, `child_process`. Wrapping
is skipped entirely (zero overhead) whenever a project declares no
non-`"."` scopes, which is the common case.

**Known residual gap:** the wrapper only intercepts plain factory functions
(`net.connect`, `createServer`, `dgram.createSocket`, `child_process.exec`/
`spawn`) — a PascalCase constructor like `net.Socket` is deliberately left
unwrapped (wrapping a constructor with a plain closure breaks `instanceof`),
so `new require('net').Socket()` followed by manually calling
`.connect()` on it bypasses scoping. `fs.createReadStream`/`createWriteStream`
are a special case: their actual native read/write happens on a later tick,
after the synchronous wrapper call has already reverted the scope, so those
two specifically capture and re-apply the creator's scope around the
deferred call (see the `__streamScope` comments in `fs.rs`) rather than
relying on the generic wrapper.

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

## 6.8 `allow-net` And Your Own Server's Bind Host

`allow-net` grants are matched against the exact host being checked (plus
`*`/`*.suffix` wildcards — see `docs/06-permissions/01-capability-model.md`).
That's the right model for *outbound* connections (`fetch`, `http.request`,
raw TCP `connect`): granting `allow-net: ["api.example.com"]` must not also
let the script reach `127.0.0.1` or some other host.

But `http.createServer()`/`net.createServer()` binding your **own** server is
a different action with a different risk, and a literal-match check used to
make it fail in a confusing way: `server.listen(port)` with no explicit host
defaults to binding `"0.0.0.0"` (all interfaces), so even a project with

```json
{
  "3va": {
    "permissions": {
      ".": { "allow-net": ["127.0.0.1"] }
    }
  }
}
```

would see its own `server.listen(8080, () => ...)` silently fail to start —
the permission check compared the literal bind host `"0.0.0.0"` against the
granted `"127.0.0.1"` and found no match, denying it, with no visible error
(the failure surfaces as an unhandled rejection inside the async listen
call, not a printed exception).

**Fix:** `PermissionState::check_bind(host)` (`crates/permissions/src/capability.rs`)
is used only at the `listen()` call sites (`crates/js/src/builtins/http_server.rs`,
`crates/js/src/builtins/tcp.rs`'s `__netListen`). When the bind host is a
wildcard/loopback address (`0.0.0.0`, `::`, `127.0.0.1`, `::1`, `localhost`),
*any* existing `allow-net` grant — for any host — authorizes the bind, since
running a server you wrote is treated as implied by having any network
capability at all. Explicit `deny-net` entries still block it.

This relaxation is bind-only. Outbound checks (`fetch`, `http.request`, TCP
`connect`, the PQ-TLS client) still call the strict `check()` — granting
`allow-net: ["api.example.com"]` still cannot be used to reach `127.0.0.1` or
any other unlisted host; that would otherwise be an SSRF hole.

In short: any `allow-net` entry (even for an unrelated host) is now
sufficient for your own `.listen()` to bind on `0.0.0.0`/`localhost`. If you
want your server bound to a *specific* non-wildcard address instead (e.g.
`server.listen(8080, "10.0.0.5")`), that still requires an `allow-net` entry
matching `10.0.0.5` exactly (or `*`).

---

## 6.9 Precedence Summary

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
