use std::path::{Path, PathBuf};

use crate::builtins::modules;

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

pub fn source_is_esm(code: &str, path: &str) -> bool {
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
            || has_dynamic_import(t)
        {
            return true;
        }
    }
    false
}

/// True if `line` contains a bare `import(` call (dynamic import) not
/// preceded by an identifier character — catches `await import(...)`,
/// `const x = import(...)`, etc., not just the static `import`/`export`
/// declarations the line-prefix checks above catch. Without this, a file
/// using only dynamic import (no static import/export) never gets routed
/// through transpile_to_cjs, and the transpiler's `import(` → `__importAsync(`
/// rewrite (which only runs on the ESM path) never fires.
fn has_dynamic_import(line: &str) -> bool {
    if let Some(pos) = line.find("import(") {
        let before_ok = pos == 0 || {
            let prev = line.as_bytes()[pos - 1];
            !(prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$')
        };
        if before_ok {
            return true;
        }
    }
    false
}

pub fn resolve_esm(base: &str, specifier: &str) -> PathBuf {
    let base_dir = Path::new(base)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    resolve_esm_from_dir(&base_dir.to_string_lossy(), specifier)
}

/// Same resolution algorithm as [`resolve_esm`], but `dir` is already a
/// directory (not a file whose parent needs to be taken first). Used by
/// `require()`, which tracks the current module's directory directly.
pub fn resolve_esm_from_dir(dir: &str, specifier: &str) -> PathBuf {
    if specifier.starts_with("./") || specifier.starts_with("../") {
        resolve_relative(&Path::new(dir).join(specifier))
    } else if specifier.starts_with('/') || Path::new(specifier).is_absolute() {
        resolve_relative(&PathBuf::from(specifier))
    } else {
        match modules::resolve_path_from_esm(specifier, Some(dir)) {
            Ok(p) => p,
            Err(_msg) => resolve_node_module_esm(Path::new(dir), specifier),
        }
    }
}

fn resolve_relative(base: &Path) -> PathBuf {
    if base.is_file() {
        return base.to_path_buf();
    }
    for ext in &["js", "mjs", "ts", "tsx", "jsx", "cjs"] {
        let p = base.with_extension(ext);
        if p.is_file() {
            return p;
        }
    }
    for index in &[
        "index.js",
        "index.mjs",
        "index.ts",
        "index.tsx",
        "index.jsx",
    ] {
        let p = base.join(index);
        if p.is_file() {
            return p;
        }
    }
    base.to_path_buf()
}

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

#[cfg(test)]
mod tests {
    use super::{resolve_esm, source_is_esm};

    #[test]
    fn resolve_esm_finds_jsx_for_extensionless_relative_import() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("App.jsx"),
            "export default function App() {}",
        )
        .unwrap();
        let base = dir.path().join("main.jsx");
        let resolved = resolve_esm(&base.to_string_lossy(), "./App");
        assert_eq!(resolved, dir.path().join("App.jsx"));
    }

    #[test]
    fn resolve_esm_finds_index_jsx_for_extensionless_directory_import() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("components")).unwrap();
        std::fs::write(
            dir.path().join("components/index.jsx"),
            "export default function C() {}",
        )
        .unwrap();
        let base = dir.path().join("main.jsx");
        let resolved = resolve_esm(&base.to_string_lossy(), "./components");
        assert_eq!(resolved, dir.path().join("components/index.jsx"));
    }

    #[test]
    fn mjs_is_always_esm_regardless_of_content() {
        assert!(source_is_esm("module.exports = 42;", "lib/index.mjs"));
    }

    #[test]
    fn cjs_is_never_esm_regardless_of_content() {
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
    fn dynamic_import_inside_function_body_still_needs_esm_transpile() {
        // Unlike static import/export (spec-required to be top-level),
        // dynamic import() is valid anywhere an expression is — but it still
        // must route through transpile_to_cjs so the `import(` ->
        // `__importAsync(` rewrite fires; otherwise it reaches V8 as
        // unsupported native dynamic import and throws.
        let src = "function load() { return import('x'); }\nmodule.exports = load;\n";
        assert!(source_is_esm(src, "index.js"));
    }

    #[test]
    fn identifier_ending_in_import_is_not_treated_as_dynamic_import() {
        // "doimport(" contains the substring "import(" but is preceded by
        // an identifier char ('o'), so the boundary check in
        // has_dynamic_import must reject it.
        let src = "function doimport(x) { return x; }\nmodule.exports = doimport;\n";
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
