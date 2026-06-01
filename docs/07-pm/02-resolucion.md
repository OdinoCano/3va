# 02 - DEPENDENCY RESOLUTION

## 2.1 Resolution Algorithm

3va's dependency resolver implements an npm-compatible iterative algorithm that
fetches package metadata from the npm registry and walks the full transitive
dependency tree.  It is designed for correctness, determinism, and performance.

## 2.2 Resolution Process

### 2.2.1 Steps

```
1. Parse project package.json
2. Sort root dependencies alphabetically (determinism guarantee)
3. For each package on the stack:
   a. Skip if already resolved (with conflict warning on mismatch)
   b. Hit in-memory metadata cache → queue transitive deps
   c. Batch remaining uncached packages → fetch concurrently from registry
4. Populate DependencyGraph with resolved nodes
5. Generate lockfile from the resolved graph
```

### 2.2.2 Flow Diagram

```
┌──────────────┐
│ package.json │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│ Sort & push  │  ← alphabetical order for determinism
│ onto stack   │
└──────┬───────┘
       │
       ▼
┌──────────────┐     cache hit
│ Pop (name,   │────────────────► queue transitive deps (sorted)
│  version)    │
└──────┬───────┘
       │ cache miss
       ▼
┌──────────────┐
│ Batch fetch  │  ← parallel tokio::spawn per uncached package
│ (concurrent) │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Populate    │  ← find_best_match selects highest satisfying version
│  graph cache │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Conflict    │  ← tracing::warn! when resolved version ≠ required range
│  detection   │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Lockfile    │
│  generation  │
└──────────────┘
```

## 2.3 Version Matching

### 2.3.1 Supported Range Formats

The `SemverRange` parser handles all common npm range forms:

| Format | Example | Meaning |
|--------|---------|---------|
| Exact | `1.2.3`, `=1.2.3` | Exact version |
| Caret | `^1.2.3` | Compatible: `>=1.2.3 <2.0.0` |
| Caret (0.x) | `^0.2.3` | Pins minor: `>=0.2.3 <0.3.0` |
| Caret (0.0.x) | `^0.0.3` | Exact patch: `=0.0.3` |
| Tilde | `~1.2.3` | Patch-compatible: `>=1.2.3 <1.3.0` |
| Comparators | `>1.0.0`, `>=1.0.0`, `<2.0.0`, `<=2.0.0` | Single bound |
| Compound | `>=1.0.0 <2.0.0` | AND of two ranges |
| Wildcard | `*`, `` (empty) | Any version |
| X-range (major) | `1`, `1.x` | Same as `^1.0.0` |
| X-range (minor) | `1.2`, `1.2.x` | Same as `~1.2.0` |
| Dist-tags | `latest`, `next`, `beta` | Treated as `*` (any version) |

### 2.3.2 Resolution Strategy

`Resolver::find_best_match` selects the highest version that satisfies a range:

```rust
pub(crate) fn find_best_match<'a>(
    nodes: &'a [DependencyNode],
    version: &str,
) -> Option<&'a DependencyNode>
```

- Parses `version` into a `SemverRange`; returns `None` for truly invalid strings
  and logs a `tracing::warn!` for any node whose stored version string cannot be parsed
- Picks the candidate with the highest parsed `Semver` (major → minor → patch →
  pre-release ordering)

### 2.3.3 Determinism

Resolution order is deterministic across runs:

- The initial dependency stack is sorted **descending by name** before the first
  pop, so `pop()` processes packages in ascending alphabetical order
- Transitive dependencies are sorted the same way before being pushed onto the stack
- `HashMap` iteration order is never relied upon for resolution outcome

This guarantees that the same `package.json` always produces the same
`3va-lock.json` regardless of the runtime hash-map seed.

### 2.3.4 Conflict Detection

When a package is already resolved and a new transitive constraint arrives that
the resolved version does not satisfy, a structured warning is emitted:

```
WARN version conflict: resolved version does not satisfy this constraint
  package=foo resolved=1.5.0 required=^2.0.0
```

Full backtracking is not performed; the first satisfying version wins.  The
warning surfaces conflicts that would otherwise be silent.

## 2.4 Parallel Package Fetching

Uncached packages are fetched concurrently in batches:

```
while stack is not empty:
  if next package is in cache → use it
  else:
    collect all uncached packages from top of stack → batch
    tokio::spawn one HTTP GET per batch item        → concurrent
    await all handles
    push batch back onto stack (now cached)
```

Each fetch uses a 20-second timeout.  Packages that return a non-2xx response
or cannot be parsed are skipped with a `tracing::warn!`.

## 2.5 Lockfile

### 2.5.1 Format (`3va-lock.json`)

```json
{
  "lockfileVersion": 1,
  "name": "my-project",
  "version": "1.0.0",
  "dependencies": {
    "lodash": {
      "version": "4.17.21",
      "resolved": "https://registry.npmjs.org/lodash/-/lodash-4.17.21.tgz",
      "integrity": "sha512-...",
      "registry": "https://registry.npmjs.org"
    }
  }
}
```

### 2.5.2 `collect_installed` — Installed Package Discovery

`audit_packages` and `audit_packages_silent` both call `collect_installed(node_modules)` to enumerate installed packages:

1. **Lockfile path**: derived from `node_modules.parent()` — workspace-safe; not
   hardcoded to the process CWD
2. **Lockfile present**: reads exact versions from `3va-lock.json`
3. **Lockfile absent**: walks `node_modules/` and reads each package's
   `package.json` for the `"version"` field via `read_package_version`

## 2.6 Cache

### 2.6.1 Structure

```
~/.3va/cache/
├── metadata/
│   └── <package>/
│       └── versions.json
├── tarballs/
│   └── <package>-<version>.tgz
└── extracted/
    └── <package>-<version>/
```

### 2.6.2 In-memory Metadata Cache

The `Resolver` holds an in-memory `HashMap<String, Vec<DependencyNode>>` keyed
by package name.  On a cache hit, no HTTP request is made and `find_best_match`
selects the best version from the already-fetched version list.

---

*Implemented in `crates/pm/src/resolver.rs` and `crates/pm/src/semver.rs`.*
