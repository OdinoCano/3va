use rquickjs::loader::{Loader, Resolver};
use rquickjs::{Ctx, Module};
use std::path::{Path, PathBuf};
use vvva_permissions::{Capability, PermissionState};

use crate::transpiler;

// ── Resolver ─────────────────────────────────────────────────────────────────

/// Resolves ESM import specifiers to canonical absolute file paths.
pub struct EsmResolver;

impl Resolver for EsmResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
    ) -> rquickjs::Result<String> {
        let resolved = resolve_esm(base, name);
        let canonical = resolved.canonicalize().unwrap_or(resolved);
        Ok(canonical.to_string_lossy().to_string())
    }
}

// ── Public helpers ────────────────────────────────────────────────────────────

/// Walk up from `start_dir` looking for the first `node_modules/<name>` that exists.
/// Mirrors Node.js module resolution: checks start_dir/node_modules, then ../node_modules, etc.
pub fn find_in_node_modules(start_dir: &Path, name: &str) -> Option<PathBuf> {
    let mut dir = start_dir.to_path_buf();
    loop {
        let pkg_dir = dir.join("node_modules").join(name);
        if pkg_dir.exists() {
            return Some(pkg_dir);
        }
        match dir.parent() {
            Some(p) if p != dir => dir = p.to_path_buf(),
            _ => return None,
        }
    }
}

// ── Loader ────────────────────────────────────────────────────────────────────

/// Loads ESM modules from the filesystem, transpiling TypeScript and checking permissions.
pub struct EsmLoader {
    pub permissions: PermissionState,
}

impl Loader for EsmLoader {
    fn load<'js>(&mut self, ctx: &Ctx<'js>, name: &str) -> rquickjs::Result<Module<'js>> {
        let path = Path::new(name);

        if !self
            .permissions
            .check(&Capability::FileRead(path.to_path_buf()))
        {
            return Err(rquickjs::Error::new_loading_message(
                name,
                "Permission denied: --allow-read required",
            ));
        }

        let source = std::fs::read_to_string(path)
            .map_err(|e| rquickjs::Error::new_loading_message(name, e.to_string()))?;

        let source = if name.ends_with(".ts") || name.ends_with(".tsx") {
            transpiler::transpile(&source)
        } else {
            source
        };

        Module::declare(ctx.clone(), name, source)
    }
}

// ── Path resolution helpers ───────────────────────────────────────────────────

/// Resolve an ESM import specifier relative to a base file path.
pub fn resolve_esm(base: &str, specifier: &str) -> PathBuf {
    if specifier.starts_with("./") || specifier.starts_with("../") {
        let base_dir = Path::new(base).parent().unwrap_or(Path::new("."));
        resolve_relative(&base_dir.join(specifier))
    } else if specifier.starts_with('/') {
        resolve_relative(&PathBuf::from(specifier))
    } else {
        // Walk up from the importing file's directory, then fall back to cwd.
        let base_dir = Path::new(base)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        resolve_node_module_esm(&base_dir, specifier)
    }
}

fn resolve_relative(base: &Path) -> PathBuf {
    if base.is_file() {
        return base.to_path_buf();
    }
    for ext in &["js", "mjs", "ts", "tsx", "cjs"] {
        let p = base.with_extension(ext);
        if p.is_file() {
            return p;
        }
    }
    for index in &["index.js", "index.mjs", "index.ts"] {
        let p = base.join(index);
        if p.is_file() {
            return p;
        }
    }
    base.to_path_buf()
}

/// Resolve the package root export from a parsed package.json.
/// Handles: string, {"import":..., "default":...}, {".":{...}}, {".":{string}}.
fn resolve_exports_root(json: &serde_json::Value, pkg_dir: &Path) -> Option<PathBuf> {
    let exports = &json["exports"];
    if exports.is_null() {
        return None;
    }
    let entry_str = if let Some(s) = exports.as_str() {
        // "exports": "./index.js"
        s.to_string()
    } else if let Some(dot) = exports.get(".") {
        // "exports": { ".": ... }
        resolve_condition(dot)?
    } else {
        // "exports": { "import": ..., "default": ... }
        resolve_condition(exports)?
    };
    let path = resolve_relative(&pkg_dir.join(entry_str.trim_start_matches("./")));
    if path.is_file() { Some(path) } else { None }
}

/// Pick a string value from a conditional exports object.
/// Priority: "import" > "module" > "default" > bare string.
fn resolve_condition(val: &serde_json::Value) -> Option<String> {
    if let Some(s) = val.as_str() {
        return Some(s.to_string());
    }
    for key in &["import", "module", "default"] {
        if let Some(s) = val[key].as_str() {
            return Some(s.to_string());
        }
        // Nested condition object
        if val[key].is_object()
            && let Some(s) = resolve_condition(&val[key])
        {
            return Some(s);
        }
    }
    None
}

fn resolve_node_module_esm(start_dir: &Path, name: &str) -> PathBuf {
    // Walk up the directory tree to find the nearest node_modules/<name>.
    let pkg_dir = find_in_node_modules(start_dir, name)
        .unwrap_or_else(|| start_dir.join("node_modules").join(name));
    if pkg_dir.is_dir() {
        let pkg_json = pkg_dir.join("package.json");
        if let Ok(content) = std::fs::read_to_string(&pkg_json)
            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content)
        {
            // exports field takes priority over module/main.
            let exports_entry = resolve_exports_root(&json, &pkg_dir);
            if let Some(path) = exports_entry {
                return path;
            }
            for field in &["module", "main"] {
                if let Some(entry) = json[field].as_str() {
                    let path = resolve_relative(&pkg_dir.join(entry));
                    if path.is_file() {
                        return path;
                    }
                }
            }
        }
        let fallback = resolve_relative(&pkg_dir);
        if fallback.is_file() {
            return fallback;
        }
        pkg_dir
    } else {
        pkg_dir
    }
}

/// Legacy helper: load and declare a module from a file path.
pub fn load_module<'js>(ctx: Ctx<'js>, path: &Path) -> rquickjs::Result<(String, Module<'js>)> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let name = canonical.to_string_lossy().to_string();

    let source = std::fs::read_to_string(&canonical)
        .map_err(|_| rquickjs::Error::new_from_js("io", "module file not found"))?;

    let source = if name.ends_with(".ts") || name.ends_with(".tsx") {
        transpiler::transpile(&source)
    } else {
        source
    };

    let module = Module::declare(ctx, name.clone(), source)?;
    Ok((name, module))
}
