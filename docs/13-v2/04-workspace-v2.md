# 04 - Workspace v2

## 4.1 Overview

The v1.0.0 workspace implementation covers basic install and list operations. v2.0.0 adds topological execution, affected-only mode, inter-package graph visualization, dependency hoisting, and per-package permission scopes.

---

## 4.2 Topological Script Execution

```bash
# Run "build" in dependency order across all packages
3va workspace run build

# Run "test" only in packages affected by changes since main
3va workspace run test --affected

# Run "lint" in parallel (no topological ordering)
3va workspace run lint --parallel
```

**Execution model:**

1. Parse all `package.json` files in the workspace.
2. Build a directed acyclic graph (DAG) of inter-package dependencies.
3. Execute scripts in topological order (leaves first). Packages with no dependency relationship run concurrently.
4. A package's script is only started after all its dependency packages have completed successfully. On failure, dependent packages are skipped and reported as `skipped (upstream failed)`.

**Output:**

```
[workspace] build — 4 packages
  ✓ @my/utils          0.4s
  ✓ @my/core           1.2s   (depends on: @my/utils)
  ✓ @my/api            2.8s   (depends on: @my/core)
  ✗ @my/frontend       failed (depends on: @my/core)
  ↷ @my/e2e            skipped (upstream failed)
```

---

## 4.3 Affected-Only Mode (`--affected`)

```bash
3va workspace run test --affected [--base=main]
```

Detects packages changed since the merge base with `--base` (default: `main`) using `git diff --name-only`. A package is "affected" if:

- Any file inside its directory changed, **or**
- Any package it depends on (direct or transitive) is affected.

This allows CI to run only the tests that can be impacted by a PR, rather than the full suite.

---

## 4.4 Workspace Graph

```bash
# ASCII tree
3va workspace graph

# DOT format (pipe to graphviz)
3va workspace graph --format=dot | dot -Tsvg > graph.svg

# JSON (for tooling)
3va workspace graph --format=json
```

**ASCII example:**

```
@my/e2e
└── @my/frontend
    └── @my/api
        └── @my/core
            └── @my/utils
```

---

## 4.5 Dependency Hoisting

v2.0.0 deduplicates compatible package versions into a workspace-root `node_modules/`, following npm workspaces conventions:

- A package version is hoisted if it satisfies all dependents' version ranges.
- Conflicting versions remain in each package's local `node_modules/`.
- Hoisting is reported in `3va workspace install` output.

```
[workspace] Hoisted 47 packages to root node_modules/
[workspace] 3 packages retained locally (version conflicts):
  axios@0.27.2 (kept in @my/legacy-api)
  axios@1.6.0  (hoisted for @my/api, @my/core, @my/frontend)
```

---

## 4.6 Per-Package Permission Scopes

Each package can declare the permissions it requires in `package.json`. The workspace runner uses these declarations to restrict sub-process execution.

```jsonc
// packages/api/package.json
{
  "name": "@my/api",
  "3va": {
    "permissions": {
      "net": ["api.example.com", "db.internal"],
      "read": ["./config"],
      "write": ["./logs"]
    }
  }
}
```

When running `3va workspace run build`, each package's script runs with only the permissions declared in its `package.json#3va.permissions`. This prevents a compromised package script from accessing resources outside its declared scope, even if the workspace root was started with broader permissions.

### 4.6.1 Path Resolution Rules

Relative paths declared inside `3va.permissions` (such as `"./config"` or `"../shared"`) are resolved **relative to the package's individual directory** (e.g. `/projects/monorepo/packages/api/config`), rather than the workspace root. This maintains absolute local containment.

### 4.6.2 Hoisted Module Resolution

The module loader (both `EsmResolver` and CommonJS) is updated in Workspace v2 to walk up the directory tree to search for dependencies:
1. Search local `node_modules` under the current package subdirectory (resolves local dependency conflicts).
2. Walk up to search the parent `/node_modules` (resolves hoisted/shared dependencies).
3. If not found, continue walking up to the filesystem root.

---

## 4.7 `3va workspace info` (enhanced)

v2.0.0 adds richer output to `3va workspace info`:

```
Workspace root:    /projects/monorepo
Packages:          5
Total deps:        312 (47 hoisted, 8 local conflicts)
Scripts available: build, test, lint, typecheck
Farthest dep chain: @my/e2e → @my/frontend → @my/api → @my/core → @my/utils (depth 5)
```
