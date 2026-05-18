# 02 - RESOLUCIÓN DE DEPENDENCIAS

## 2.1 Algoritmo de Resolución

El resolvedor de dependencias de 3va implementa un algoritmo compatible con npm para resolver el árbol de dependencias.

## 2.2 Proceso de Resolución

### 2.2.1 Pasos

```
1. Parsear package.json del proyecto
2. Obtener metadata de paquetes del registry
3. Resolver conflictos de versiones
4. Construir árbol de dependencias
5. Verificar lockfile
6. Generar nuevo lockfile si es necesario
```

### 2.2.2 Diagrama de Flujo

```
┌──────────────┐
│ package.json│
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Parsear   │
│ dependencies│
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Consultar │ ───► Fetch package metadata
│   Registry  │       from npm registry
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Resolver  │ ───► Algoritmo de version matching
│  Versiones  │       (semver)
└──────┬───────┘
       │
       ▼
┌──────────────┐
│   Detectar  │ ───► peerDependencies conflicts
│  Conflictos │       duplicates, mismatches
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Construir  │
│   Árbol     │
└──────┬───────┘
       │
       ▼
┌──────────────┐
│  Comparar   │ ───► Si diff, regenera lockfile
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

### 2.3.2 Algoritmo de Resolución

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

            // 2. Resolver versión
            let resolved = self.resolve_version(&metadata, version);

            // 3. Añadir al grafo
            graph.add(name, resolved);

            // 4. Recursivamente resolver dependencias
            if let Some(sub_deps) = resolved.dependencies {
                for (sub_name, sub_version) in sub_deps {
                    self.resolve_dep(&mut graph, &sub_name, &sub_version);
                }
            }
        }

        // 5. Resolver conflictos
        self.resolve_conflicts(&mut graph);

        graph
    }

    fn resolve_conflicts(&self, graph: &mut DependencyGraph) {
        // Detectar y resolver conflictos de versiones
        // - Mismo paquete con diferentes versiones
        // - peerDependencies conflicts
    }
}
```

### 2.3.3 Manejo de Conflicts

| Scenario | Estrategia |
|----------|------------|
| A → B@1, C → B@2 | Usar B@2 (dupes allowed) |
| A → B@1, peer: B@2 | Resolver a versión compatible |
| A → C@1, B → C@1 | Optimizar (dedupe) |
| Circular | Resolver hasta profundidad máxima |

## 2.4 Fetch de Paquetes

### 2.4.1 Descarga

```rust
pub struct PackageFetcher {
    client: HttpClient,
    cache: FileCache,
    registry: String,
}

impl PackageFetcher {
    pub async fn fetch(&self, package: &str, version: &str) -> anyhow::Result<Package> {
        // 1. Verificar cache
        if let Some(cached) = self.cache.get(package, version) {
            return Ok(cached);
        }

        // 2. Descargar del registry
        let tarball = self.client.download(&format!(
            "{}/{}/-/{}-{}.tgz",
            self.registry, package, package, version
        )).await?;

        // 3. Verificar hash
        let expected_hash = self.get_hash_from_metadata(package, version)?;
        let actual_hash = sha256(&tarball);

        if expected_hash != actual_hash {
            anyhow::bail!("Hash mismatch for {}@{}", package, version);
        }

        // 4. Extraer
        let extracted = self.extract(tarball)?;

        // 5. Cachear
        self.cache.put(package, version, &extracted);

        Ok(extracted)
    }
}
```

## 2.5 Lockfile

### 2.5.1 Formato

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

### 2.5.2 Generación

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

### 2.6.1 Estructura

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

### 2.6.2 Política de Cache

```rust
pub struct CacheConfig {
    pub max_size: u64,           // 1GB default
    pub ttl: Duration,          // 7 días
    pub prune_on_install: bool, // Limpiar al instalar
}

impl Cache {
    pub fn get_or_fetch(&mut self, package: &str) -> anyhow::Result<Package> {
        // 1. Check内存 cache
        // 2. Check disk cache
        // 3. Fetch if not found
    }
}
```

---

*Resolución conforme a npm algorithm y semver specification.*