# vvva_cli

The `3va` binary. Parses flags, builds the permission set, and dispatches to the appropriate runtime command.

## Commands

| Command | Description |
|---------|-------------|
| `3va run <file>` | Execute a JS/TS file in the sandbox |
| `3va install <pkg>` | Install a package from a registry |
| `3va audit` | Scan `node_modules/` for malware signatures |
| `3va bundle <file>` | Bundle a JS/TS entry point |
| `3va test` | Run the test suite |
| `3va fmt` | Format JS/TS source files |

## Permission flags

`--allow-read`, `--allow-write`, `--allow-net`, `--allow-env`, `--allow-child-process`, `--allow-ffi`

All flags accept an optional comma-separated scope (e.g. `--allow-read=/app,/tmp`).

## Docs

`docs/03-cli/`
