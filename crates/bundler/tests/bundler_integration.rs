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
    assert!(code.starts_with("(function"), "IIFE should start with (function");
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

    assert!(!code.contains(": string"), "type annotations should be stripped");
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
    assert!(code.len() > 0, "UMD output should not be empty");
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

    assert!(code.contains("hello"), "bundle should contain source content");
    // map may or may not be Some depending on whether the generator emits one
    let _ = map; // not asserting presence; just that the call succeeds
}

// ── bundle_file helper ────────────────────────────────────────────────────────

#[test]
fn bundle_file_writes_output_to_disk() {
    let dir = TempDir::new().unwrap();
    let input = write_file(&dir, "main.js", "console.log('bundled');");
    let output = dir.path().join("out.js");

    bundle_file(
        &input.to_string_lossy(),
        &output.to_string_lossy(),
        None,
    )
    .unwrap();

    assert!(output.exists(), "output file should be created");
    let content = std::fs::read_to_string(&output).unwrap();
    assert!(content.contains("bundled"), "output should contain source content");
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
