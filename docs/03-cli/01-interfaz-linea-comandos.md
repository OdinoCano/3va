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

| Option | Description |
|--------|-------------|
| --help, -h | Shows help |
| --version, -v | Shows version |

## 1.3 Subcommands

### 1.3.1 Command: run
Executes a JavaScript or TypeScript file with the 3va runtime.

```
3va run [OPTIONS] <FILE>
```

**Permission flags:**
| Flag | Description |
|------|-------------|
| --allow-read[=PATH] | Allow file read access |
| --allow-write[=PATH] | Allow file write access |
| --allow-net[=HOST] | Allow network access |
| --allow-env | Allow environment variable access |
| --allow-child-process | Allow spawning child processes |

**Example:**
```bash
3va run app.ts --allow-read=/app --allow-net=api.example.com
```

### 1.3.2 Command: install
Installs one or more packages from the registry.

```
3va install [<PACKAGE[@VERSION]>] [--allow-net=HOST]
```

If no package is specified, installs all dependencies listed in `package.json`.

**Options:**
| Option | Description |
|--------|-------------|
| --allow-net | Registry host(s) to allow (e.g. `registry.npmjs.org`) |

**Example:**
```bash
3va install axios
3va install axios@1.7.9 --allow-net=registry.npmjs.org
```

### 1.3.3 Command: test
Runs the test suite.

```
3va test [OPTIONS] [FILES_OR_DIRS]...
```

**Options:**
| Option | Description |
|--------|-------------|
| --watch, -w | Watch mode — re-runs on file changes |
| --coverage | Generate statement-level coverage report |
| --update-snapshots, -u | Update snapshots instead of failing on mismatch |

No support for `--bail` or `--test-name-pattern`.

**Example:**
```bash
3va test
3va test tests/ --coverage
3va test --watch
```

### 1.3.4 Command: bundle
Bundles code for distribution.

```
3va bundle [OPTIONS] <ENTRY_FILE>
```

**Options:**
| Option | Description |
|--------|-------------|
| --output, -o | Output file path (default: `dist/bundle.js`) |
| --split | Enable code splitting (creates separate chunks) |
| --minify | Minify output |
| --source-map | Generate source map |

**Example:**
```bash
3va bundle index.ts --output dist/app.js --minify --source-map
```

### 1.3.5 Command: dev
Development server with hot module replacement.

```
3va dev [OPTIONS]
```

**Options:**
| Option | Description |
|--------|-------------|
| --port, -p | Port to listen on (default: 3000) |
| --host | Host to bind to (default: 127.0.0.1) |
| --open | Open browser automatically |
| --public-dir | Public directory for static assets (default: `public`) |

**Example:**
```bash
3va dev --port 8080 --open
```

### 1.3.6 Command: audit
Runs a 3-phase security audit (malware + CVE + secrets) on installed packages.

```
3va audit [OPTIONS]
```

**Options:**
| Option | Description |
|--------|-------------|
| --deny | Exit with error code 9 if vulnerabilities are found |
| --json | JSON-formatted output |
| --secrets | Include secrets scan in output |
| --update-cache | Force refresh of the OSV advisory cache |

**Example:**
```bash
3va audit --deny
3va audit --json --secrets
```

### 1.3.7 Command: sandbox
Opens an interactive JavaScript REPL with isolated permissions.

```
3va sandbox
```

**REPL commands:**
| Command | Description |
|---------|-------------|
| .help | Show available commands |
| .exit / .quit | Exit the sandbox |
| .clear | Clear the current buffer |
| .allow-read | Grant read permission |
| .allow-net | Grant network permission |
| .permissions | Show current permissions |

### 1.3.8 Command: doctor
Checks the environment and reports missing dependencies or misconfigurations.

```
3va doctor
```

### 1.3.9 Commands: update / reinstall

```
3va update [PACKAGES...] [--allow-net=HOST]
3va reinstall <PACKAGE[@VERSION]> [--allow-net=HOST]
```

`update` upgrades all packages in the lockfile (or specified packages) while preserving their original registry. `reinstall` force-reinstalls a single package.

## 1.4 Permission Flags

### 1.4.1 Permission System

Permissions follow the "deny by default" principle. Permission flags allow granular control over which operations are allowed.

### 1.4.2 Permission Flags

| Flag | Resource | Description |
|------|----------|-------------|
| --allow-read | File system | Allows reading files |
| --allow-read=PATH | Specific path | Allows reading a specific path |
| --allow-write | File system | Allows writing files |
| --allow-write=PATH | Specific path | Allows writing to a specific path |
| --allow-net | Network | Allows network connections |
| --allow-net=HOST | Hostname/IP | Allows connecting to a specific host |
| --allow-env | Environment | Allows accessing environment variables |
| --allow-child-process | Processes | Allows creating child processes |
| --allow-ffi | FFI | Allows native function calls |

### 1.4.3 Permission Examples

```bash
# Allow read-only access to current directory
3va run script.ts --allow-read=.

# Allow access to a specific API
3va run app.ts --allow-net=api.github.com

# Full permissions for development
3va run dev.ts --allow-read --allow-write --allow-net --allow-env --allow-child-process

# Install from npm
3va install express --allow-net=registry.npmjs.org
```

## 1.5 Error Management

### 1.5.1 Exit Codes

| Code | Meaning | Example |
|------|---------|---------|
| 0 | Success | Execution completed |
| 1 | General error | Unknown failure |
| 2 | Usage error | Invalid arguments |
| 4 | Permission error | Permission denied |
| 5 | Module error | Module not found |
| 6 | Runtime error | JS error |
| 7 | Bundle error | Build error |
| 8 | Test error | Test failed |
| 9 | Security error | Vulnerability detected |

---

*Interface compliant with IEEE 829 and CLI standards.*
