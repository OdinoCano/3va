# 04 - USAGE EXAMPLES

## 4.1 Script Execution

### 4.1.1 Basic Execution

```bash
# JavaScript
3va run hello.js

# TypeScript (automatic transpilation)
3va run app.ts
```

### 4.1.2 With Granular Permissions

Permissions are denied by default. They are granted explicitly:

```bash
# Read access to a specific directory
3va run app.ts --allow-read=/app/data

# Network access to a specific host
3va run app.ts --allow-net=api.example.com

# Combining permissions
3va run app.ts --allow-read=/app/config --allow-net=api.example.com --allow-env

# Write access
3va run app.ts --allow-write=/tmp/output
```

---

## 4.2 Package Manager

### 4.2.1 Key Concepts

**The host in `--allow-net` defines the registry.** There is no separate `--registry` flag.

| Command | Registry used |
|---------|---------------|
| `--allow-net=registry.npmjs.org` | npm |
| `--allow-net=registry.yarnpkg.com` | Yarn |
| `--allow-net=jsr.io` | JSR |

**Without `--allow-net` the network is denied:**
```bash
3va install axios
# ✗ Network access denied.
#   3va install axios --allow-net=registry.npmjs.org
#   3va install axios --allow-net=registry.yarnpkg.com
#   3va install axios --allow-net=jsr.io
```

### 4.2.2 Installation from npm

```bash
# Latest version
3va install axios --allow-net=registry.npmjs.org

# Specific version
3va install axios@1.7.2 --allow-net=registry.npmjs.org

# Non-existent version → shows alternatives
3va install axios@99.0.0 --allow-net=registry.npmjs.org
# ✗ Version axios@99.0.0 not found in registry.
#
#   Versions available near 99.0.0:
#     axios@1.7.9
#     axios@1.7.8
#     axios@1.7.7
#     axios@1.7.6
#     axios@1.7.5
```

### 4.2.3 Installation from Yarn

```bash
3va install axios --allow-net=registry.yarnpkg.com
3va install react@18.3.1 --allow-net=registry.yarnpkg.com
```

### 4.2.4 Installation from JSR

JSR only accepts scoped packages (`@scope/name`):

```bash
# Correct — scoped package
3va install @std/path --allow-net=jsr.io
3va install @std/path@0.196.0 --allow-net=jsr.io

# Error — unscoped package not valid on JSR
3va install axios --allow-net=jsr.io
# ✗ JSR only supports scoped packages (e.g. @scope/name)
```

### 4.2.5 Multi-Registry Project

Dependencies from different registries can coexist in the same project:

```bash
# axios from npm, react from Yarn, @std/path from JSR
3va install axios --allow-net=registry.npmjs.org
3va install react --allow-net=registry.yarnpkg.com
3va install @std/path --allow-net=jsr.io
```

The lockfile `3va-lock.json` records the origin of each one:

```json
{
  "dependencies": {
    "axios":     { "version": "1.7.2",   "registry": "registry.npmjs.org" },
    "react":     { "version": "18.3.1",  "registry": "registry.yarnpkg.com" },
    "@std/path": { "version": "0.196.0", "registry": "jsr.io" }
  }
}
```

### 4.2.6 Reinstallation

```bash
3va reinstall axios --allow-net=registry.npmjs.org
3va reinstall @std/path --allow-net=jsr.io
```

### 4.2.7 Update

`update` respects the registry recorded in the lockfile for each package:

```bash
# Without --allow-net: the CLI informs which hosts are needed
3va update
# ✗ Update requires network access to:
#
#     registry.npmjs.org        (axios)
#     registry.yarnpkg.com      (react)
#     jsr.io                    (@std/path)
#
# Run: 3va update --allow-net=registry.npmjs.org,registry.yarnpkg.com,jsr.io

# Update all
3va update --allow-net=registry.npmjs.org,registry.yarnpkg.com,jsr.io

# Update a single package
3va update axios --allow-net=registry.npmjs.org

# Update specific packages from different registries
3va update axios @std/path --allow-net=registry.npmjs.org,jsr.io
```

**Migrating a package to another registry** (explicit action, recorded in the lockfile):

```bash
# axios will be updated from Yarn going forward
3va install axios --allow-net=registry.yarnpkg.com
```

---

## 4.3 Testing

```bash
# All tests in the current directory
3va test

# Specific directory
3va test tests/

# Specific file
3va test tests/auth.test.ts
```

---

## 4.4 Bundler

```bash
# Bundle with default output (dist/bundle.js)
3va bundle src/index.ts

# Custom output
3va bundle src/index.ts --output dist/app.js

# Run the resulting bundle
3va run dist/bundle.js --allow-net=api.example.com
```

---

## 4.5 Accessibility

Disables colors and animations for screen readers and Braille terminals (EN 301 549):

```bash
3va --accessible run app.ts
3va --accessible install axios --allow-net=registry.npmjs.org
3va --accessible update --allow-net=registry.npmjs.org,jsr.io
```

---

## 4.6 Scripts in `package.json`

```json
{
  "scripts": {
    "start":   "3va run src/index.ts --allow-net=api.mycompany.com",
    "build":   "3va bundle src/index.ts --output dist/app.js",
    "test":    "3va test",
    "install": "3va install --allow-net=registry.npmjs.org",
    "update":  "3va update --allow-net=registry.npmjs.org,jsr.io"
  }
}
```

---

*Examples compliant with IEEE 829 and 3va's capability model.*
