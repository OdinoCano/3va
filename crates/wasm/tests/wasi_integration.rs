use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;
use vvva_permissions::PermissionState;
use vvva_wasm::WasmEngine;

// A simple WASI module in WAT format that attempts to read from the current directory
// or uses an environment variable, but for our test, we just check if it instantiates and runs
// `_start` without trapping (or trapping as expected if access is denied).
// We'll write a simple WAT that does nothing to ensure the engine loads correctly.
const NOOP_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func $start (export "_start")
    nop
  )
)
"#;

#[tokio::test]
async fn test_wasm_engine_instantiates() {
    let mut temp = NamedTempFile::new().unwrap();
    temp.write_all(NOOP_WAT.as_bytes()).unwrap();
    let path = temp.path();

    let state = PermissionState::new();
    let permissions = Arc::new(state);
    let engine = WasmEngine::new(permissions).unwrap();

    let args = vec![];
    let result = engine.eval_file_with_args(path, &args).await;
    assert!(
        result.is_ok(),
        "Engine should execute noop WAT successfully: {:?}",
        result
    );
}
