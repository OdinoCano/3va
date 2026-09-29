# 07 - Migration Tool (`3va codemod`)

## 7.1 Overview

To facilitate transitioning from v1.0.0 to v2.0.0, 3va provides an automated migration tool (`3va codemod`). The tool rewrites JavaScript and TypeScript source files in place: it renames v1 APIs and maps positional arguments to the v2.0.0 object parameters, leaving the rest of each file untouched.

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

The codemod matches the literal call text (`pq.dsa.sign(`, `pq.kem.generateKeypair`, …) and balances parentheses to find each call's arguments. It does not parse the file: calls reached through an alias or a destructured import (`const { sign } = pq.dsa`) are not rewritten, so review `--dry-run` output and search for remaining v1 calls.

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

## 7.4 How it works

1. **File collection:** every `.js`/`.ts` (and related) file under the given paths is read. There is no pre-flight check of the git working tree and no `--force` flag: commit or stash first so the change is easy to review.
2. **Text rewriting:** the rules above are applied as literal text replacements; only the matched call text changes, so formatting elsewhere is preserved.
3. **Backup creation:** for every modified file, a copy `<filename>.<ext>.bak` is written next to it unless `--no-backup` is given. These files are not ignored by git: add `*.bak` to `.gitignore`, or delete them once the migration is reviewed (`3va codemod --revert` restores the originals and removes the backups).

---

## 7.5 Verification

- **Unit tests** in the CLI crate cover the renames and the `pq.dsa.sign` / `pq.dsa.verify` argument rewrites.
- **`--dry-run`** prints a unified diff and writes nothing; use it before every real run.
