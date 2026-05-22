# 03 - OPTIONS AND FLAGS

## 3.1 Overview

This document details all options and flags available in the 3va CLI.

## 3.2 Permission Options

These flags grant capabilities to the `run` command. Permissions are denied by default.

### `--allow-read[=PATH]`

Grants read access to the file system.

```bash
# Global read access
3va run app.ts --allow-read

# Read access to a specific path
3va run app.ts --allow-read=/app/data
3va run app.ts --allow-read=/app/config --allow-read=/tmp
```

### `--allow-write[=PATH]`

Grants write access to the file system.

```bash
3va run app.ts --allow-write=/tmp/output
3va run app.ts --allow-write=/app/cache
```

### `--allow-net[=HOST]`

Grants network access. Also used by `install`, `update`, and `reinstall` to specify the registry.

```bash
# Global network access
3va run app.ts --allow-net

# Specific host
3va run app.ts --allow-net=api.example.com

# Multiple hosts
3va run app.ts --allow-net=api.example.com --allow-net=cdn.example.com
```

### `--allow-env`

Grants access to environment variables.

```bash
3va run app.ts --allow-env
```

### `--allow-child-process`

Grants permission to spawn child processes.

```bash
3va run app.ts --allow-child-process
```

### `--allow-ffi`

Grants permission to call native functions via FFI.

```bash
3va run app.ts --allow-ffi
```

## 3.3 Bundle Options

Available in `3va bundle`:

| Option | Abbreviation | Default | Description |
|--------|-------------|---------|-------------|
| `--output <PATH>` | `-o` | `dist/bundle.js` | Output file |
| `--split` | | | Enable code splitting |
| `--minify` | | | Minify output |
| `--source-map` | | | Generate `.map` file |

```bash
3va bundle src/index.ts -o dist/app.js --minify --source-map
3va bundle src/index.ts --split -o dist/
```

## 3.4 Test Options

Available in `3va test`:

| Option | Abbreviation | Description |
|--------|-------------|-------------|
| `--watch` | `-w` | Watch mode — re-runs on file changes |
| `--coverage` | | Statement-level coverage report |
| `--update-snapshots` | `-u` | Update snapshots instead of failing |

```bash
3va test --watch
3va test --coverage
3va test --update-snapshots
3va test tests/ --coverage --watch
```

## 3.5 Dev Server Options

Available in `3va dev`:

| Option | Default | Description |
|--------|---------|-------------|
| `--port` | `3000` | Port to listen on |
| `--host` | `127.0.0.1` | Address to bind to |
| `--open` | | Open browser on start |
| `--public-dir` | `public/` | Static files directory |

```bash
3va dev --port 8080 --host 0.0.0.0 --open
3va dev --public-dir www/
```

## 3.6 Audit Options

Available in `3va audit`:

| Option | Description |
|--------|-------------|
| `--deny` | Exit non-zero if HIGH or CRITICAL findings exist |
| `--update-cache` | Force fresh OSV data (bypasses 24 h TTL) |
| `--secrets` | Enable secrets scanner (Phase 3) |
| `--json` | Machine-readable JSON output |

```bash
3va audit --deny
3va audit --json --secrets
3va audit --update-cache --deny
```

## 3.7 Global Option

### `--accessible`

Enables accessible mode: disables colors, animations, and special characters. Compliant with EN 301 549 for screen readers and Braille terminals.

```bash
3va --accessible run app.ts
3va --accessible test --coverage
3va --accessible audit --deny
```

---

*Options compliant with IEEE 829 and CLI design.*
