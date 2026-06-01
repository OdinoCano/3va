# Permissions

3va uses a **capability-based, deny-by-default** permission model. No sensitive operation is allowed unless you explicitly grant it via a flag.

## Available permissions

| Flag | Grants access to |
|------|-----------------|
| `--allow-read[=<path>]` | Filesystem reads, optionally scoped to a path |
| `--allow-write[=<path>]` | Filesystem writes, optionally scoped to a path |
| `--allow-net[=<host>]` | Network (TCP, HTTP, WebSocket), optionally scoped to a host |
| `--allow-env[=<var>]` | Environment variables, optionally scoped to a variable name |
| `--allow-child-process` | Spawning child processes |
| `--allow-ffi[=<path>]` | Loading native `.node` addons (NAPI), optionally scoped to a path |

## Scoping examples

```bash
# Allow reading only from /app/config
3va run app.ts --allow-read=/app/config

# Allow network only to a specific host
3va run app.ts --allow-net=api.example.com

# Allow a specific environment variable
3va run app.ts --allow-env=DATABASE_URL

# Allow loading a specific native addon
3va run app.ts --allow-ffi=./build/Release/addon.node
```

## Unscoped (broad) permissions

Omitting the `=<scope>` grants access to the entire capability:

```bash
# Allow reading from anywhere on the filesystem
3va run app.ts --allow-read

# Allow all network access
3va run app.ts --allow-net
```

Use broad permissions only when necessary. Prefer scoped permissions in production.

## Dynamic permissions in the sandbox

Inside `3va sandbox`, permissions can be granted at runtime:

```
> .allow-read /tmp
> .allow-net api.example.com
> .permissions
```

## Package manager permissions

The package manager requires explicit network access to the registry host:

```bash
3va install axios --allow-net=registry.npmjs.org
```

Post-install scripts are **never** executed, regardless of permissions.

## Design rationale

The permission model is inspired by QubesOS, WASI, and the Chrome sandbox. The goal is to make the blast radius of a compromised dependency as small as possible. A package that only needs to parse JSON should never be able to read your SSH keys or phone home.
