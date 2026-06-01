# 07 - Migration Tool (`3va codemod`)

## 7.1 Overview

To facilitate transitioning from v1.0.0 to v2.0.0, 3va provides an automated migration tool (`3va codemod`). The tool parses JavaScript and TypeScript source files, performs AST-level renames and parameter mappings to match the new v2.0.0 APIs, and writes clean, formatted changes back to disk.

---

## 7.2 CLI Specification

```bash
# Preview changes without writing them (dry-run)
3va codemod --from=1 --to=2 ./src --dry-run

# Run migration on specific files or directories
3va codemod --from=1 --to=2 ./src ./tests

# Revert changes using backups (if not using git)
3va codemod --revert
```

| Flag | Default | Description |
|------|---------|-------------|
| `--from <ver>` | `1` | Source version of the code (`1` or `1.x`) |
| `--to <ver>` | `2` | Target version of the code (`2` or `2.x`) |
| `--dry-run` | `false` | Emits a unified diff of proposed changes to stdout without modifying files |
| `--no-backup` | `false` | Suppresses creation of `.bak` backup files |
| `--revert` | — | Restores files from `.bak` backup files and removes backups |

---

## 7.3 Transformation Rules

The codemod parses files using the built-in **Oxc AST Parser** to identify precise nodes where v1 API structures are used.

### 7.3.1 Rule: `crypto.pq` Renames

The codemod converts snake_case and camelCase discrepancies in the post-quantum crypto APIs:

| Target (v1.0.0) | Replacement (v2.0.0) |
|-----------------|----------------------|
| `pq.kem.generateKeypair` | `pq.kem.generateKeyPair` |
| `pq.dsa.generateKeypair` | `pq.dsa.generateKeyPair` |

### 7.3.2 Rule: `pq.dsa.sign` Signature Realignment

Converts positional argument signatures to named object parameter patterns.

**Before (v1.0.0):**
```js
const sig = pq.dsa.sign(privateKeyHex, messageHex);
```

**After (v2.0.0):**
```js
const sig = pq.dsa.sign({ key: privateKeyHex, data: messageHex });
```

### 7.3.3 Rule: `pq.dsa.verify` Signature Realignment

**Before (v1.0.0):**
```js
const ok = pq.dsa.verify(publicKeyHex, messageHex, signatureHex);
```

**After (v2.0.0):**
```js
const ok = pq.dsa.verify({ key: publicKeyHex, data: messageHex, signature: signatureHex });
```

---

## 7.4 Implementation Architecture

1. **Pre-flight Checks:** Verify the target directory exists and check if git has unstaged changes. If unstaged changes exist, the codemod halts and warns the developer to commit or stash changes (can be overridden with `--force`).
2. **AST Parsing & Matching:** 3va's CLI parses files into an Abstract Syntax Tree (AST) using Oxc's parser. It walks the AST to find `CallExpression` nodes matching the rules (e.g. MemberExpression chain `pq.dsa.sign`).
3. **Source Patching:** Rather than generating new code from the AST (which could destroy spacing and custom formatting), the codemod calculates the exact offset span of matching nodes and patches the source file buffer directly.
4. **Backup Creation:** For every modified file, a duplicate `<filename>.<ext>.bak` is created unless `--no-backup` is specified.

---

## 7.5 Verification Plan

- **AST Matcher Tests:** Crate level unit tests verifying that various formats of `pq.dsa.sign` (such as destructured imports `const { sign } = pq.dsa`, imports with aliases, etc.) are matched and rewritten correctly.
- **Dry-run Integration Tests:** Runs the codemod over v1.0.0 integration test files, asserts that the stdout diff is a valid unified diff, and verifies no source files were modified.
- **AST Correctness:** The codemod compiles the modified code in a temporary environment to ensure the generated code remains syntactically valid TypeScript/JavaScript.
