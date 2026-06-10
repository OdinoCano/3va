// Framework compatibility tests — import.meta.url/env/hot and related APIs.
//
// These are the foundation required for meta-frameworks (Astro, SvelteKit,
// Next.js, Remix, Nuxt) to run their production SSR output on 3va.
//
// Run: cargo test -p vvva_js --test framework_compat

use std::sync::Arc;
use tempfile::tempdir;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_with_read(path: &str) -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::FileRead(std::path::PathBuf::from(path)));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}

// ── import.meta.url ───────────────────────────────────────────────────────────

/// Entry-point files with `import.meta.url` must not crash and must return a
/// `file://` URL pointing to the script's own path.
#[tokio::test]
async fn import_meta_url_is_file_url_in_entry_point() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("entry.ts");
    std::fs::write(
        &script,
        r#"
import path from 'path';
globalThis.__test_meta_url = import.meta.url;
"#,
    )
    .unwrap();

    let e = engine_with_read(dir.path().to_str().unwrap()).await;
    e.eval_file(&script).await.unwrap();

    let url = e
        .eval_to_string("globalThis.__test_meta_url")
        .await
        .unwrap();
    assert!(
        url.starts_with("file://"),
        "expected file:// URL, got: {url}"
    );
    assert!(
        url.contains("entry"),
        "URL should contain the script name, got: {url}"
    );
}

/// `new URL('.', import.meta.url).pathname` — the canonical Vite/Astro pattern
/// for computing `__dirname` in ESM output.
#[tokio::test]
async fn import_meta_url_dirname_pattern() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("server.mjs");
    std::fs::write(
        &script,
        r#"
var dirUrl = new URL('.', import.meta.url);
globalThis.__test_dir_url = dirUrl.href;
"#,
    )
    .unwrap();

    let e = engine_with_read(dir.path().to_str().unwrap()).await;
    e.eval_file(&script).await.unwrap();

    let href = e.eval_to_string("globalThis.__test_dir_url").await.unwrap();
    assert!(
        href.starts_with("file://"),
        "expected file:// href, got: {href}"
    );
}

// ── import.meta.env ───────────────────────────────────────────────────────────

/// `import.meta.env.SSR` must be `true` (3va is always a server-side runtime).
#[tokio::test]
async fn import_meta_env_ssr_is_true() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("env_test.ts");
    std::fs::write(
        &script,
        r#"
import { readFileSync } from 'fs';
globalThis.__test_ssr = import.meta.env.SSR;
globalThis.__test_mode = import.meta.env.MODE;
"#,
    )
    .unwrap();

    let e = engine_with_read(dir.path().to_str().unwrap()).await;
    e.eval_file(&script).await.unwrap();

    let ssr = e
        .eval_to_string("String(globalThis.__test_ssr)")
        .await
        .unwrap();
    assert_eq!(ssr, "true", "import.meta.env.SSR should be true");

    let mode = e.eval_to_string("globalThis.__test_mode").await.unwrap();
    assert!(!mode.is_empty(), "import.meta.env.MODE should not be empty");
}

/// `import.meta.env.BASE_URL` must exist (used by Vite/Astro for asset paths).
#[tokio::test]
async fn import_meta_env_base_url_exists() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("base_url.ts");
    std::fs::write(
        &script,
        r#"
globalThis.__test_base = import.meta.env.BASE_URL;
"#,
    )
    .unwrap();

    let e = engine_with_read(dir.path().to_str().unwrap()).await;
    e.eval_file(&script).await.unwrap();

    let base = e.eval_to_string("globalThis.__test_base").await.unwrap();
    assert_eq!(base, "/", "BASE_URL should default to '/'");
}

// ── import.meta.hot ───────────────────────────────────────────────────────────

/// `import.meta.hot` must be `undefined` in 3va (no HMR server-side).
/// Frameworks guard HMR setup with `if (import.meta.hot)` — this must not crash.
#[tokio::test]
async fn import_meta_hot_is_undefined() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("hot.ts");
    std::fs::write(
        &script,
        r#"
// Typical Vite HMR guard pattern — must not crash on the server
if (import.meta.hot) {
  import.meta.hot.accept(() => {});
}
globalThis.__test_hot = typeof import.meta.hot;
"#,
    )
    .unwrap();

    let e = engine_with_read(dir.path().to_str().unwrap()).await;
    e.eval_file(&script).await.unwrap();

    let hot = e.eval_to_string("globalThis.__test_hot").await.unwrap();
    assert_eq!(hot, "undefined", "import.meta.hot should be undefined");
}

// ── import.meta in required modules ──────────────────────────────────────────

/// When a file loaded via `require()` uses `import.meta.url`, it should get
/// the URL of *that* file — not the entry point's URL.
#[tokio::test]
async fn import_meta_url_in_required_module_is_own_path() {
    let dir = tempdir().unwrap();

    // Module that exports its own URL
    let module_path = dir.path().join("util.js");
    std::fs::write(
        &module_path,
        r#"
import path from 'path';
export const myUrl = import.meta.url;
"#,
    )
    .unwrap();

    // Entry that requires the module
    let entry_path = dir.path().join("entry.ts");
    let module_path_str = module_path.to_str().unwrap().replace('\\', "/");
    std::fs::write(
        &entry_path,
        format!(
            r#"
const {{ myUrl }} = require({path:?});
globalThis.__test_module_url = myUrl;
globalThis.__test_entry_url = import.meta.url;
"#,
            path = module_path_str
        ),
    )
    .unwrap();

    let e = engine_with_read(dir.path().to_str().unwrap()).await;
    e.eval_file(&entry_path).await.unwrap();

    let module_url = e
        .eval_to_string("globalThis.__test_module_url")
        .await
        .unwrap();
    let entry_url = e
        .eval_to_string("globalThis.__test_entry_url")
        .await
        .unwrap();

    assert!(
        module_url.starts_with("file://"),
        "module URL should be file://, got: {module_url}"
    );
    assert!(
        entry_url.starts_with("file://"),
        "entry URL should be file://, got: {entry_url}"
    );
    assert!(
        module_url.contains("util"),
        "module URL should contain 'util', got: {module_url}"
    );
    assert!(
        entry_url.contains("entry"),
        "entry URL should contain 'entry', got: {entry_url}"
    );
    assert_ne!(module_url, entry_url, "each file should have its own URL");
}

// ── Typical framework boot pattern ────────────────────────────────────────────

/// Simulate the pattern used by Astro, Remix, and SvelteKit to bootstrap their
/// SSR server entry points.  The key idiom is:
///   `const __dirname = new URL('.', import.meta.url).pathname`
/// combined with real `import path from 'path'` and `import { existsSync } from 'fs'`
/// that are actually used — verifying the static import → require() conversion.
#[tokio::test]
async fn framework_bootstrap_pattern_does_not_crash() {
    let dir = tempdir().unwrap();
    let script = dir.path().join("server.ts");
    std::fs::write(
        &script,
        r#"
import path from 'path';
import { existsSync } from 'fs';

// Astro/SvelteKit/Remix bootstrap pattern
const __dirname_esm = new URL('.', import.meta.url).pathname;
const mode = import.meta.env.MODE;
const isProd = import.meta.env.PROD;
const isSSR = import.meta.env.SSR;

// HMR guard — must be a no-op on the server
if (import.meta.hot) {
  import.meta.hot.accept();
}

// Use the imported modules so they are NOT dead-code-eliminated —
// this verifies that static imports with real usages get converted.
const joined = path.join(__dirname_esm, 'sub');
const dirExists = existsSync(__dirname_esm);

globalThis.__test_boot = JSON.stringify({
  dirname: __dirname_esm,
  mode,
  isProd,
  isSSR,
  joined,
  dirExists,
});
"#,
    )
    .unwrap();

    let e = engine_with_read(dir.path().to_str().unwrap()).await;
    // Must not throw
    e.eval_file(&script).await.unwrap();

    let boot_raw = e.eval_to_string("globalThis.__test_boot").await.unwrap();
    let boot: serde_json::Value =
        serde_json::from_str(&boot_raw).expect("__test_boot should be valid JSON");

    assert!(
        boot["dirname"]
            .as_str()
            .unwrap_or("")
            .contains(dir.path().to_str().unwrap()),
        "dirname should match temp dir, got: {boot_raw}"
    );
    assert_eq!(boot["isSSR"], serde_json::Value::Bool(true));
    assert!(!boot["mode"].as_str().unwrap_or("").is_empty());
    // Verify the imported modules were actually usable (not just no-op dead code).
    assert!(
        boot["joined"].as_str().unwrap_or("").contains("sub"),
        "path.join should work, got: {boot_raw}"
    );
    assert_eq!(
        boot["dirExists"],
        serde_json::Value::Bool(true),
        "existsSync should return true, got: {boot_raw}"
    );
}
