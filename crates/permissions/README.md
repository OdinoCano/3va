# vvva_permissions

Capability-based permission model for the 3va sandbox. All access is denied by default; capabilities must be granted explicitly at startup.

## Key types

- **`PermissionState`** — holds the granted capability set; call `.check(&Capability)` at every syscall boundary
- **`Capability`** — enum of grantable operations: `FileRead(path)`, `FileWrite(path)`, `Network(host)`, `EnvAccess`, `EnvVar(name)`, `SpawnProcess`
- **`PermissionEnforcer`** — wraps `PermissionState` and returns `Err` on denied access (use this in builtins)

## Design

Permissions follow a deny-by-default, prefix-matching model. `--allow-read=/app` grants `FileRead` for any path under `/app`; nothing else. There are no ambient capabilities.

## Docs

`docs/06-permissions/`
