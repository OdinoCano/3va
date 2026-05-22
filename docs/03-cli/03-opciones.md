# 03 - OPTIONS AND FLAGS

## 3.1 Options System

This document details all available options and flags in 3va.

## 3.2 Options by Category

### 3.2.1 Permission Options

#### `--allow-read`

Allows file system read operations.

Usage types:
```bash
# Allow global read access
--allow-read

# Allow reading a specific path
--allow-read=/path/to/dir
--allow-read=/path/to/file.js
--allow-read=/path/to/dir/*
```

Pattern matching:
- `/path` - Allows everything in that directory
- `/path/*` - Equivalent to the above
- `/path/**/*.js` - All .js files recursively

#### `--allow-write`

Allows file system write operations.

Usage types:
```bash
# Allow global write access
--allow-write

# Allow writing to a specific path
--allow-write=/tmp
--allow-write=/app/cache
```

#### `--allow-net`

Allows network connections.

Usage types:
```bash
# Allow all network access
--allow-net

# Allow specific hosts
--allow-net=api.example.com
--allow-net=*.example.com
--allow-net=192.168.1.0/24

# Multiple hosts
--allow-net=api.example.com --allow-net=cdn.example.com
```

Supported patterns:
- `host` - Exact host
- `*.host` - Subdomains
- `host:port` - Specific host and port
- `*.host:8080` - Subdomains with port

#### `--allow-env`

Allows access to environment variables.

Usage:
```bash
--allow-env
```

Limited access:
```bash
# Only specific variables (future)
--allow-env=PATH,HOME
```

#### `--allow-child-process`

Allows creating child processes.

Usage:
```bash
--allow-child-process
```

With restrictions (future):
```bash
--allow-child-process=git,curl
```

### 3.2.2 Deny Options

Deny flags are used to remove specific permissions when using a preset that grants more than necessary.

```bash
# Local development but no network
3va run dev.ts --allow-read --allow-write --deny-net

# Test environment without child processes
3va run test.ts --allow-read --deny-child-process
```

### 3.2.3 Runtime Options

#### `--inspect`

Enables the Chrome debug inspector.

```bash
3va run app.ts --inspect
# Listening on ws://127.0.0.1:9229/...
```

#### `--inspect-brk`

Inspector with initial breakpoint.

```bash
3va run app.ts --inspect-brk
```

#### `--watch`

Auto-reload on changes.

```bash
3va run app.ts --watch
```

### 3.2.4 Package Manager Options

#### `--save`, `--save-dev`, `--save-peer`

Dependency location.

```bash
3va install lodash --save           # dependencies
3va install jest --save-dev         # devDependencies
3va install react --save-peer       # peerDependencies
```

#### `--global`

Global package installation.

```bash
3va install typescript --global
```

### 3.2.5 Build Options

#### `--out-dir`

Output directory.

```bash
3va build index.ts --out-dir ./dist
```

#### `--format`

Bundle format.

```bash
3va build index.ts --format=esm    # ES Modules
3va build index.ts --format=cjs    # CommonJS
3va build index.ts --format=iife   # IIFE
```

#### `--target`

Compilation target.

```bash
3va build index.ts --target=node
3va build index.ts --target=browser
3va build index.ts --target=webworker
```

#### `--minify`

Minify the output.

```bash
3va build index.ts --minify
```

#### `--source-map`

Generate source maps.

```bash
3va build index.ts --source-map
3va build index.ts --source-map=hidden
```

### 3.2.6 Testing Options

#### `--coverage`

Generate coverage report.

```bash
3va test --coverage
```

#### `--update-snapshots`

Automatically update snapshots.

```bash
3va test --update-snapshots
```

#### `--reporter`

Select reporter.

```bash
3va test --reporter=spec
3va test --reporter=dot
3va test --reporter=json
```

## 3.3 Permission Presets

### 3.3.1 `preset:node`

Simulates Node.js behavior.

```bash
3va run app.ts --preset=node
# Equivalent to:
--allow-read --allow-write --allow-net --allow-env --allow-child-process
```

### 3.3.2 `preset:browser`

Simulates browser environment.

```bash
3va run app.ts --preset=browser
# Equivalent to:
--allow-net --allow-read --allow-write
```

### 3.3.3 `preset:none`

No permissions (more restrictive than default).

```bash
3va run app.ts --preset=none
# Equivalent to:
# (no permissions granted by default)
```

## 3.4 CLI Environment Variables

| Variable | Description |
|----------|-------------|
| `3VA_CONFIG` | Path to configuration file |
| `3VA_LOG_LEVEL` | Logging level |
| `3VA_CACHE_DIR` | Cache directory |
| `3VA_REGISTRY` | npm registry to use |

*Options compliant with IEEE 829 and CLI design.*
