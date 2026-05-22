# 01 - COMMAND LINE INTERFACE

## 1.1 Overview

The 3va command line interface (CLI) is the primary entry point for all system operations. Implemented via Rust's `clap` library, it provides a consistent user experience with modern tools like Bun, npm, and cargo.

## 1.2 Main Command Structure

### 1.2.1 Usage Format
```
3va [GLOBAL OPTIONS] <COMMAND> [COMMAND OPTIONS] [ARGUMENTS]
```

### 1.2.2 Global Options
Global options are available for all commands:

| Option | Description | Default |
|--------|-------------|---------|
| --help, -h | Shows help | - |
| --version, -v | Shows version | - |
| --verbose, -V | Verbose output | false |
| --quiet, -q | Suppresses output | false |
| --json | JSON-formatted output | false |
| --config | Configuration file | ~/.3va/config.json |

### 1.2.3 Verbosity Level
```
0: error      - Only critical errors
1: warn       - Warnings and errors
2: info       - General information (default)
3: debug      - Detailed debugging
4: trace      - Full traces
```

## 1.3 Subcommands

### 1.3.1 Command: run
Executes a JavaScript or TypeScript file with the 3va runtime.

```
3va run [OPTIONS] <FILE> [-- <SCRIPT_ARGS>...]
```

**Specific options:**
| Option | Description |
|--------|-------------|
| --inspect | Enables Chrome inspector |
| --inspect-brk | Inspector with initial breakpoint |
| --watch | Auto-reload on changes |
| --env | Environment variables as JSON |

**Example:**
```bash
3va run app.ts --allow-read=/app --allow-net=api.example.com
```

### 1.3.2 Command: install
Installs one or more packages from the registry.

```
3va install [OPTIONS] <PACKAGE>[@<VERSION>]...
```

**Specific options:**
| Option | Description |
|--------|-------------|
| --save | Adds to dependencies |
| --save-dev | Adds to devDependencies |
| --save-peer | Adds to peerDependencies |
| --global | Global installation |
| --allow-net | Allow network access |

**Example:**
```bash
3va install axios lodash --save
```

### 1.3.3 Command: test
Runs the test suite.

```
3va test [OPTIONS] [FILES_OR_PATTERNS]...
```

**Specific options:**
| Option | Description |
|--------|-------------|
| --watch | Watch mode |
| --coverage | Generate coverage |
| --update-snapshots | Update snapshots |
| --bail | Stop on first failure |
| --test-name-pattern | Filter by name |

**Example:**
```bash
3va test --coverage --bail
```

### 1.3.4 Command: build
Bundles code for distribution.

```
3va build [OPTIONS] <ENTRY_FILE>
```

**Specific options:**
| Option | Description |
|--------|-------------|
| --out-dir | Output directory |
| --format | Format: esm, cjs, iife |
| --target | Target: node, browser, webworker |
| --minify | Minify output |
| --source-map | Generate source maps |

**Example:**
```bash
3va build index.ts --out-dir ./dist --minify
```

### 1.3.5 Command: eval
Evaluates inline JavaScript code.

```
3va eval [OPTIONS] <CODE>
```

**Specific options:**
| Option | Description |
|--------|-------------|
| --print | Prints the result |
| --json | JSON output |

**Example:**
```bash
3va eval "console.log('Hello ' + 3va')"
```

## 1.4 Permission Flags

### 1.4.1 Permission System

Permissions follow the "deny by default" principle. Permission flags allow granular control over which operations are allowed.

### 1.4.2 Permission Flags

| Flag | Resource | Description |
|------|----------|-------------|
| --allow-read | File system | Allows reading files |
| --allow-read= | Specific path | Allows reading a specific path |
| --allow-write | File system | Allows writing files |
| --allow-write= | Specific path | Allows writing to a specific path |
| --allow-net | Network | Allows network connections |
| --allow-net= | Hostname/IP | Allows connecting to a specific host |
| --allow-env | Environment | Allows accessing environment variables |
| --allow-child-process | Processes | Allows creating child processes |
| --allow-ffi | FFI | Allows native function calls |

### 1.4.3 Deny Flags

| Flag | Description |
|------|-------------|
| --deny-read | Denies file read |
| --deny-write | Denies file write |
| --deny-net | Denies network connections |
| --deny-env | Denies environment access |
| --deny-child-process | Denies process creation |

### 1.4.4 Permission Examples

```bash
# Allow read-only access to current directory
3va run script.ts --allow-read=.

# Allow access to a specific API
3va run app.ts --allow-net=api.github.com

# Full permissions for development
3va run dev.ts --allow-read --allow-write --allow-net --allow-env --allow-child-process

# Deny environment but allow network
3va run app.ts --deny-env --allow-net
```

## 1.5 Error Management

### 1.5.1 Exit Codes

| Code | Meaning | Example |
|------|---------|---------|
| 0 | Success | Execution completed |
| 1 | General error | Unknown failure |
| 2 | Usage error | Invalid arguments |
| 3 | Configuration error | Invalid config |
| 4 | Permission error | Permission denied |
| 5 | Module error | Module not found |
| 6 | Runtime error | JS error |
| 7 | Bundle error | Build error |
| 8 | Test error | Test failed |
| 9 | Security error | Vulnerability detected |

### 1.5.2 Error Message Format

**Text mode:**
```
Error: Permission denied: FileRead(/etc/passwd)
  --> app.ts:5:1
```

**JSON mode:**
```json
{
  "error": "permission_denied",
  "message": "Permission denied: FileRead(/etc/passwd)",
  "location": {
    "file": "app.ts",
    "line": 5,
    "column": 1
  }
}
```

---

*Interface compliant with IEEE 829 and CLI standards.*
