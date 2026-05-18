# 02 - CARGA DE MÓDULOS

## 2.1 Sistema de Carga de Módulos

El loader de módulos de 3va implementa resolución de módulos compatible con Node.js, soportando tanto CommonJS (CJS) como ECMAScript Modules (ESM).

## 2.2 Algoritmo de Resolución

### 2.2.1 Resolución de Rutas

```
resolve(request, parentPath):
    ├── Si es URL absoluta:
    │   └── Devolver URL tal cual
    │
    ├── Si es módulo integrado:
    │   └── Devolver ruta al módulo nativo
    │
    ├── Si es ruta relativa (./ o ../):
    │   └── Resolver contra parentPath
    │
    └── Si es nombre de paquete:
        └── Buscar en node_modules:
            1. Verificar ./node_modules/ de parent
            2. Subir a parent.parent/node_modules/
            3. Repetir hasta raíz
            4. En cada nivel, buscar package.json "main"
            5. Si no tiene main, buscar index.js/ts
```

### 2.2.2 Resolución de Extensiones

```
resolveExtensions = ['.mjs', '.cjs', '.js', '.ts', '.tsx', '.jsx', '.json']

Para cada extensión:
    1. Probar con extensión
    2. Si archivo existe, devolver
    3. Si no, probar siguiente
```

### 2.2.3 Implementación en Rust

```rust
pub struct ModuleResolver {
    root_path: PathBuf,
    node_modules_paths: Vec<PathBuf>,
    extensions: Vec<String>,
}

impl ModuleResolver {
    pub fn resolve(&self, request: &str, parent: &ModuleId) -> anyhow::Result<PathBuf> {
        // 1. Determinar tipo de request
        match ModuleRequestType::parse(request) {
            // Módulo integrado
            ModuleRequestType::BuiltIn(name) => {
                self.resolve_builtin(name)
            }
            // Ruta relativa
            ModuleRequestType::Relative { path, is_dir } => {
                self.resolve_relative(path, parent, is_dir)
            }
            // Paquete npm
            ModuleRequestType::Package { name, subpath } => {
                self.resolve_package(name, subpath, parent)
            }
        }
    }

    fn resolve_relative(&self, path: &str, parent: &ModuleId, _is_dir: bool) -> anyhow::Result<PathBuf> {
        let base = parent.path.parent().unwrap_or(&self.root_path);
        let resolved = base.join(path);

        // Probar extensiones
        for ext in &self.extensions {
            let with_ext = resolved.with_extension(ext.trim_start_matches('.'));
            if with_ext.exists() {
                return Ok(with_ext);
            }
        }

        // Si es directorio, buscar index
        if resolved.is_dir() {
            for ext in &self.extensions {
                let index = resolved.join(format!("index{}", ext));
                if index.exists() {
                    return Ok(index);
                }
            }
        }

        anyhow::bail!("Module not found: {}", request)
    }

    fn resolve_package(&self, name: &str, subpath: &str, parent: &ModuleId) -> anyhow::Result<PathBuf> {
        // Buscar en node_modules upward
        let mut current = parent.path.parent().unwrap_or(&self.root_path).to_path_buf();

        loop {
            let node_modules = current.join("node_modules").join(name);
            if node_modules.exists() {
                // Leer package.json
                return self.resolve_package_entry(&node_modules, subpath);
            }

            if !current.pop() {
                anyhow::bail!("Package not found: {}", name);
            }
        }
    }
}
```

## 2.3 CommonJS

### 2.3.1 Implementación de require()

```rust
pub struct CommonJsLoader {
    resolver: ModuleResolver,
    cache: ModuleCache,
}

impl CommonJsLoader {
    pub fn require(&self, ctx: &Context, request: &str) -> anyhow::Result<Value> {
        // 1. Verificar cache
        if let Some(cached) = self.cache.get(request) {
            return Ok(cached.exports.clone());
        }

        // 2. Resolver path
        let path = self.resolver.resolve(request, &self.current_module)?;

        // 3. Verificar permiso de lectura
        if !permissions.check(&Capability::FileRead(path.clone())) {
            anyhow::bail!("Permission denied: FileRead({})", path.display());
        }

        // 4. Cargar código fuente
        let source = std::fs::read_to_string(&path)?;

        // 5. Envolver en función (wrapper)
        let wrapped = Self::wrap(source, &path);

        // 6. Evaluar
        let result = ctx.eval(&wrapped)?;

        // 7. Cachear y devolver
        self.cache.set(request, module);
        Ok(module.exports)
    }

    fn wrap(source: &str, path: &Path) -> String {
        let filename = path.to_string_lossy();
        let dirname = path.parent().unwrap_or(path).to_string_lossy();

        format!(
            r#"(function(exports, require, module, __filename, __dirname) {{
{}
}})"#,
            source
        )
    }
}
```

### 2.3.2 module.exports

```javascript
// Módulos CommonJS en 3va
module.exports = { foo: 'bar' };
exports.bar = 'baz';

// Equivalente a:
module.exports.bar = 'baz';
```

### 2.3.3 Resolution de package.json

```json
// package.json
{
    "main": "dist/index.js",
    "exports": {
        ".": "./dist/index.js",
        "./feature": "./dist/feature.js"
    }
}
```

## 2.4 ESM (ECMAScript Modules)

### 2.4.1 Carga de Módulos ESM

```rust
pub struct EsmLoader {
    resolver: ModuleResolver,
    import_map: ImportMap,
}

impl EsmLoader {
    pub async fn load_module(&self, url: &str) -> anyhow::Result<Module> {
        // 1. Resolver URL (soporta import map)
        let resolved = self.resolve_import(url, &self.import_map)?;

        // 2. Verificar permiso
        if !permissions.check(&Capability::Network(&resolved)) && !resolved.starts_with("file://") {
            anyhow::bail!("Permission denied: Network({})", resolved);
        }

        // 3. Fetch del código fuente
        let source = self.fetch_source(&resolved).await?;

        // 4. Parsear como módulo
        let ast = Self::parse_module(&source, &resolved)?;

        // 5. Evaluar
        let module = self.evaluate_module(ast, &resolved)?;

        Ok(module)
    }
}
```

### 2.4.2 Import/Export

```javascript
// Named exports
export const foo = 1;
export function bar() { }

// Default export
export default function() { }

// Import
import { foo, bar } from './module';
import defaultExport from './module';
import * as ns from './module';

// Dynamic import
const mod = await import('./module');
```

### 2.4.3 Import Maps

```html
<!-- Soporte de import maps -->
<script type="importmap">
{
    "imports": {
        "lodash": "https://cdn.example.com/lodash.js",
        "@scope/package": "https://cdn.example.com/scope/package.js"
    }
}
</script>
```

## 2.5 Module Cache

### 2.5.1 Cache de Módulos

```rust
pub struct ModuleCache {
    modules: HashMap<ModuleKey, CachedModule>,
    imports_pending: HashMap<ModuleKey, Vec<Receiver<Result<Value>>>>,
}

pub struct CachedModule {
    pub exports: Value,
    pub loaded: bool,
    pub loading: bool,
}

impl ModuleCache {
    pub fn get(&self, key: &ModuleKey) -> Option<&CachedModule> {
        self.modules.get(key)
    }

    pub fn set(&mut self, key: ModuleKey, module: CachedModule) {
        self.modules.insert(key, module);
    }
}
```

### 2.5.2 Invalidation de Cache

```javascript
// Para debugging: invalidar cache
3va run app.ts --no-cache
// O en código (futuro):
module._cache = null;
delete require.cache[require.resolve('./module')];
```

---

*Carga de módulos conforme a Node.js Module Resolution y ECMAScript Spec.*