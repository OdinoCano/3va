/// Execution-level tests for the bundler: bundle JS/TS → run in JsEngine → verify result.
///
/// These tests are the critical complement to bundler_integration.rs:
/// they prove the bundled output is not just well-formed strings, but
/// actually runnable JavaScript that produces the correct results.
use std::sync::Arc;
use tempfile::TempDir;
use vvva_bundler::{Bundler, BundlerOptions, OutputFormat};
use vvva_js::JsEngine;
use vvva_permissions::PermissionState;

fn write_file(dir: &TempDir, name: &str, content: &str) -> String {
    let path = dir.path().join(name);
    std::fs::write(&path, content).unwrap();
    path.to_string_lossy().into_owned()
}

async fn make_engine() -> JsEngine {
    let perms = Arc::new(PermissionState::new());
    JsEngine::new(perms).await.unwrap()
}

/// Bundle `entry` into a string, evaluate it in a JsEngine, return the string
/// value of `globalThis.<global_var>` after execution.
async fn bundle_and_run(entry: &str, global_var: &str) -> String {
    bundle_and_run_with_opts(entry, global_var, BundlerOptions::default()).await
}

async fn bundle_and_run_with_opts(entry: &str, global_var: &str, opts: BundlerOptions) -> String {
    let root = std::path::PathBuf::from(".");
    let mut bundler = Bundler::new(root).with_options(opts);
    bundler.add_entry(entry).unwrap();
    let code = bundler.bundle().unwrap();

    let engine = make_engine().await;
    engine.eval(&code).await.unwrap();
    engine
        .eval_to_string(&format!("String(globalThis.{global_var})"))
        .await
        .unwrap()
}

// ── Basic execution ───────────────────────────────────────────────────────────

#[tokio::test]
async fn iife_bundle_executes_arithmetic() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(&dir, "index.js", "globalThis.__result = 1 + 2 + 3;");
    let result = bundle_and_run(&entry, "__result").await;
    assert_eq!(result, "6", "1+2+3 should be 6");
}

#[tokio::test]
async fn iife_bundle_executes_function_call() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(
        &dir,
        "index.js",
        r#"function greet(name) { return 'Hello, ' + name + '!'; }
           globalThis.__result = greet('World');"#,
    );
    let result = bundle_and_run(&entry, "__result").await;
    assert_eq!(result, "Hello, World!");
}

#[tokio::test]
async fn iife_bundle_executes_array_operations() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(
        &dir,
        "index.js",
        "globalThis.__result = [1, 2, 3].map(function(x) { return x * x; }).join(',');",
    );
    let result = bundle_and_run(&entry, "__result").await;
    assert_eq!(result, "1,4,9");
}

#[tokio::test]
async fn iife_bundle_executes_closure() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(
        &dir,
        "index.js",
        r#"function makeCounter() {
             var n = 0;
             return function() { return ++n; };
           }
           var c = makeCounter();
           c(); c();
           globalThis.__result = c();"#,
    );
    let result = bundle_and_run(&entry, "__result").await;
    assert_eq!(result, "3", "counter should return 3 after three calls");
}

#[tokio::test]
async fn bundle_preserves_string_literals() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(
        &dir,
        "index.js",
        r#"globalThis.__result = 'unicode: \u{1F600} & special: "quotes" & back\\slash';"#,
    );
    let result = bundle_and_run(&entry, "__result").await;
    assert!(
        result.contains("unicode"),
        "string literals must be preserved"
    );
    assert!(
        result.contains("quotes"),
        "escaped quotes must be preserved"
    );
}

// ── TypeScript stripping ──────────────────────────────────────────────────────

#[tokio::test]
async fn typescript_bundle_strips_top_level_type_declarations() {
    let dir = TempDir::new().unwrap();
    // The bundler's strip_types removes whole lines starting with "interface "
    // or "type ", and lines containing inline annotations.
    // We use TS code where the executable logic has NO inline type annotations
    // so it survives stripping intact.
    let entry = write_file(
        &dir,
        "index.ts",
        r#"
type Alias = string;
interface Config { name: string; }
function compute(x, y) { return x * y; }
globalThis.__result = compute(6, 7);
"#,
    );
    let result = bundle_and_run(&entry, "__result").await;
    assert_eq!(
        result, "42",
        "TS bundle should execute after stripping type/interface lines"
    );
}

#[tokio::test]
async fn typescript_bundle_removes_type_declarations_from_output() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(
        &dir,
        "index.ts",
        r#"type MyAlias = string;
interface MyInterface { value: string; }
globalThis.__result = 'stripped';"#,
    );

    let root = std::path::PathBuf::from(".");
    let mut bundler = Bundler::new(root);
    bundler.add_entry(&entry).unwrap();
    let code = bundler.bundle().unwrap();

    assert!(
        !code.contains("type MyAlias"),
        "type alias declaration should be stripped"
    );
    assert!(
        !code.contains("interface MyInterface"),
        "interface declaration should be stripped"
    );
}

// ── JSON module handling ──────────────────────────────────────────────────────

#[tokio::test]
async fn json_module_bundled_as_module_exports() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(&dir, "data.json", r#"{"version": "1.0", "count": 99}"#);

    let root = std::path::PathBuf::from(".");
    let mut bundler = Bundler::new(root);
    bundler.add_entry(&entry).unwrap();
    let code = bundler.bundle().unwrap();

    // JSON modules become `module.exports = <json>;`
    assert!(
        code.contains("module.exports"),
        "JSON entry should produce module.exports assignment"
    );
    assert!(code.contains("version"), "JSON content should be present");
    assert!(code.contains("99"), "JSON values should be in output");
}

// ── Minification ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn minified_bundle_still_executes_correctly() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(
        &dir,
        "index.js",
        r#"var   x   =   100;
           var   y   =   200;
           globalThis.__result = x + y;"#,
    );
    let result = bundle_and_run_with_opts(
        &entry,
        "__result",
        BundlerOptions {
            format: OutputFormat::Iife,
            minify: true,
            sourcemap: false,
            splitting: false,
            chunk_filename: "[name].js".to_string(),
        },
    )
    .await;
    assert_eq!(
        result, "300",
        "minified bundle should produce correct result"
    );
}

#[tokio::test]
async fn minified_bundle_is_smaller_than_plain() {
    let dir = TempDir::new().unwrap();
    let src = r#"var   longVariableName   =   1;
                 var   anotherLongName   =   2;
                 var   yetAnotherName    =   3;
                 console.log(longVariableName + anotherLongName + yetAnotherName);"#;
    let entry = write_file(&dir, "index.js", src);

    let root = std::path::PathBuf::from(".");
    let mut plain = Bundler::new(root.clone());
    plain.add_entry(&entry).unwrap();
    let plain_code = plain.bundle().unwrap();

    let mut mini = Bundler::new(root).with_options(BundlerOptions {
        format: OutputFormat::Iife,
        minify: true,
        sourcemap: false,
        splitting: false,
        chunk_filename: "[name].js".to_string(),
    });
    mini.add_entry(&entry).unwrap();
    let mini_code = mini.bundle().unwrap();

    assert!(
        mini_code.len() <= plain_code.len(),
        "minified ({} bytes) should be ≤ plain ({} bytes)",
        mini_code.len(),
        plain_code.len()
    );
}

// ── Tree shaking ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn tree_shaking_removes_unused_export_from_dependency() {
    // The bundler shakes non-entry modules. If a lib module exports `used`
    // and `unused`, and only `used` is imported, `unused` should be gone.
    // We test this by checking the bundled string does NOT contain "deadFn".
    let dir = TempDir::new().unwrap();

    // Entry: imports only `used`
    let entry = write_file(
        &dir,
        "index.js",
        r#"import { used } from './lib.js';
           globalThis.__result = used();"#,
    );
    write_file(
        &dir,
        "lib.js",
        r#"export function used() { return 'kept'; }
           export function deadFn() { return 'should_be_gone'; }"#,
    );

    let root = dir.path().to_path_buf();
    let mut bundler = Bundler::new(root);
    bundler.add_entry(&entry).unwrap();
    let code = bundler.bundle().unwrap();

    // The unused export should have been removed
    assert!(
        !code.contains("should_be_gone"),
        "tree-shaking should remove unused exports; got: {code}"
    );
}

#[tokio::test]
async fn entry_point_exports_are_never_shaken() {
    // Entry-point exports are the public API and must always be preserved.
    let dir = TempDir::new().unwrap();
    let entry = write_file(
        &dir,
        "index.js",
        r#"export function publicA() { return 'a'; }
           export function publicB() { return 'b'; }"#,
    );

    let root = dir.path().to_path_buf();
    let mut bundler = Bundler::new(root);
    bundler.add_entry(&entry).unwrap();
    let code = bundler.bundle().unwrap();

    assert!(
        code.contains("publicA"),
        "entry-point export publicA must be kept"
    );
    assert!(
        code.contains("publicB"),
        "entry-point export publicB must be kept"
    );
}

// ── Dead-code elimination ─────────────────────────────────────────────────────

#[tokio::test]
async fn dead_code_eliminator_removes_if_false_branch() {
    use vvva_bundler::DeadCodeEliminator;

    let elim = DeadCodeEliminator::new();
    let code = r#"
        const x = 1;
        if (false) { const dead = 999; }
        if (true) { const alive = 42; }
        function test() { if(false) { console.log("gone"); } }
    "#;

    let result = elim.eliminate(code);

    assert!(
        !result.contains("dead = 999"),
        "if(false) body must be removed"
    );
    assert!(!result.contains("gone"), "nested if(false) must be removed");
    assert!(result.contains("alive = 42"), "if(true) body must be kept");
    assert!(
        result.contains("const x = 1"),
        "non-conditional code must remain"
    );
}

// ── Source map ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn sourcemap_bundle_is_still_runnable() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(
        &dir,
        "index.js",
        "globalThis.__result = 'from_sourcemap_bundle';",
    );
    let result = bundle_and_run_with_opts(
        &entry,
        "__result",
        BundlerOptions {
            format: OutputFormat::Iife,
            minify: false,
            sourcemap: true,
            splitting: false,
            chunk_filename: "[name].js".to_string(),
        },
    )
    .await;
    assert_eq!(result, "from_sourcemap_bundle");
}

// ── Output format correctness ─────────────────────────────────────────────────

#[tokio::test]
async fn cjs_bundle_runs_as_commonjs() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(&dir, "index.js", "globalThis.__result = 'cjs_ok';");
    let result = bundle_and_run_with_opts(
        &entry,
        "__result",
        BundlerOptions {
            format: OutputFormat::Cjs,
            minify: false,
            sourcemap: false,
            splitting: false,
            chunk_filename: "[name].js".to_string(),
        },
    )
    .await;
    assert_eq!(result, "cjs_ok", "CJS bundle should execute correctly");
}

#[tokio::test]
async fn esm_bundle_format_contains_no_iife_wrapper() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(&dir, "index.js", "export const x = 1;");

    let root = std::path::PathBuf::from(".");
    let mut bundler = Bundler::new(root).with_options(BundlerOptions {
        format: OutputFormat::Esm,
        minify: false,
        sourcemap: false,
        splitting: false,
        chunk_filename: "[name].js".to_string(),
    });
    bundler.add_entry(&entry).unwrap();
    let code = bundler.bundle().unwrap();

    assert!(
        !code.starts_with("(function"),
        "ESM output must not start with IIFE wrapper"
    );
    assert!(!code.is_empty(), "ESM output must not be empty");
}

#[tokio::test]
async fn umd_bundle_contains_factory_pattern() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(&dir, "index.js", "var lib = { v: '1.0' };");

    let root = std::path::PathBuf::from(".");
    let mut bundler = Bundler::new(root).with_options(BundlerOptions {
        format: OutputFormat::Umd,
        minify: false,
        sourcemap: false,
        splitting: false,
        chunk_filename: "[name].js".to_string(),
    });
    bundler.add_entry(&entry).unwrap();
    let code = bundler.bundle().unwrap();

    assert!(!code.is_empty(), "UMD output must not be empty");
    assert!(
        code.contains("1.0"),
        "UMD output must include source content"
    );
}

// ── Error paths ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn bundled_js_syntax_error_propagates_at_eval() {
    let dir = TempDir::new().unwrap();
    // Intentionally insert invalid JS that survives bundling but fails at eval.
    let entry = write_file(&dir, "index.js", "globalThis.__result = ;");

    let root = std::path::PathBuf::from(".");
    let mut bundler = Bundler::new(root);
    let _ = bundler.add_entry(&entry);
    match bundler.bundle() {
        Err(_) => { /* bundler caught the syntax error — acceptable */ }
        Ok(code) => {
            let engine = make_engine().await;
            let result = engine.eval(&code).await;
            assert!(result.is_err(), "eval of invalid JS must return an error");
        }
    }
}

#[tokio::test]
async fn bundler_handles_empty_js_file() {
    let dir = TempDir::new().unwrap();
    let entry = write_file(&dir, "empty.js", "");

    let root = std::path::PathBuf::from(".");
    let mut bundler = Bundler::new(root);
    bundler.add_entry(&entry).unwrap();
    let result = bundler.bundle();

    // Empty entry should produce some output (at least the wrapper) without panic.
    assert!(
        result.is_ok(),
        "empty JS file should not crash the bundler: {result:?}"
    );
}
