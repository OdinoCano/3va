# vvva_pm

Package manager for 3va — install, update, audit, and lockfile management.

## Key public functions

| Function | Description |
|----------|-------------|
| `install_packages(pkgs, allow_net)` | Download and extract packages |
| `audit_packages()` | Scan `node_modules/` and print a threat report |
| `audit_packages_silent()` | Same scan, no output — returns `Ok(false)` on critical threats |
| `update_packages(pkgs, allow_net)` | Re-fetch and re-lock packages |

## Lockfile

Dependencies are tracked in `3va-lock.json`. The lockfile pins exact versions and registries; install without a lockfile falls back to scanning `node_modules/` directly.

## Malware scanner

`malware_scanner` uses the oxc AST to detect dangerous patterns (obfuscated strings, `eval`, suspicious network calls) at install time.

## Docs

`docs/07-pm/`
