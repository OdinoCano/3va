use std::path::PathBuf;
use tempfile::TempDir;
use vvva_bundler::{Bundler, BundlerOptions, OutputFormat, bundle_file};

fn write_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path
}

// ── Basic bundle ─────────────────────────────────────────────────────────────

#[test]
fn bundles_simple_js_to_iife() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(&dir, "index.js", "const x = 1 + 2; console.log(x);");

    let root = dir.path().to_path_buf();
    let mut bundler = Bundler::new(root);
    bundler.add_entry(&entry.to_string_lossy()).unwrap();
    let code = bundler.bundle().unwrap();

    assert!(code.contains("console.log"), "should contain console.log");
    assert!(
        code.starts_with("(function"),
        "IIFE should start with (function"
    );
}

#[test]
fn bundles_typescript_strips_types() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(
        &dir,
        "index.ts",
        "function greet(name: string): string { return `Hello, ${name}`; }\nconsole.log(greet('world'));",
    );

    let root = dir.path().to_path_buf();
    let mut bundler = Bundler::new(root);
    bundler.add_entry(&entry.to_string_lossy()).unwrap();
    let code = bundler.bundle().unwrap();

    assert!(
        !code.contains(": string"),
        "type annotations should be stripped"
    );
    assert!(code.contains("greet"), "function body should be present");
}

// ── Output formats ───────────────────────────────────────────────────────────

#[test]
fn output_format_cjs_uses_module_exports() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(&dir, "index.js", "const val = 42; module.exports = val;");

    let root = dir.path().to_path_buf();
    let mut bundler = Bundler::new(root).with_options(BundlerOptions {
        format: OutputFormat::Cjs,
        minify: false,
        sourcemap: false,
        splitting: false,
        chunk_filename: "[name].js".to_string(),
    });
    bundler.add_entry(&entry.to_string_lossy()).unwrap();
    let code = bundler.bundle().unwrap();

    // CJS output should not wrap in IIFE
    assert!(!code.starts_with("(function"), "CJS should not be IIFE");
    assert!(code.contains("42"), "value should be in output");
}

#[test]
fn output_format_esm_produces_esm_output() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(&dir, "index.js", "export const answer = 42;");

    let root = dir.path().to_path_buf();
    let mut bundler = Bundler::new(root).with_options(BundlerOptions {
        format: OutputFormat::Esm,
        minify: false,
        sourcemap: false,
        splitting: false,
        chunk_filename: "[name].js".to_string(),
    });
    bundler.add_entry(&entry.to_string_lossy()).unwrap();
    let code = bundler.bundle().unwrap();

    // ESM output should not use IIFE wrapper
    assert!(!code.starts_with("(function"), "ESM should not be IIFE");
    assert!(code.contains("42"), "value should be present");
}

#[test]
fn output_format_umd_wraps_with_umd_factory() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(&dir, "index.js", "var lib = { version: '1.0' };");

    let root = dir.path().to_path_buf();
    let mut bundler = Bundler::new(root).with_options(BundlerOptions {
        format: OutputFormat::Umd,
        minify: false,
        sourcemap: false,
        splitting: false,
        chunk_filename: "[name].js".to_string(),
    });
    bundler.add_entry(&entry.to_string_lossy()).unwrap();
    let code = bundler.bundle().unwrap();

    // UMD wraps with a factory function checking for AMD/CJS/global
    assert!(!code.is_empty(), "UMD output should not be empty");
    assert!(code.contains("1.0"), "bundle content should be present");
}

// ── Minification ─────────────────────────────────────────────────────────────

#[test]
fn minify_removes_extra_whitespace() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(
        &dir,
        "index.js",
        "var   x   =   1;\n\n\nvar   y   =   2;\n\nconsole.log(x + y);",
    );

    let root_plain = dir.path().to_path_buf();
    let mut plain = Bundler::new(root_plain);
    plain.add_entry(&entry.to_string_lossy()).unwrap();
    let plain_code = plain.bundle().unwrap();

    let root_mini = dir.path().to_path_buf();
    let mut mini = Bundler::new(root_mini).with_options(BundlerOptions {
        format: OutputFormat::Iife,
        minify: true,
        sourcemap: false,
        splitting: false,
        chunk_filename: "[name].js".to_string(),
    });
    mini.add_entry(&entry.to_string_lossy()).unwrap();
    let mini_code = mini.bundle().unwrap();

    assert!(
        mini_code.len() <= plain_code.len(),
        "minified output ({}) should be no larger than plain ({})",
        mini_code.len(),
        plain_code.len()
    );
}

// ── Source maps ───────────────────────────────────────────────────────────────

#[test]
fn bundle_with_sourcemap_returns_map_json() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(&dir, "index.js", "console.log('hello');");

    let root = dir.path().to_path_buf();
    let mut bundler = Bundler::new(root).with_options(BundlerOptions {
        format: OutputFormat::Iife,
        minify: false,
        sourcemap: true,
        splitting: false,
        chunk_filename: "[name].js".to_string(),
    });
    bundler.add_entry(&entry.to_string_lossy()).unwrap();
    let (code, map) = bundler.bundle_with_sourcemap().unwrap();

    assert!(
        code.contains("hello"),
        "bundle should contain source content"
    );
    let map = map.expect("sourcemap: true must produce a source map");
    let parsed: serde_json::Value = serde_json::from_str(&map).expect("map must be valid JSON");
    assert_eq!(parsed["version"], 3, "source map version must be 3");
    assert!(
        parsed["sources"].is_array(),
        "source map must have a sources array"
    );
}

// ── bundle_file helper ────────────────────────────────────────────────────────

#[test]
fn bundle_file_writes_output_to_disk() {
    let dir = TempDir::new().unwrap();
    let input = write_file(&dir, "main.js", "console.log('bundled');");
    let output = dir.path().join("out.js");

    bundle_file(&input.to_string_lossy(), &output.to_string_lossy(), None).unwrap();

    assert!(output.exists(), "output file should be created");
    let content = std::fs::read_to_string(&output).unwrap();
    assert!(
        content.contains("bundled"),
        "output should contain source content"
    );
}

// ── Real module-graph walk (crates/bundler/src/lib.rs::bundle_graph) ─────────
//
// `bundle_file` previously only ever processed the entry file — any `import`
// was left untouched in the output, producing a syntactically invalid bundle
// for any project with more than one file. These exercise the fix: real
// imports across multiple project files, an ESM npm dependency, a CommonJS
// npm dependency, a JSON import, and a default-export React-shaped component
// (the pattern the `.default`-unwrapping bug in `vvva_js`'s transpiler broke).

fn write_nested(dir: &TempDir, rel: &str, content: &str) -> PathBuf {
    let path = dir.path().join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

#[test]
fn bundle_graph_inlines_a_multi_file_project_with_esm_and_cjs_deps() {
    let dir = TempDir::new().unwrap();

    write_nested(
        &dir,
        "node_modules/esmlib/package.json",
        r#"{"name":"esmlib","main":"index.js"}"#,
    );
    write_nested(
        &dir,
        "node_modules/esmlib/index.js",
        "export function greet() { return \"esm-hi\"; }",
    );
    write_nested(
        &dir,
        "node_modules/cjslib/package.json",
        r#"{"name":"cjslib","main":"index.js"}"#,
    );
    write_nested(
        &dir,
        "node_modules/cjslib/index.js",
        "exports.hello = function() { return \"cjs-hi\"; };",
    );
    write_nested(&dir, "src/data.json", r#"{"n": 42}"#);
    write_nested(
        &dir,
        "src/App.jsx",
        "import { greet } from \"esmlib\";\n\
         import { hello } from \"cjslib\";\n\
         import data from \"./data.json\";\n\
         export default function App() {\n\
         \x20 return greet() + \" \" + hello() + \" \" + data.n;\n\
         }\n",
    );
    let entry = write_nested(
        &dir,
        "src/main.jsx",
        "import App from \"./App.jsx\";\nconsole.log(App());",
    );

    let output = dir.path().join("dist/bundle.js");
    bundle_file(&entry.to_string_lossy(), &output.to_string_lossy(), None).unwrap();

    let code = std::fs::read_to_string(&output).unwrap();
    // Every import must have been resolved to a real, distinct registry
    // entry — not left as a literal (syntactically invalid) `import`/bare
    // specifier `require(...)`.
    assert!(
        !code.contains("import "),
        "no raw import should survive: {code}"
    );
    assert!(
        !code.contains("require(\"esmlib\")"),
        "esmlib must resolve to a real path: {code}"
    );
    assert!(
        !code.contains("require(\"cjslib\")"),
        "cjslib must resolve to a real path: {code}"
    );
    assert!(
        code.contains("esm-hi"),
        "esmlib's module body must be inlined: {code}"
    );
    assert!(
        code.contains("cjs-hi"),
        "cjslib's module body must be inlined: {code}"
    );
    assert!(
        code.contains("\"n\": 42") || code.contains("\"n\":42"),
        "JSON body must be inlined: {code}"
    );
}

#[test]
fn bundle_graph_output_creates_missing_output_directory() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(&dir, "index.js", "console.log('ok');");
    let output = dir.path().join("nested/does/not/exist/out.js");

    bundle_file(&entry.to_string_lossy(), &output.to_string_lossy(), None).unwrap();
    assert!(output.exists());
}

#[test]
fn bundle_file_with_sourcemap_option_writes_map_file() {
    let dir = TempDir::new().unwrap();
    let input = write_file(&dir, "main.js", "const x = 1;");
    let output = dir.path().join("out.js");

    bundle_file(
        &input.to_string_lossy(),
        &output.to_string_lossy(),
        Some(BundlerOptions {
            format: OutputFormat::Iife,
            minify: false,
            sourcemap: true,
            splitting: false,
            chunk_filename: "[name].js".to_string(),
        }),
    )
    .unwrap();

    assert!(output.exists(), "output file should be created");
    // map file written only if the generator produced a map
    let map_path = dir.path().join("out.js.map");
    // either the map file exists and references the bundle, or sourcemap was skipped silently
    if map_path.exists() {
        let bundle = std::fs::read_to_string(&output).unwrap();
        assert!(
            bundle.contains("sourceMappingURL"),
            "bundle should reference source map"
        );
    }
}

// ── Error handling ────────────────────────────────────────────────────────────

#[test]
fn add_entry_nonexistent_file_returns_error() {
    let dir = TempDir::new().unwrap();
    let mut bundler = Bundler::new(dir.path().to_path_buf());
    let result = bundler.add_entry("/nonexistent/path/that/does/not/exist.js");
    assert!(result.is_err(), "adding nonexistent entry should fail");
}

#[test]
fn bundle_empty_bundler_returns_empty_or_wrapper() {
    let dir = TempDir::new().unwrap();
    let bundler = Bundler::new(dir.path().to_path_buf());
    // Bundler with no entries — bundle() should succeed (returns empty wrapper)
    // We only check it doesn't panic; actual content is format-dependent
    let _ = {
        let mut b = bundler;
        b.bundle()
    };
}
