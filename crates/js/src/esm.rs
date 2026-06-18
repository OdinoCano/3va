use rquickjs::loader::{Loader, Resolver};
use rquickjs::{Ctx, Module};
use std::path::{Path, PathBuf};
use vvva_permissions::{Capability, PermissionState};

use crate::builtins::modules;
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
/// Only finds the package directory (no subpath), for bare-name lookups.
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

/// Detect whether source is ESM.
/// Extension wins: .mjs = always ESM, .cjs = always CJS.
/// For .js/.ts fall back to scanning top-level import/export statements,
/// skipping line and block comments to avoid false positives.
fn source_is_esm(code: &str, path: &str) -> bool {
    if path.ends_with(".mjs") {
        return true;
    }
    if path.ends_with(".cjs") {
        return false;
    }
    let mut in_block = false;
    for line in code.lines() {
        let t = line.trim();
        if in_block {
            // Close the block comment; code may follow */ on the same line,
            // but we conservatively skip the whole line.
            if t.contains("*/") {
                in_block = false;
            }
            continue;
        }
        if t.is_empty() || t.starts_with("//") {
            continue;
        }
        if t.starts_with("/*") {
            in_block = true;
            continue;
        }
        if t.starts_with("import ")
            || t.starts_with("import{")
            || t.starts_with("export ")
            || t.starts_with("export{")
            || t.starts_with("export default")
        {
            return true;
        }
        // First non-comment, non-empty line that isn't an import/export
        // → CJS. Stop scanning to avoid false positives from inline imports
        // inside function bodies.
        break;
    }
    false
}

/// Detect commonly-used named CJS export patterns via simple string scanning.
fn extract_cjs_named_exports(source: &str) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut names = BTreeSet::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("exports.")
            && let Some(eq_pos) = rest.find(" = ")
        {
            let name = rest[..eq_pos].trim();
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
            {
                names.insert(name.to_string());
            }
        }
        if let Some(rest) = trimmed.strip_prefix("module.exports.")
            && let Some(eq_pos) = rest.find(" = ")
        {
            let name = rest[..eq_pos].trim();
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
            {
                names.insert(name.to_string());
            }
        }
    }
    names.retain(|n| n != "default" && n != "__esModule");
    names.into_iter().collect()
}

/// Wrap a CommonJS source in an ESM module shell.
/// Provides `require`, `module`, `exports`, `__filename`, `__dirname` in scope
/// and re-exports all named exports as static ESM exports.
fn wrap_cjs_for_esm(source: &str, filename: &str, dirname: &str) -> String {
    let named_exports = extract_cjs_named_exports(source);
    let mut export_stmts = String::new();
    for name in &named_exports {
        export_stmts.push_str(&format!(
            "export const {} = __cjsExports__['{}'];\n",
            name, name
        ));
    }
    format!(
        r#"const __module = {{ exports: {{}} }};
const __exports = __module.exports;
const __require = globalThis.require;
const __filename = {:?};
const __dirname = {:?};
(function(exports, module, require, __filename, __dirname) {{
{}
}})(__exports, __module, __require, __filename, __dirname);
const __cjsExports__ = __module.exports;
export default __cjsExports__;
{}"#,
        filename, dirname, source, export_stmts
    )
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

        let source = if name.ends_with(".tsx") || name.ends_with(".jsx") {
            transpiler::transpile_jsx(&source)
        } else if name.ends_with(".ts") || name.ends_with(".mts") || name.ends_with(".cts") {
            transpiler::transpile(&source)
        } else {
            transpiler::transpile_js(&source)
        };

        // If the file is CommonJS, wrap it for ESM so that
        // `import 'cjs-package'` works with default and named exports.
        if !source_is_esm(&source, name) {
            let dirname = path
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let filename = name.to_string();
            let wrapped = wrap_cjs_for_esm(&source, &filename, &dirname);
            return Module::declare(ctx.clone(), name, wrapped);
        }

        Module::declare(ctx.clone(), name, source)
    }
}

// ── Path resolution helpers ───────────────────────────────────────────────────

/// Resolve an ESM import specifier relative to a base file path.
/// Delegates to the CJS resolver for bare specifiers so exports field handling
/// is consistent between `import 'pkg'` and `require('pkg')`.
pub fn resolve_esm(base: &str, specifier: &str) -> PathBuf {
    if specifier.starts_with("./") || specifier.starts_with("../") {
        let base_dir = Path::new(base).parent().unwrap_or(Path::new("."));
        resolve_relative(&base_dir.join(specifier))
    } else if specifier.starts_with('/') || Path::new(specifier).is_absolute() {
        resolve_relative(&PathBuf::from(specifier))
    } else {
        let base_dir = Path::new(base)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let base_str = base_dir.to_string_lossy().to_string();
        match modules::resolve_path_from_esm(specifier, Some(&base_str)) {
            Ok(p) => p,
            Err(_msg) => resolve_node_module_esm(&base_dir, specifier),
        }
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
fn resolve_exports_root(json: &serde_json::Value, pkg_dir: &Path) -> Option<PathBuf> {
    let exports = &json["exports"];
    if exports.is_null() {
        return None;
    }
    let entry_str = if let Some(s) = exports.as_str() {
        s.to_string()
    } else if let Some(dot) = exports.get(".") {
        resolve_condition(dot)?
    } else {
        resolve_condition(exports)?
    };
    let path = resolve_relative(&pkg_dir.join(entry_str.trim_start_matches("./")));
    if path.is_file() { Some(path) } else { None }
}

/// Pick a string value from a conditional exports object.
fn resolve_condition(val: &serde_json::Value) -> Option<String> {
    if let Some(s) = val.as_str() {
        return Some(s.to_string());
    }
    for key in &["import", "module", "default"] {
        if let Some(s) = val[key].as_str() {
            return Some(s.to_string());
        }
        if val[key].is_object()
            && let Some(s) = resolve_condition(&val[key])
        {
            return Some(s);
        }
    }
    None
}

/// Fallback ESM resolver for bare specifiers.
fn resolve_node_module_esm(start_dir: &Path, name: &str) -> PathBuf {
    let (pkg_name, subpath) = modules::split_bare_specifier(name);
    let subpath = subpath.map(|s| s.to_string());

    let pkg_dir = find_in_node_modules(start_dir, pkg_name)
        .unwrap_or_else(|| start_dir.join("node_modules").join(pkg_name));

    if !pkg_dir.is_dir() {
        return if let Some(ref sub) = subpath {
            pkg_dir.join(sub)
        } else {
            pkg_dir
        };
    }

    let pkg_json = pkg_dir.join("package.json");
    let json = std::fs::read_to_string(&pkg_json)
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok());

    if let Some(ref sub) = subpath {
        if let Some(ref j) = json
            && let Some(exports) = j.get("exports")
        {
            let export_key = if sub.starts_with("./") {
                sub.clone()
            } else {
                format!("./{}", sub)
            };
            if let Some(val) = exports.get(&export_key)
                && let Some(path) = modules::resolve_exports_value(val, &pkg_dir, true)
            {
                return path;
            }
            if exports.is_object()
                && let Some(Some(result)) = modules::resolve_exports_pattern(
                    exports.as_object().unwrap(),
                    &export_key,
                    &pkg_dir,
                )
            {
                return result;
            }
        }
        let p = pkg_dir.join(sub);
        let resolved = resolve_relative(&p);
        if resolved.is_file() {
            return resolved;
        }
        if p.is_dir() {
            let entry = resolve_node_module_entry(&p);
            if entry.is_file() {
                return entry;
            }
        }
        return p;
    }

    if let Some(ref j) = json {
        let exports_entry = resolve_exports_root(j, &pkg_dir);
        if let Some(path) = exports_entry {
            return path;
        }
        if let Some(rn) = j.get("react-native").and_then(|v| v.as_str()) {
            let path = resolve_relative(&pkg_dir.join(rn));
            if path.is_file() {
                return path;
            }
        }
        if let Some(mod_entry) = j.get("module").and_then(|v| v.as_str()) {
            let path = resolve_relative(&pkg_dir.join(mod_entry));
            if path.is_file() {
                return path;
            }
        }
        if let Some(main) = j.get("main").and_then(|v| v.as_str()) {
            let path = resolve_relative(&pkg_dir.join(main));
            if path.is_file() {
                return path;
            }
        }
        if let Some(browser) = j.get("browser").and_then(|v| v.as_str()) {
            let path = resolve_relative(&pkg_dir.join(browser));
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
}

fn resolve_node_module_entry(pkg_dir: &Path) -> PathBuf {
    let content = std::fs::read_to_string(pkg_dir.join("package.json")).ok();
    let json = content.and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok());
    if let Some(ref j) = json
        && let Some(main) = j.get("main").and_then(|v| v.as_str())
    {
        let path = resolve_relative(&pkg_dir.join(main));
        if path.is_file() {
            return path;
        }
    }
    let idx = pkg_dir.join("index.js");
    if idx.is_file() {
        return idx;
    }
    let idx_ts = pkg_dir.join("index.ts");
    if idx_ts.is_file() {
        return idx_ts;
    }
    pkg_dir.to_path_buf()
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

#[cfg(test)]
mod tests {
    use super::source_is_esm;

    // ── Bug 4: source_is_esm must use extension first ─────────────────────────

    #[test]
    fn mjs_is_always_esm_regardless_of_content() {
        assert!(source_is_esm("module.exports = 42;", "lib/index.mjs"));
    }

    #[test]
    fn cjs_is_never_esm_regardless_of_content() {
        // Even if the file somehow contains "import" it's still CJS by extension.
        assert!(!source_is_esm(
            "const x = require('import');",
            "lib/index.cjs"
        ));
    }

    #[test]
    fn js_with_top_level_import_is_esm() {
        assert!(source_is_esm("import foo from 'bar';\n", "index.js"));
    }

    #[test]
    fn js_with_top_level_export_is_esm() {
        assert!(source_is_esm("export default 42;\n", "index.js"));
    }

    #[test]
    fn js_cjs_module_is_not_esm() {
        let src = "'use strict';\nconst x = require('y');\nmodule.exports = x;\n";
        assert!(!source_is_esm(src, "index.js"));
    }

    #[test]
    fn import_inside_function_body_is_not_top_level_esm() {
        // "break on first real line" heuristic: the first non-comment line is a
        // function declaration, not an import → should return false.
        let src = "function load() { return import('x'); }\nmodule.exports = load;\n";
        assert!(!source_is_esm(src, "index.js"));
    }

    #[test]
    fn import_keyword_in_line_comment_not_esm() {
        let src = "// import foo from 'bar'\nmodule.exports = {};\n";
        assert!(!source_is_esm(src, "index.js"));
    }

    #[test]
    fn import_keyword_in_block_comment_not_esm() {
        let src = "/* import foo from 'bar' */\nmodule.exports = {};\n";
        assert!(!source_is_esm(src, "index.js"));
    }

    #[test]
    fn export_brace_form_is_esm() {
        assert!(source_is_esm("export{foo,bar};\n", "index.js"));
    }
}
