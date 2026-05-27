use std::io::Write;
use std::sync::Arc;
use tempfile::NamedTempFile;
use vvva_permissions::{Capability, PermissionState};
use vvva_wasm::WasmEngine;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn engine(perms: PermissionState) -> WasmEngine {
    WasmEngine::new(Arc::new(perms)).unwrap()
}

fn no_perms_engine() -> WasmEngine {
    engine(PermissionState::new())
}

async fn run_wat(engine: &WasmEngine, wat: &str, args: &[&str]) -> anyhow::Result<()> {
    let mut tmp = NamedTempFile::with_suffix(".wat").unwrap();
    tmp.write_all(wat.as_bytes()).unwrap();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    engine.eval_file_with_args(tmp.path(), &args).await
}

// ── Module loading ────────────────────────────────────────────────────────────

/// The minimal noop WAT module used as a baseline.
const NOOP_WAT: &str = r#"
(module
  (memory (export "memory") 1)
  (func $start (export "_start") nop)
)
"#;

#[tokio::test]
async fn noop_module_executes_successfully() {
    let result = run_wat(&no_perms_engine(), NOOP_WAT, &[]).await;
    assert!(
        result.is_ok(),
        "noop WAT should execute without error: {result:?}"
    );
}

#[tokio::test]
async fn module_without_start_returns_descriptive_error() {
    const NO_START: &str = r#"
    (module
      (memory (export "memory") 1)
      (func $helper (result i32) i32.const 42)
    )
    "#;
    let result = run_wat(&no_perms_engine(), NO_START, &[]).await;
    assert!(result.is_err(), "module without _start must fail");
    let msg = format!("{:?}", result.unwrap_err());
    assert!(
        msg.contains("_start"),
        "error should mention missing '_start', got: {msg}"
    );
}

#[tokio::test]
async fn invalid_wat_syntax_fails_gracefully() {
    const BAD_WAT: &str = "(module (this is not valid wat !!!))";
    let result = run_wat(&no_perms_engine(), BAD_WAT, &[]).await;
    assert!(
        result.is_err(),
        "malformed WAT should return Err, not panic"
    );
}

#[tokio::test]
async fn binary_wasm_format_loads_successfully() {
    // Compile NOOP_WAT to binary .wasm in the test, write as .wasm extension.
    let wasm_bytes = wat::parse_str(NOOP_WAT).unwrap();
    let mut tmp = NamedTempFile::with_suffix(".wasm").unwrap();
    tmp.write_all(&wasm_bytes).unwrap();

    let result = no_perms_engine().eval_file_with_args(tmp.path(), &[]).await;
    assert!(
        result.is_ok(),
        "binary .wasm should load and run: {result:?}"
    );
}

// ── WASI stdout (fd_write) ────────────────────────────────────────────────────

/// Module that writes "hello" to stdout via WASI fd_write.
/// We can't capture the output here, but we verify it runs without error.
const FD_WRITE_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory 1)
  (export "memory" (memory 0))
  ;; iovec at 0: ptr=8, len=5
  (data (i32.const 0) "\08\00\00\00\05\00\00\00")
  ;; "hello" at 8
  (data (i32.const 8) "hello")
  (func $start (export "_start")
    (drop (call $fd_write
      (i32.const 1)   ;; fd = stdout
      (i32.const 0)   ;; iovs
      (i32.const 1)   ;; iovs_len
      (i32.const 20)  ;; nwritten out-ptr
    ))
  )
)
"#;

#[tokio::test]
async fn wasi_fd_write_to_stdout_runs_without_error() {
    let result = run_wat(&no_perms_engine(), FD_WRITE_WAT, &[]).await;
    assert!(
        result.is_ok(),
        "fd_write to stdout should succeed: {result:?}"
    );
}

// ── Args forwarding ───────────────────────────────────────────────────────────

/// Module that checks argc >= 2 (program_name + one extra arg).
/// Returns normally on success; calls proc_exit(1) if argc < 2.
const CHECK_ARGS_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "args_sizes_get"
    (func $args_sizes_get (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "proc_exit"
    (func $proc_exit (param i32)))
  (memory 1)
  (export "memory" (memory 0))
  (func $start (export "_start")
    ;; Write argc to mem[0], argv_buf_size to mem[4]
    (drop (call $args_sizes_get (i32.const 0) (i32.const 4)))
    ;; If argc < 2 → signal failure
    (if (i32.lt_u (i32.load (i32.const 0)) (i32.const 2))
      (then (call $proc_exit (i32.const 1)))
    )
    ;; argc >= 2 → return normally (success)
  )
)
"#;

#[tokio::test]
async fn args_are_forwarded_to_wasm_module() {
    // With one extra arg, argc should be 2 (argv[0]=program, argv[1]="hello").
    let result = run_wat(&no_perms_engine(), CHECK_ARGS_WAT, &["hello"]).await;
    assert!(
        result.is_ok(),
        "module should see argc=2 when one arg is passed: {result:?}"
    );
}

#[tokio::test]
async fn no_extra_args_gives_argc_one() {
    // Without extra args, argc=1, module calls proc_exit(1) → Err.
    let result = run_wat(&no_perms_engine(), CHECK_ARGS_WAT, &[]).await;
    assert!(result.is_err(), "module should fail when argc < 2");
}

// ── Environment variable permissions ─────────────────────────────────────────

/// Module that checks environ count > 0. Returns normally on success,
/// calls proc_exit(1) if no env vars are visible.
const CHECK_ENV_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "environ_sizes_get"
    (func $environ_sizes_get (param i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "proc_exit"
    (func $proc_exit (param i32)))
  (memory 1)
  (export "memory" (memory 0))
  (func $start (export "_start")
    ;; environ_sizes_get: write count to mem[0], buf_size to mem[4]
    (drop (call $environ_sizes_get (i32.const 0) (i32.const 4)))
    ;; If count == 0 → signal failure
    (if (i32.eqz (i32.load (i32.const 0)))
      (then (call $proc_exit (i32.const 1)))
    )
    ;; count > 0 → return normally (success)
  )
)
"#;

#[tokio::test]
async fn env_vars_visible_with_env_access_permission() {
    let perms = PermissionState::new();
    // Grant full env access → builder.inherit_env() will be called.
    // The test process always has env vars (PATH, HOME, etc.).
    perms.grant(Capability::EnvAccess);
    let result = run_wat(&engine(perms), CHECK_ENV_WAT, &[]).await;
    assert!(
        result.is_ok(),
        "env vars should be visible when EnvAccess is granted: {result:?}"
    );
}

#[tokio::test]
async fn env_vars_empty_without_permission() {
    // No env capabilities granted → WASI context has 0 env vars.
    let result = run_wat(&no_perms_engine(), CHECK_ENV_WAT, &[]).await;
    assert!(
        result.is_err(),
        "module should fail when no env permission is granted"
    );
}

#[tokio::test]
async fn scoped_env_var_is_visible_in_wasm() {
    // Granting a specific var → it shows up in environ (count >= 1).
    // We use PATH which is always set in the test process.
    let perms = PermissionState::new();
    perms.grant(Capability::EnvVar("PATH".to_string()));
    let result = run_wat(&engine(perms), CHECK_ENV_WAT, &[]).await;
    assert!(
        result.is_ok(),
        "scoped EnvVar should appear in WASI environ: {result:?}"
    );
}

// ── File-system permissions ───────────────────────────────────────────────────

/// Module that tries to open "test.txt" in fd=3 (first preopened dir).
/// Returns normally if path_open succeeds (ret==0); calls proc_exit(1) on error.
/// WASI preview1 path_open signature: (i32 i32 i32 i32 i32 i64 i64 i32 i32) -> i32
/// dirflags is i32 (lookupflags), rights are i64 (rights).
const TRY_OPEN_FILE_WAT: &str = r#"
(module
  (import "wasi_snapshot_preview1" "path_open"
    (func $path_open
      (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "proc_exit"
    (func $proc_exit (param i32)))
  (memory 1)
  (export "memory" (memory 0))
  ;; "test.txt" at mem[0]
  (data (i32.const 0) "test.txt")
  (func $start (export "_start")
    (local $ret i32)
    ;; path_open(fd=3, dirflags=0, path=0, path_len=8, oflags=0,
    ;;           rights_base=2, rights_inh=2, fdflags=0, opened_fd_ptr=100)
    (local.set $ret
      (call $path_open
        (i32.const 3)    ;; fd  (first preopened dir)
        (i32.const 0)    ;; dirflags (lookupflags, i32)
        (i32.const 0)    ;; path ptr
        (i32.const 8)    ;; path len ("test.txt")
        (i32.const 0)    ;; oflags
        (i64.const 2)    ;; rights_base  (WASI_RIGHT_FD_READ=2)
        (i64.const 2)    ;; rights_inheriting
        (i32.const 0)    ;; fdflags
        (i32.const 100)  ;; result fd ptr
      )
    )
    ;; Non-zero means error → call proc_exit to signal failure
    (if (local.get $ret)
      (then (call $proc_exit (i32.const 1)))
    )
    ;; Zero means success → return normally
  )
)
"#;

#[tokio::test]
async fn file_read_allowed_with_file_read_permission() {
    let dir = tempfile::TempDir::new().unwrap();
    // Create the file that the WASM module will try to open.
    std::fs::write(dir.path().join("test.txt"), b"hello").unwrap();

    let perms = PermissionState::new();
    perms.grant(Capability::FileRead(dir.path().to_path_buf()));
    let eng = engine(perms);

    let result = run_wat(&eng, TRY_OPEN_FILE_WAT, &[]).await;
    assert!(
        result.is_ok(),
        "file open should succeed with FileRead permission: {result:?}"
    );
}

#[tokio::test]
async fn file_read_denied_without_permission() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("test.txt"), b"hello").unwrap();

    // No FileRead granted → no preopened dirs → path_open returns EBADF/ENOTCAPABLE.
    let result = run_wat(&no_perms_engine(), TRY_OPEN_FILE_WAT, &[]).await;
    assert!(
        result.is_err(),
        "file open should fail without FileRead permission"
    );
}

#[tokio::test]
async fn file_write_permission_allows_directory_access() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("test.txt"), b"content").unwrap();

    let perms = PermissionState::new();
    perms.grant(Capability::FileWrite(dir.path().to_path_buf()));
    let eng = engine(perms);

    // FileWrite preopens the dir with all permissions → read should also work.
    let result = run_wat(&eng, TRY_OPEN_FILE_WAT, &[]).await;
    assert!(
        result.is_ok(),
        "FileWrite should preopened dir with full access: {result:?}"
    );
}

// ── Multiple permissions combined ─────────────────────────────────────────────

#[tokio::test]
async fn module_runs_with_all_permissions_granted() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("test.txt"), b"data").unwrap();

    let perms = PermissionState::new();
    perms.grant(Capability::FileRead(dir.path().to_path_buf()));
    perms.grant(Capability::EnvAccess);
    let eng = engine(perms);

    // Both the file-open and env-check modules should pass.
    let file_ok = run_wat(&eng, TRY_OPEN_FILE_WAT, &[]).await;
    let env_ok = run_wat(&eng, CHECK_ENV_WAT, &[]).await;

    assert!(file_ok.is_ok(), "file access with permission: {file_ok:?}");
    assert!(env_ok.is_ok(), "env access with permission: {env_ok:?}");
}

// ── Error propagation ─────────────────────────────────────────────────────────

#[tokio::test]
async fn wasm_trap_propagates_as_rust_error() {
    const TRAP_WAT: &str = r#"
    (module
      (memory 1) (export "memory" (memory 0))
      (func $start (export "_start") unreachable)
    )
    "#;
    let result = run_wat(&no_perms_engine(), TRAP_WAT, &[]).await;
    assert!(result.is_err(), "unreachable trap must propagate as Err");
}

#[tokio::test]
async fn nonexistent_file_returns_error() {
    let perms = Arc::new(PermissionState::new());
    let eng = WasmEngine::new(perms).unwrap();
    let result = eng
        .eval_file_with_args(std::path::Path::new("/no/such/file.wat"), &[])
        .await;
    assert!(
        result.is_err(),
        "loading a nonexistent file must return Err"
    );
}
