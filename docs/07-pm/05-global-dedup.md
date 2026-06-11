# 05 - GLOBAL CONTENT-ADDRESSABLE STORE

## 5.1 Overview

3va uses a **global content-addressable store** at `~/.3va/store/` to deduplicate packages across projects on the same machine. A package at a given version is extracted from its tarball exactly once; every project that needs it hard-links the files directly from the store into its own `node_modules/`.

This mirrors pnpm's store model. The result is that disk space is shared: ten projects depending on `axios@1.7.2` keep one copy on disk, not ten.

---

## 5.2 Directory Layout

```
~/.3va/
└── store/
    ├── registry.npmjs.org/
    │   ├── axios@1.7.2/
    │   │   ├── package.json
    │   │   ├── dist/
    │   │   └── ...
    │   └── lodash@4.17.21/
    │       └── ...
    └── jsr.io/
        └── @std+path@0.196.0/
            └── ...
```

The store is keyed by `{registry}/{name}@{version}`. Scoped packages (`@scope/name`) use `+` as the separator in the directory name to remain filesystem-safe (`@scope+name@version`).

---

## 5.3 Per-project Layout (`node_modules/`)

When a package is installed into a project, 3va creates a **virtual store** inside the project's `node_modules/.3va/` and a symlink at the top level:

```
node_modules/
├── axios                          ← symlink → .3va/axios@1.7.2/node_modules/axios/
└── .3va/
    └── axios@1.7.2/
        └── node_modules/
            └── axios/             ← hard-linked files from ~/.3va/store/
                ├── package.json
                ├── dist/
                └── ...
```

This mirrors pnpm's `node_modules/.pnpm/` layout. The top-level `node_modules/` contains only symlinks; the actual bytes are in `.3va/` (per-project) which are in turn hard-linked from the global store.

---

## 5.4 Install Flow

```
3va install axios
         │
         ▼
Resolve version (registry API)
         │
         ▼
~/.3va/store/registry.npmjs.org/axios@1.7.2/ exists?
         │
    ┌────┴────┐
   Yes        No
    │          │
    │          ▼
    │    Download tarball
    │    Verify integrity (SHA-512)
    │    Extract atomically via tmp dir + rename(2)
    │    Write to store
    │
    ▼
link_to_virtual_store:
  hard-link store → node_modules/.3va/axios@1.7.2/
         │
         ▼
Create symlink:
  node_modules/axios → .3va/axios@1.7.2/node_modules/axios/
         │
         ▼
Update 3va-lock.json
```

### Atomic extraction

Extraction uses a temporary sibling directory followed by `rename(2)`. If the process is interrupted, no partial state is left in the store:

```
~/.3va/store/registry.npmjs.org/axios@1.7.2__tmp_<pid>/   ← extraction target
                                                 ↓ rename(2)
~/.3va/store/registry.npmjs.org/axios@1.7.2/               ← final location
```

Two concurrent processes writing the same package are safe: both renames succeed and the result is idempotent (last writer wins, both payloads are identical for the same version).

---

## 5.5 Hard-link vs. Copy Fallback

Files are hard-linked from the global store when possible. A hard link is attempted first; if it fails (cross-filesystem, unsupported FS, or permissions), a full copy is performed as a fallback:

```rust
// crates/pm/src/store.rs
if std::fs::hard_link(&src_path, &dst_path).is_err() {
    std::fs::copy(&src_path, &dst_path)?;
}
```

On most Linux and macOS setups, hard links are used and no extra bytes are consumed.

---

## 5.6 Per-project Independence

Although packages are shared at the byte level, **permissions are declared per-project**. Two projects can depend on the same `axios@1.7.2` bytes but grant it different capabilities:

```bash
# Project A: only allows read from a config dir
cd /projects/app-a
3va run main.ts --allow-net=api.internal --allow-read=./config

# Project B: unrestricted network
cd /projects/app-b
3va run main.ts --allow-net=
```

The global store is concerned only with storage, not with execution context.

---

## 5.7 Store Maintenance

### Verify store integrity

```bash
3va store verify
# Checks every entry has a complete package.json
# Reports corrupt or partial extractions
```

### Inspect store size

```bash
3va store stats
# Shows total packages, disk usage, registries present
```

### Clear unused entries

```bash
3va store prune
# Removes store entries not referenced by any project's 3va-lock.json
# on the machine (cross-project scan — planned for v2.1)
```

---

## 5.8 Environment Variable Override

The store root can be relocated via environment variable (useful in containers or CI where `~` is ephemeral):

```bash
export _3VA_STORE=/mnt/cache/3va-store
3va install axios --allow-net=registry.npmjs.org
# Extracts to /mnt/cache/3va-store/registry.npmjs.org/axios@1.7.2/
```

---

*Implemented in `crates/pm/src/store.rs` — `ContentStore`.*
