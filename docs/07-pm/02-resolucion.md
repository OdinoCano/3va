# 02 - DEPENDENCY RESOLUTION

## 2.1 Resolution Algorithm

3va's dependency resolver implements an npm-compatible algorithm to resolve the dependency tree.

## 2.2 Resolution Process

### 2.2.1 Steps

```
1. Parse project package.json
2. Get package metadata from registry
3. Resolve version conflicts
4. Build dependency tree
5. Verify lockfile
6. Generate new lockfile if necessary
```

### 2.2.2 Flow Diagram

```
┌──────────────┐
│ package.json│
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Parse    │
│ dependencies│
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Query    │ ───► Fetch package metadata
│   Registry  │       from npm registry
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Resolve  │ ───► Version matching algorithm
│  Versions  │       (semver)
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Detect   │ ───► peerDependencies conflicts
│  Conflicts │       duplicates, mismatches
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Build    │
│   Tree     │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Compare   │ ───► If diff, regenerate lockfile
│  lockfile   │
└──────────────┘
```

## 2.3 Version Matching

### 2.3.1 Semver

```rust
// Soporte para versiones semver
pub enum SemverMatch {
    Exact(String),        // 1.0.0
    Caret(String),        // ^1.0.0 -> >=1.0.0 <2.0.0
    Tilde(String),       // ~1.0.0 -> >=1.0.0 <1.1.0
    Range(String),       // >=1.0.0 <2.0.0
    Gt(String),          // >1.0.0
    Gte(String),         // >=1.0.0
    Lt(String),          // <1.0.0
    Lte(String),         // <=1.0.0
}
```

### 2.3.2 Resolution Algorithm

```rust
pub struct Resolver {
    registry: RegistryClient,
    cache: ResolutionCache,
}

impl Resolver {
    pub fn resolve(&self, deps: &HashMap<String, String>) -> DependencyGraph {
        let mut graph = DependencyGraph::new();

        for (name, version) in deps {
            // 1. Fetch package metadata
            let metadata = self.registry.fetch(name, version);

            // 2. Resolve version
            let resolved = self.resolve_version(&metadata, version);

            // 3. Add to graph
            graph.add(name, resolved);

            // 4. Recursively resolve dependencies
            if let Some(sub_deps) = resolved.dependencies {
                for (sub_name, sub_version) in sub_deps {
                    self.resolve_dep(&mut graph, &sub_name, &sub_version);
                }
            }
        }

        // 5. Resolve conflicts
        self.resolve_conflicts(&mut graph);

        graph
    }

    fn resolve_conflicts(&self, graph: &mut DependencyGraph) {
        // Detect and resolve version conflicts
        // - Same package with different versions
        // - peerDependencies conflicts
    }
}
```

### 2.3.3 Conflict Handling

| Scenario | Strategy |
|----------|----------|
| A → B@1, C → B@2 | Use B@2 (dupes allowed) |
| A → B@1, peer: B@2 | Resolve to compatible version |
| A → C@1, B → C@1 | Optimize (dedupe) |
| Circular | Resolve up to maximum depth |

## 2.4 Package Fetch

### 2.4.1 Download

```rust
pub struct PackageFetcher {
    client: HttpClient,
    cache: FileCache,
    registry: String,
}

impl PackageFetcher {
    pub async fn fetch(&self, package: &str, version: &str) -> anyhow::Result<Package> {
        // 1. Check cache
        if let Some(cached) = self.cache.get(package, version) {
            return Ok(cached);
        }

        // 2. Download from registry
        let tarball = self.client.download(&format!(
            "{}/{}/-/{}-{}.tgz",
            self.registry, package, package, version
        )).await?;

        // 3. Verify hash
        let expected_hash = self.get_hash_from_metadata(package, version)?;
        let actual_hash = sha256(&tarball);

        if expected_hash != actual_hash {
            anyhow::bail!("Hash mismatch for {}@{}", package, version);
        }

        // 4. Extract
        let extracted = self.extract(tarball)?;

        // 5. Cache
        self.cache.put(package, version, &extracted);

        Ok(extracted)
    }
}
```

## 2.5 Lockfile

### 2.5.1 Format

```json
{
    "lockfileVersion": 3,
    "name": "my-project",
    "version": "1.0.0",
    "packages": {
        "": {
            "dependencies": {
                "lodash": "^4.17.21"
            }
        },
        "node_modules/lodash": {
            "version": "4.17.21",
            "resolved": "https://registry.npmjs.org/lodash/-/lodash-4.17.21.tgz",
            "integrity": "sha512-..."
        },
        "node_modules/lodash/package.json": {
            "name": "lodash",
            "version": "4.17.21"
        }
    },
    "dependencies": {
        "lodash": {
            "version": "4.17.21",
            "resolved": "https://registry.npmjs.org/lodash/-/lodash-4.17.21.tgz",
            "integrity": "sha512-..."
        }
    }
}
```

### 2.5.2 Generation

```rust
pub fn generate_lockfile(graph: &DependencyGraph) -> Lockfile {
    let mut packages = HashMap::new();
    let mut dependencies = HashMap::new();

    for (name, node) in graph.nodes() {
        packages.insert(
            format!("node_modules/{}", name),
            LockfilePackage {
                version: node.version.clone(),
                resolved: node.resolved.clone(),
                integrity: node.integrity.clone(),
            }
        );

        dependencies.insert(name, LockfileDep {
            version: node.version.clone(),
            resolved: node.resolved.clone(),
            integrity: node.integrity.clone(),
            dependencies: node.deps.clone(),
        });
    }

    Lockfile {
        lockfileVersion: 3,
        name: graph.root_name.clone(),
        version: graph.root_version.clone(),
        packages,
        dependencies,
    }
}
```

## 2.6 Cache

### 2.6.1 Structure

```
~/.3va/cache/
├── metadata/
│   └── package/
│       └── versions.json
├── tarballs/
│   └── package-version.tgz
└── extracted/
    └── package-version/
        └── package/
```

### 2.6.2 Cache Policy

```rust
pub struct CacheConfig {
    pub max_size: u64,           // 1GB default
    pub ttl: Duration,          // 7 days
    pub prune_on_install: bool, // Clean on install
}

impl Cache {
    pub fn get_or_fetch(&mut self, package: &str) -> anyhow::Result<Package> {
        // 1. Check in-memory cache
        // 2. Check disk cache
        // 3. Fetch if not found
    }
}
```

---

*Resolution compliant with npm algorithm and semver specification.*