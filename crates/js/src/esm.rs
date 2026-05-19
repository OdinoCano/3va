use rquickjs::{Ctx, Module};
use std::path::{Path, PathBuf};

use crate::transpiler;

/// Resolve an ESM import specifier relative to a base file path.
pub fn resolve_esm(base: &str, specifier: &str) -> PathBuf {
    if specifier.starts_with("./") || specifier.starts_with("../") {
        // Relative import: resolve from base file's directory
        let base_dir = Path::new(base).parent().unwrap_or(Path::new("."));
        let joined = base_dir.join(specifier);
        resolve_relative(&joined)
    } else if specifier.starts_with('/') {
        resolve_relative(&PathBuf::from(specifier))
    } else {
        // Bare specifier: node_modules
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        resolve_node_module_esm(&cwd, specifier)
    }
}

fn resolve_relative(base: &PathBuf) -> PathBuf {
    if base.is_file() {
        return base.clone();
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
    base.clone()
}

fn resolve_node_module_esm(cwd: &PathBuf, name: &str) -> PathBuf {
    let pkg_dir = cwd.join("node_modules").join(name);
    if pkg_dir.is_dir() {
        let pkg_json = pkg_dir.join("package.json");
        if let Ok(content) = std::fs::read_to_string(&pkg_json)
            && let Ok(json) = serde_json::from_str::<serde_json::Value>(&content)
        {
            // Prefer "module" (ESM entry), then "main"
            for field in &["module", "main"] {
                if let Some(entry) = json[field].as_str() {
                    let path = resolve_relative(&pkg_dir.join(entry));
                    if path.is_file() {
                        return path;
                    }
                }
            }
            // Try exports["."] as string
            if let Some(exp) = json["exports"]["."].as_str() {
                let path = resolve_relative(&pkg_dir.join(exp.trim_start_matches("./")));
                if path.is_file() {
                    return path;
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

/// Load and declare a module from a file path.
/// Returns the canonical path used as the module name.
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
