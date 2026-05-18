use rquickjs::{Ctx, Function, Result, function::Rest};
use std::rc::Rc;
use std::cell::RefCell;
use vvva_permissions::{PermissionState, Capability};
use std::path::PathBuf;

/// Inject CommonJS `require()`, `module`, `exports`, `__filename`, `__dirname` globals.
///
/// Strategy: inject a native `__readFile(path) -> String` function that handles
/// permission checks and file I/O. The JS-side `require()` wrapper handles the
/// module caching, wrapping, and evaluation — avoiding rquickjs `Value<'js>` lifetime
/// issues in closures.
pub fn inject_require(ctx: &Ctx, permissions: Rc<RefCell<PermissionState>>) -> Result<()> {
    let globals = ctx.globals();

    // Initialize module cache and CommonJS globals
    ctx.eval::<(), _>(r#"
        globalThis.__requireCache = {};
        globalThis.module = { exports: {} };
        globalThis.exports = globalThis.module.exports;
        globalThis.__filename = '';
        globalThis.__dirname = '';
    "#)?;

    // Native __readFile(path) -> String
    // Returns the (optionally transpiled) source, or throws on error.
    let perms = permissions.clone();
    let read_file_fn = Function::new(ctx.clone(), move |args: Rest<String>| -> Result<String> {
        let path_str = args.0.into_iter().next()
            .ok_or_else(|| rquickjs::Error::new_from_js("value", "__readFile() needs a path"))?;

        let full_path = resolve_path(&path_str);

        // Permission check
        {
            let p = perms.borrow();
            if !p.check(&Capability::FileRead(full_path.clone())) {
                // Return error indicator — JS side will throw
                return Err(rquickjs::Error::new_from_js(
                    "permission",
                    "permission denied",
                ));
            }
        }

        // Read the file
        let source = std::fs::read_to_string(&full_path).map_err(|_| {
            rquickjs::Error::new_from_js("io", "file not found")
        })?;

        // Transpile if TypeScript
        let source = if path_str.ends_with(".ts") || path_str.ends_with(".tsx") {
            crate::transpiler::transpile(&source)
        } else {
            source
        };

        Ok(source)
    })?;
    globals.set("__readFile", read_file_fn)?;

    // Native __resolvePath(path) -> String
    let resolve_fn = Function::new(ctx.clone(), |args: Rest<String>| -> String {
        let path_str = args.0.into_iter().next().unwrap_or_default();
        resolve_path(&path_str).to_string_lossy().to_string()
    })?;
    globals.set("__resolvePath", resolve_fn)?;

    // JS-level require() implementation
    // This avoids rquickjs Value<'js> lifetime issues by keeping all evaluation in JS.
    ctx.eval::<(), _>(r#"
        globalThis.require = function(path) {
            var resolvedPath = __resolvePath(path);

            // Check cache
            if (globalThis.__requireCache[resolvedPath] !== undefined) {
                return globalThis.__requireCache[resolvedPath];
            }

            // Read and (if needed) transpile the file
            var source = __readFile(path);

            // Compute dirname
            var dirname = resolvedPath.replace(/\/[^\/]*$/, '') || '.';
            var filename = resolvedPath;

            // Save and restore outer module state
            var savedModule = globalThis.module;
            var savedExports = globalThis.exports;
            var savedFilename = globalThis.__filename;
            var savedDirname = globalThis.__dirname;

            globalThis.module = { exports: {} };
            globalThis.exports = globalThis.module.exports;
            globalThis.__filename = filename;
            globalThis.__dirname = dirname;

            // Execute the module with CJS wrapper
            // We use eval() with the module wrapper
            var wrapper = '(function(exports, module, __filename, __dirname) {\n' +
                source +
                '\n})(globalThis.exports, globalThis.module, globalThis.__filename, globalThis.__dirname);';
            eval(wrapper);

            var result = globalThis.module.exports;

            // Restore outer state
            globalThis.module = savedModule;
            globalThis.exports = savedExports;
            globalThis.__filename = savedFilename;
            globalThis.__dirname = savedDirname;

            // Cache the result
            globalThis.__requireCache[resolvedPath] = result;

            return result;
        };
    "#)?;

    Ok(())
}

/// Resolve a module path relative to the current working directory.
fn resolve_path(path: &str) -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    if path.starts_with("./") || path.starts_with("../") || path.starts_with('/') {
        let base = if path.starts_with('/') {
            PathBuf::from(path)
        } else {
            cwd.join(path)
        };

        // Try exact path, then .js, then .ts, then index.js
        if base.exists() {
            return base;
        }
        let with_js = base.with_extension("js");
        if with_js.exists() {
            return with_js;
        }
        let with_ts = base.with_extension("ts");
        if with_ts.exists() {
            return with_ts;
        }
        let index_js = base.join("index.js");
        if index_js.exists() {
            return index_js;
        }
        base
    } else {
        // node_modules resolution (simplified)
        cwd.join("node_modules").join(path)
    }
}
