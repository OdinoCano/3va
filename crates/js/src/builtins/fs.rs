use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use v8::{ContextScope, FunctionCallbackArguments, HandleScope, PinScope, ReturnValue};
use vvva_permissions::{Capability, PermissionState};

// Thread-local, not a process-wide static: a `OnceLock` here only accepts the
// *first* engine's permissions ever created in the process and silently
// ignores every later `inject_fs` call, so every other `JsEngine` (each test
// in `cargo test`, each worker in a long-lived process) would silently
// inherit the first engine's grants instead of its own. Each `JsEngine`'s V8
// isolate never migrates threads mid-lifetime, so a thread-local correctly
// scopes permissions per engine.
thread_local! {
    static FS_PERMISSIONS: std::cell::RefCell<Option<Arc<PermissionState>>> =
        const { std::cell::RefCell::new(None) };
}
fn perms() -> Arc<PermissionState> {
    FS_PERMISSIONS.with(|p| {
        p.borrow()
            .clone()
            .expect("inject_fs not called on this thread")
    })
}

static FS_FD_TABLE: std::sync::OnceLock<Arc<Mutex<FdTable>>> = std::sync::OnceLock::new();
fn fdt() -> &'static Arc<Mutex<FdTable>> {
    FS_FD_TABLE.get().unwrap()
}

/// Each live watcher: holds the notify handle (dropping it stops watching)
/// and an async receiver for incoming events.
struct WatcherEntry {
    _watcher: notify::RecommendedWatcher,
    rx: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<notify::Result<notify::Event>>>,
}
type WatchTable = Arc<Mutex<HashMap<u32, Arc<WatcherEntry>>>>;
static FS_WATCH_TABLE: std::sync::OnceLock<WatchTable> = std::sync::OnceLock::new();

/// Shared file-descriptor table: fd → open file handle.
/// fd numbers start at 3 (0/1/2 = stdin/stdout/stderr).
#[derive(Default)]
struct FdTable {
    next_fd: i32,
    table: HashMap<i32, std::fs::File>,
}

impl FdTable {
    fn new() -> Self {
        Self {
            next_fd: 3,
            table: HashMap::new(),
        }
    }
    fn insert(&mut self, file: std::fs::File) -> i32 {
        let fd = self.next_fd;
        self.next_fd += 1;
        self.table.insert(fd, file);
        fd
    }
    fn get_mut(&mut self, fd: i32) -> Option<&mut std::fs::File> {
        self.table.get_mut(&fd)
    }
    fn remove(&mut self, fd: i32) -> bool {
        self.table.remove(&fd).is_some()
    }
}

fn flags_to_open_options(flags: &str) -> std::fs::OpenOptions {
    let mut opts = std::fs::OpenOptions::new();
    match flags {
        "r" | "rs" => {
            opts.read(true);
        }
        "r+" | "rs+" => {
            opts.read(true).write(true);
        }
        "w" => {
            opts.write(true).create(true).truncate(true);
        }
        "w+" => {
            opts.read(true).write(true).create(true).truncate(true);
        }
        "wx" => {
            opts.write(true).create_new(true);
        }
        "wx+" => {
            opts.read(true).write(true).create_new(true);
        }
        "a" => {
            opts.append(true).create(true);
        }
        "a+" => {
            opts.read(true).append(true).create(true);
        }
        "ax" => {
            opts.append(true).create_new(true);
        }
        "ax+" => {
            opts.read(true).append(true).create_new(true);
        }
        _ => {
            opts.read(true);
        }
    }
    opts
}

fn js_err<'s>(scope: &mut PinScope<'s, '_>, msg: impl AsRef<str>) -> v8::Local<'s, v8::Value> {
    let escaped = msg.as_ref().replace('\\', "\\\\").replace('"', "\\\"");
    let src = format!("new Error(\"{}\")", escaped);
    let source = v8::String::new(scope, &src).unwrap();
    v8::Script::compile(scope, source, None)
        .and_then(|s| s.run(scope))
        .unwrap_or_else(|| v8::undefined(scope).into())
}

/// Create a Node.js-style filesystem error with a `.code` property so that
/// callers (e.g. Prisma) can distinguish ENOENT from other errors.
fn fs_err<'s>(
    scope: &mut PinScope<'s, '_>,
    io_err: &std::io::Error,
    path: &str,
) -> v8::Local<'s, v8::Value> {
    let code = match io_err.kind() {
        std::io::ErrorKind::NotFound => "ENOENT",
        std::io::ErrorKind::PermissionDenied => "EACCES",
        std::io::ErrorKind::AlreadyExists => "EEXIST",
        std::io::ErrorKind::IsADirectory => "EISDIR",
        _ => "EIO",
    };
    let msg = format!("{}: {}: '{}'", code, io_err, path);
    let escaped_msg = msg.replace('\\', "\\\\").replace('"', "\\\"");
    let escaped_path = path.replace('\\', "\\\\").replace('"', "\\\"");
    let src = format!(
        "(function(){{var e=new Error(\"{}\");e.code=\"{}\";e.path=\"{}\";return e;}})()",
        escaped_msg, code, escaped_path
    );
    let source = v8::String::new(scope, &src).unwrap();
    v8::Script::compile(scope, source, None)
        .and_then(|s| s.run(scope))
        .unwrap_or_else(|| v8::undefined(scope).into())
}

fn perm_err<'s>(
    scope: &mut PinScope<'s, '_>,
    flag: &str,
    path: &std::path::Path,
) -> v8::Local<'s, v8::Value> {
    let msg = format!(
        "Permission denied: --allow-{}={} is required",
        flag,
        path.display()
    );
    let escaped_msg = msg.replace('\\', "\\\\").replace('"', "\\\"");
    let escaped_path = path
        .display()
        .to_string()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let src = format!(
        "(function(){{var e=new Error(\"{}\");e.code=\"EACCES\";e.path=\"{}\";return e;}})()",
        escaped_msg, escaped_path
    );
    let source = v8::String::new(scope, &src).unwrap();
    v8::Script::compile(scope, source, None)
        .and_then(|s| s.run(scope))
        .unwrap_or_else(|| v8::undefined(scope).into())
}

fn set_fn(
    scope: &mut ContextScope<HandleScope>,
    obj: v8::Local<v8::Object>,
    name: &str,
    f: impl v8::MapFnTo<v8::FunctionCallback>,
) {
    let func = v8::Function::new(scope, f).unwrap();
    let key = v8::String::new(scope, name).unwrap().into();
    obj.set(scope, key, func.into());
}

fn stat_meta_to_json(meta: &std::fs::Metadata) -> String {
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0);
    let atime_ms = meta
        .accessed()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0);
    let ctime_ms = meta
        .created()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as f64)
        .unwrap_or(mtime_ms);
    #[cfg(unix)]
    let mode = meta.permissions().mode();
    #[cfg(not(unix))]
    let mode: u32 = if meta.permissions().readonly() {
        0o444
    } else {
        0o644
    };
    let is_dir = meta.is_dir();
    let is_file = meta.is_file();
    let is_symlink = meta.file_type().is_symlink();
    format!(
        r#"{{"size":{},"mode":{},"isFile":{},"isDirectory":{},"isSymbolicLink":{},"mtimeMs":{},"atimeMs":{},"ctimeMs":{},"birthtimeMs":{},"nlink":1,"uid":0,"gid":0,"ino":0,"dev":0,"rdev":0}}"#,
        meta.len(),
        mode,
        is_file,
        is_dir,
        is_symlink,
        mtime_ms,
        atime_ms,
        ctime_ms,
        ctime_ms
    )
}

pub fn inject_fs(
    scope: &mut ContextScope<HandleScope>,
    permissions: Arc<PermissionState>,
) -> anyhow::Result<()> {
    FS_PERMISSIONS.with(|p| *p.borrow_mut() = Some(permissions));
    FS_FD_TABLE.set(Arc::new(Mutex::new(FdTable::new()))).ok();
    let context = scope.get_current_context();
    let globals = context.global(scope);

    // ── __fsFdOpen(path, flags_str) -> fd ─────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsFdOpen",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let flags = args.get(1).to_rust_string_lossy(scope);
            let path = PathBuf::from(&path_str);
            let needs_write = matches!(
                flags.as_str(),
                "w" | "w+" | "wx" | "wx+" | "a" | "a+" | "ax" | "ax+" | "r+" | "rs+"
            );
            if needs_write {
                if !perms().check(&Capability::FileWrite(path.clone())) {
                    let err = perm_err(scope, "write", &path);
                    scope.throw_exception(err);
                    return;
                }
            } else if !perms().check(&Capability::FileRead(path.clone())) {
                let err = perm_err(scope, "read", &path);
                scope.throw_exception(err);
                return;
            }
            match flags_to_open_options(&flags).open(&path) {
                Ok(file) => {
                    let fd = fdt().lock().unwrap().insert(file);
                    rv.set(v8::Integer::new(scope, fd).into());
                }
                Err(e) => {
                    let err = fs_err(scope, &e, &path_str);
                    scope.throw_exception(err);
                }
            }
        },
    );

    // ── __fsFdClose(fd) ───────────────────────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsFdClose",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let fd = args.get(0).int32_value(scope).unwrap_or(0);
            if !fdt().lock().unwrap().remove(fd) {
                let err = js_err(scope, format!("EBADF: bad file descriptor {fd}"));
                scope.throw_exception(err);
            }
        },
    );

    // ── __fsFdRead(fd, length, position) -> Vec<u8> ───────────────────────────
    set_fn(
        scope,
        globals,
        "__fsFdRead",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let fd = args.get(0).int32_value(scope).unwrap_or(0);
            let length = args.get(1).uint32_value(scope).unwrap_or(0) as usize;
            let position_arg = args.get(2);
            let position = if position_arg.is_null_or_undefined() {
                None
            } else {
                position_arg.integer_value(scope)
            };

            let mut table = fdt().lock().unwrap();
            let file = match table.get_mut(fd) {
                Some(f) => f,
                None => {
                    let err = js_err(scope, format!("EBADF: bad file descriptor {fd}"));
                    scope.throw_exception(err);
                    return;
                }
            };
            if let Some(pos) = position
                && let Err(e) = file.seek(SeekFrom::Start(pos as u64))
            {
                let err = js_err(scope, e.to_string());
                scope.throw_exception(err);
                return;
            }
            let mut buf = vec![0u8; length.min(65536)];
            match file.read(&mut buf) {
                Ok(n) => {
                    buf.truncate(n);
                    rv.set(crate::builtins::v8_compat::uint8array_from_bytes(scope, &buf).into());
                }
                Err(e) => {
                    let err = js_err(scope, e.to_string());
                    scope.throw_exception(err);
                }
            }
        },
    );

    // ── __fsFdWrite(fd, data, position) -> bytes_written ─────────────────────
    set_fn(
        scope,
        globals,
        "__fsFdWrite",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let fd = args.get(0).int32_value(scope).unwrap_or(0);
            let data = crate::builtins::v8_compat::js_value_to_bytes(scope, args.get(1));
            let position_arg = args.get(2);
            let position = if position_arg.is_null_or_undefined() {
                None
            } else {
                position_arg.integer_value(scope)
            };

            let mut table = fdt().lock().unwrap();
            let file = match table.get_mut(fd) {
                Some(f) => f,
                None => {
                    let err = js_err(scope, format!("EBADF: bad file descriptor {fd}"));
                    scope.throw_exception(err);
                    return;
                }
            };
            if let Some(pos) = position
                && let Err(e) = file.seek(SeekFrom::Start(pos as u64))
            {
                let err = js_err(scope, e.to_string());
                scope.throw_exception(err);
                return;
            }
            match file.write_all(&data) {
                Ok(()) => rv.set(v8::Integer::new_from_unsigned(scope, data.len() as u32).into()),
                Err(e) => {
                    let err = js_err(scope, e.to_string());
                    scope.throw_exception(err);
                }
            }
        },
    );

    // ── __fsFdStat(fd) -> JSON stat ───────────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsFdStat",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let fd = args.get(0).int32_value(scope).unwrap_or(0);
            let mut table = fdt().lock().unwrap();
            let file = match table.get_mut(fd) {
                Some(f) => f,
                None => {
                    let err = js_err(scope, format!("EBADF: bad file descriptor {fd}"));
                    scope.throw_exception(err);
                    return;
                }
            };
            match file.metadata() {
                Ok(meta) => rv.set(
                    v8::String::new(scope, &stat_meta_to_json(&meta))
                        .unwrap()
                        .into(),
                ),
                Err(e) => {
                    let err = js_err(scope, e.to_string());
                    scope.throw_exception(err);
                }
            }
        },
    );

    // ── __fsMkdtemp(prefix) -> String ─────────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsMkdtemp",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let prefix = args.get(0).to_rust_string_lossy(scope);
            let path = PathBuf::from(&prefix);
            let parent = path.parent().unwrap_or(std::path::Path::new("/tmp"));
            if !perms().check(&Capability::FileWrite(parent.to_path_buf())) {
                let err = perm_err(scope, "write", parent);
                scope.throw_exception(err);
                return;
            }
            // Create a temp dir with a random suffix
            let unique = format!("{}{}", prefix, std::process::id());
            match std::fs::create_dir_all(&unique) {
                Ok(()) => rv.set(v8::String::new(scope, &unique).unwrap().into()),
                Err(e) => {
                    let err = fs_err(scope, &e, &unique);
                    scope.throw_exception(err);
                }
            }
        },
    );

    // ── __fsReadFileSync(path) -> String ──────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsReadFileSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileRead(path.clone())) {
                let err = perm_err(scope, "read", &path);
                scope.throw_exception(err);
                return;
            }
            match std::fs::read_to_string(&path) {
                Ok(content) => rv.set(v8::String::new(scope, &content).unwrap().into()),
                Err(e) => {
                    let err = fs_err(scope, &e, &path_str);
                    scope.throw_exception(err);
                }
            }
        },
    );

    // ── __fsWriteFileBytesSync(path, bytes: Vec<u8>) ──────────────────────────
    set_fn(
        scope,
        globals,
        "__fsWriteFileBytesSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let data = crate::builtins::v8_compat::js_value_to_bytes(scope, args.get(1));
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileWrite(path.clone())) {
                let err = perm_err(scope, "write", &path);
                scope.throw_exception(err);
                return;
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            if let Err(e) = std::fs::write(&path, &data) {
                let err = fs_err(scope, &e, &path_str);
                scope.throw_exception(err);
            }
        },
    );

    // ── __fsReadFileBytesSync(path) -> JSON array of byte values ──────────────
    set_fn(
        scope,
        globals,
        "__fsReadFileBytesSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileRead(path.clone())) {
                let err = perm_err(scope, "read", &path);
                scope.throw_exception(err);
                return;
            }
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let json = serde_json::to_string(&bytes).unwrap_or_else(|_| "[]".to_string());
                    rv.set(v8::String::new(scope, &json).unwrap().into());
                }
                Err(e) => {
                    let err = fs_err(scope, &e, &path_str);
                    scope.throw_exception(err);
                }
            }
        },
    );

    // ── __fsWriteFileSync(path, content) ──────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsWriteFileSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let content = args.get(1).to_rust_string_lossy(scope);
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileWrite(path.clone())) {
                let err = perm_err(scope, "write", &path);
                scope.throw_exception(err);
                return;
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).ok();
            }
            if let Err(e) = std::fs::write(&path, content) {
                let err = fs_err(scope, &e, &path_str);
                scope.throw_exception(err);
            }
        },
    );

    // ── __fsAppendFileSync(path, content) ─────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsAppendFileSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let content = args.get(1).to_rust_string_lossy(scope);
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileWrite(path.clone())) {
                let err = perm_err(scope, "write", &path);
                scope.throw_exception(err);
                return;
            }
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path);
            match file {
                Ok(mut file) => {
                    if let Err(e) = file.write_all(content.as_bytes()) {
                        let err = fs_err(scope, &e, &path_str);
                        scope.throw_exception(err);
                    }
                }
                Err(e) => {
                    let err = fs_err(scope, &e, &path_str);
                    scope.throw_exception(err);
                }
            }
        },
    );

    // ── __fsExistsSync(path) -> bool ──────────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsExistsSync",
        |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            rv.set(v8::Boolean::new(scope, PathBuf::from(path_str).exists()).into());
        },
    );

    // ── __fsStatSync(path, follow_symlinks) -> JSON ───────────────────────────
    set_fn(
        scope,
        globals,
        "__fsStatSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let follow_arg = args.get(1);
            let follow = if follow_arg.is_undefined() {
                true
            } else {
                follow_arg.to_rust_string_lossy(scope) != "false"
            };
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileRead(path.clone())) {
                let err = perm_err(scope, "read", &path);
                scope.throw_exception(err);
                return;
            }
            let meta = if follow {
                std::fs::metadata(&path)
            } else {
                std::fs::symlink_metadata(&path)
            };
            match meta {
                Ok(meta) => rv.set(
                    v8::String::new(scope, &stat_meta_to_json(&meta))
                        .unwrap()
                        .into(),
                ),
                Err(e) => {
                    let err = fs_err(scope, &e, &path_str);
                    scope.throw_exception(err);
                }
            }
        },
    );

    // ── __fsAccessSync(path, mode) -> "ok" | error message ───────────────────
    set_fn(
        scope,
        globals,
        "__fsAccessSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let mode = args.get(1).uint32_value(scope).unwrap_or(0);
            let path = PathBuf::from(&path_str);

            let result = if !path.exists() {
                format!("ENOENT: no such file or directory: '{}'", path_str)
            } else if (mode & 4 != 0 && !perms().check(&Capability::FileRead(path.clone())))
                || (mode & 2 != 0 && !perms().check(&Capability::FileWrite(path.clone())))
            {
                format!("EACCES: permission denied: '{}'", path_str)
            } else {
                "ok".to_string()
            };
            rv.set(v8::String::new(scope, &result).unwrap().into());
        },
    );

    // ── __fsRealpathSync(path) -> String ──────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsRealpathSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileRead(path.clone())) {
                let err = perm_err(scope, "read", &path);
                scope.throw_exception(err);
                return;
            }
            match std::fs::canonicalize(&path) {
                Ok(p) => rv.set(v8::String::new(scope, &p.to_string_lossy()).unwrap().into()),
                Err(e) => {
                    let err = fs_err(scope, &e, &path_str);
                    scope.throw_exception(err);
                }
            }
        },
    );

    // ── __fsReaddirSync(path) -> String (JSON array of filenames) ─────────────
    set_fn(
        scope,
        globals,
        "__fsReaddirSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileRead(path.clone())) {
                let err = perm_err(scope, "read", &path);
                scope.throw_exception(err);
                return;
            }
            match std::fs::read_dir(&path) {
                Ok(entries) => {
                    let names: Vec<String> = entries
                        .flatten()
                        .filter_map(|e| e.file_name().into_string().ok())
                        .collect();
                    let json = serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_string());
                    rv.set(v8::String::new(scope, &json).unwrap().into());
                }
                Err(e) => {
                    let err = fs_err(scope, &e, &path_str);
                    scope.throw_exception(err);
                }
            }
        },
    );

    // ── __fsMkdirSync(path) ───────────────────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsMkdirSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileWrite(path.clone())) {
                let err = perm_err(scope, "write", &path);
                scope.throw_exception(err);
                return;
            }
            if let Err(e) = std::fs::create_dir_all(&path) {
                let err = fs_err(scope, &e, &path_str);
                scope.throw_exception(err);
            }
        },
    );

    // ── __fsRmSync(path) ──────────────────────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsRmSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileWrite(path.clone())) {
                let err = perm_err(scope, "write", &path);
                scope.throw_exception(err);
                return;
            }
            let result = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            if let Err(e) = result {
                let err = fs_err(scope, &e, &path_str);
                scope.throw_exception(err);
            }
        },
    );

    // ── __fsUnlinkSync(path) ──────────────────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsUnlinkSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileWrite(path.clone())) {
                let err = perm_err(scope, "write", &path);
                scope.throw_exception(err);
                return;
            }
            if let Err(e) = std::fs::remove_file(&path) {
                let err = fs_err(scope, &e, &path_str);
                scope.throw_exception(err);
            }
        },
    );

    // ── __fsRenameSync(from, to) ──────────────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsRenameSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let from_str = args.get(0).to_rust_string_lossy(scope);
            let to_str = args.get(1).to_rust_string_lossy(scope);
            let from = PathBuf::from(&from_str);
            let to = PathBuf::from(&to_str);
            if !perms().check(&Capability::FileWrite(from.clone())) {
                let err = perm_err(scope, "write", &from);
                scope.throw_exception(err);
                return;
            }
            if !perms().check(&Capability::FileWrite(to.clone())) {
                let err = perm_err(scope, "write", &to);
                scope.throw_exception(err);
                return;
            }
            if let Err(e) = std::fs::rename(&from, &to) {
                let err = fs_err(scope, &e, &from_str);
                scope.throw_exception(err);
            }
        },
    );

    // ── __fsCpSync(src, dest) — recursive copy (Node 16+ fs.cp) ─────────────
    set_fn(
        scope,
        globals,
        "__fsCpSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let src_str = args.get(0).to_rust_string_lossy(scope);
            let dest_str = args.get(1).to_rust_string_lossy(scope);
            let src = PathBuf::from(&src_str);
            let dest = PathBuf::from(&dest_str);
            if !perms().check(&Capability::FileRead(src.clone())) {
                let err = perm_err(scope, "read", &src);
                scope.throw_exception(err);
                return;
            }
            if !perms().check(&Capability::FileWrite(dest.clone())) {
                let err = perm_err(scope, "write", &dest);
                scope.throw_exception(err);
                return;
            }
            fn copy_all(src: &PathBuf, dst: &PathBuf) -> std::io::Result<()> {
                if src.is_dir() {
                    std::fs::create_dir_all(dst)?;
                    for entry in std::fs::read_dir(src)? {
                        let entry = entry?;
                        copy_all(&entry.path(), &dst.join(entry.file_name()))?;
                    }
                } else {
                    if let Some(parent) = dst.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::copy(src, dst)?;
                }
                Ok(())
            }
            if let Err(e) = copy_all(&src, &dest) {
                let err = fs_err(scope, &e, &src_str);
                scope.throw_exception(err);
            }
        },
    );

    // ── __fsCopyFileSync(src, dest) ───────────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsCopyFileSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let src_str = args.get(0).to_rust_string_lossy(scope);
            let dest_str = args.get(1).to_rust_string_lossy(scope);
            let src = PathBuf::from(&src_str);
            let dest = PathBuf::from(&dest_str);
            if !perms().check(&Capability::FileRead(src.clone())) {
                let err = perm_err(scope, "read", &src);
                scope.throw_exception(err);
                return;
            }
            if !perms().check(&Capability::FileWrite(dest.clone())) {
                let err = perm_err(scope, "write", &dest);
                scope.throw_exception(err);
                return;
            }
            if let Err(e) = std::fs::copy(&src, &dest) {
                let err = fs_err(scope, &e, &src_str);
                scope.throw_exception(err);
            }
        },
    );

    // ── __fsChmodSync(path, mode) ─────────────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsChmodSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let mode_arg = args.get(1);
            let mode_str = if mode_arg.is_undefined() {
                "0o644".to_string()
            } else {
                mode_arg.to_rust_string_lossy(scope)
            };
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileWrite(path.clone())) {
                let err = perm_err(scope, "write", &path);
                scope.throw_exception(err);
                return;
            }
            let mode = u32::from_str_radix(mode_str.trim_start_matches("0o"), 8)
                .or_else(|_| mode_str.parse::<u32>())
                .unwrap_or(0o644);
            #[cfg(unix)]
            {
                if let Err(e) =
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                {
                    let err = fs_err(scope, &e, &path_str);
                    scope.throw_exception(err);
                }
            }
            #[cfg(not(unix))]
            {
                match std::fs::metadata(&path) {
                    Ok(meta) => {
                        let mut perms_obj = meta.permissions();
                        perms_obj.set_readonly(mode & 0o200 == 0);
                        if let Err(e) = std::fs::set_permissions(&path, perms_obj) {
                            let err = fs_err(scope, &e, &path_str);
                            scope.throw_exception(err);
                        }
                    }
                    Err(e) => {
                        let err = fs_err(scope, &e, &path_str);
                        scope.throw_exception(err);
                    }
                }
            }
        },
    );

    // ── __fsSymlinkSync(target, path) ─────────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsSymlinkSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let target_str = args.get(0).to_rust_string_lossy(scope);
            let path_str = args.get(1).to_rust_string_lossy(scope);
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileWrite(path.clone())) {
                let err = perm_err(scope, "write", &path);
                scope.throw_exception(err);
                return;
            }
            #[cfg(unix)]
            let result = std::os::unix::fs::symlink(&target_str, &path);
            #[cfg(windows)]
            let result = std::os::windows::fs::symlink_file(&target_str, &path);
            #[cfg(not(any(unix, windows)))]
            let result: std::io::Result<()> = Err(std::io::Error::other(
                "symlink not supported on this platform",
            ));
            if let Err(e) = result {
                let err = fs_err(scope, &e, &path_str);
                scope.throw_exception(err);
            }
        },
    );

    // ── fs.watch backend (inotify / kqueue / FSEvents via notify crate) ──────────
    {
        use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
        use std::sync::atomic::{AtomicU32, Ordering};
        use tokio::sync::mpsc;

        static NEXT_WATCHER_ID: AtomicU32 = AtomicU32::new(1);

        FS_WATCH_TABLE
            .set(Arc::new(Mutex::new(HashMap::new())))
            .ok();
        fn watch_table() -> &'static WatchTable {
            FS_WATCH_TABLE.get().unwrap()
        }

        // __fsWatchCreate(path, recursive) → watcher_id  (synchronous)
        set_fn(
            scope,
            globals,
            "__fsWatchCreate",
            move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let path_str = args.get(0).to_rust_string_lossy(scope);
                let recursive = args.get(1).boolean_value(scope);
                let path = PathBuf::from(&path_str);
                if !perms().check(&Capability::FileRead(path.clone())) {
                    let err = perm_err(scope, "watch", &path);
                    scope.throw_exception(err);
                    return;
                }

                let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>(256);
                let watcher = RecommendedWatcher::new(
                    move |res: notify::Result<notify::Event>| {
                        let _ = tx.blocking_send(res);
                    },
                    Config::default(),
                );
                let mut watcher = match watcher {
                    Ok(w) => w,
                    Err(e) => {
                        let err = js_err(scope, format!("fs.watch: {e}"));
                        scope.throw_exception(err);
                        return;
                    }
                };

                let mode = if recursive {
                    RecursiveMode::Recursive
                } else {
                    RecursiveMode::NonRecursive
                };
                if let Err(e) = watcher.watch(&path, mode) {
                    let err = js_err(scope, format!("fs.watch '{}': {e}", path_str));
                    scope.throw_exception(err);
                    return;
                }

                let id = NEXT_WATCHER_ID.fetch_add(1, Ordering::Relaxed);
                watch_table().lock().unwrap().insert(
                    id,
                    Arc::new(WatcherEntry {
                        _watcher: watcher,
                        rx: tokio::sync::Mutex::new(rx),
                    }),
                );
                rv.set(v8::Integer::new_from_unsigned(scope, id).into());
            },
        );

        // __fsWatchNext(id) → JSON | null  non-blocking poll for the next event.
        //   JSON: {"eventType":"change"|"rename","filename":"foo.txt"}
        //   Returns null when no event is ready yet OR the watcher was closed —
        //   the JS side polls this on an interval (see dgram.rs's __udpRecv for
        //   the same pattern). This used to block via
        //   `block_in_place`+`Handle::block_on`, which panics outright on a
        //   current_thread Tokio runtime (e.g. plain `#[tokio::test]`) — and
        //   since that panic happened inside a V8 native callback, it surfaced
        //   as an unrecoverable "failed to initiate panic" process abort rather
        //   than a normal Rust panic message.
        set_fn(
            scope,
            globals,
            "__fsWatchNext",
            move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let id = args.get(0).uint32_value(scope).unwrap_or(0);
                let entry = {
                    let guard = watch_table().lock().unwrap();
                    guard.get(&id).cloned()
                };
                let entry = match entry {
                    Some(e) => e,
                    None => {
                        rv.set(v8::null(scope).into());
                        return;
                    }
                };

                let event = match entry.rx.try_lock() {
                    Ok(mut rx) => rx.try_recv().ok(),
                    Err(_) => None,
                };

                match event {
                    None => rv.set(v8::null(scope).into()),
                    Some(Err(_)) => rv.set(v8::null(scope).into()),
                    Some(Ok(ev)) => {
                        use notify::event::{EventKind, ModifyKind, RenameMode};
                        let event_type = match ev.kind {
                            EventKind::Create(_) => "rename",
                            EventKind::Remove(_) => "rename",
                            EventKind::Modify(ModifyKind::Name(
                                RenameMode::From | RenameMode::To | RenameMode::Both,
                            )) => "rename",
                            _ => "change",
                        };
                        let filename = ev
                            .paths
                            .first()
                            .and_then(|p| {
                                p.file_name()
                                    .and_then(|n| n.to_str())
                                    .map(|s| s.to_string())
                            })
                            .unwrap_or_default();
                        let json = format!(
                            "{{\"eventType\":\"{event_type}\",\"filename\":\"{filename}\"}}"
                        );
                        rv.set(v8::String::new(scope, &json).unwrap().into());
                    }
                }
            },
        );

        // __fsWatchClose(id) → void  (synchronous; drops the watcher entry)
        set_fn(
            scope,
            globals,
            "__fsWatchClose",
            move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
                let id = args.get(0).uint32_value(scope).unwrap_or(0);
                watch_table().lock().unwrap().remove(&id);
            },
        );
    }

    // ── __fsWatchPollStat(path, interval_ms) → stat JSON (blocking) ─────────────
    // Sleeps `interval_ms`, then returns the current stat as JSON so the JS
    // layer can detect changes.
    set_fn(
        scope,
        globals,
        "__fsWatchPollStat",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let interval_ms = args.get(1).uint32_value(scope).unwrap_or(0) as u64;
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileRead(path.clone())) {
                let err = js_err(scope, format!("EACCES: permission denied: '{path_str}'"));
                scope.throw_exception(err);
                return;
            }
            // Plain thread sleep, not tokio::time::sleep via block_in_place +
            // Handle::block_on — that combination panics outright on a
            // current_thread runtime (see __fsWatchNext's history above).
            std::thread::sleep(std::time::Duration::from_millis(interval_ms));
            match std::fs::metadata(&path) {
                Ok(meta) => rv.set(
                    v8::String::new(scope, &stat_meta_to_json(&meta))
                        .unwrap()
                        .into(),
                ),
                Err(e) => {
                    let err = fs_err(scope, &e, &path_str);
                    scope.throw_exception(err);
                }
            }
        },
    );

    // ── __fsReadlinkSync(path) -> String ──────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsReadlinkSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileRead(path.clone())) {
                let err = perm_err(scope, "read", &path);
                scope.throw_exception(err);
                return;
            }
            match std::fs::read_link(&path) {
                Ok(target) => rv.set(
                    v8::String::new(scope, &target.to_string_lossy())
                        .unwrap()
                        .into(),
                ),
                Err(e) => {
                    let err = fs_err(scope, &e, &path_str);
                    scope.throw_exception(err);
                }
            }
        },
    );

    // ── __fsChownSync(path, uid, gid) ─────────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsChownSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let uid = args.get(1).uint32_value(scope).unwrap_or(0);
            let gid = args.get(2).uint32_value(scope).unwrap_or(0);
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileWrite(path.clone())) {
                let err = perm_err(scope, "write", &path);
                scope.throw_exception(err);
                return;
            }
            #[cfg(unix)]
            {
                let uid = Some(uid);
                let gid = Some(gid);
                if let Err(e) = nix::unistd::chown(
                    &path,
                    uid.map(nix::unistd::Uid::from_raw),
                    gid.map(nix::unistd::Gid::from_raw),
                ) {
                    let io_err = std::io::Error::from_raw_os_error(e as i32);
                    let err = fs_err(scope, &io_err, &path_str);
                    scope.throw_exception(err);
                }
            }
            #[cfg(not(unix))]
            {
                let _ = (uid, gid);
            }
        },
    );

    // ── __fsLchownSync(path, uid, gid) ────────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsLchownSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let uid = args.get(1).uint32_value(scope).unwrap_or(0);
            let gid = args.get(2).uint32_value(scope).unwrap_or(0);
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileWrite(path.clone())) {
                let err = perm_err(scope, "write", &path);
                scope.throw_exception(err);
                return;
            }
            #[cfg(unix)]
            {
                if let Err(e) = nix::unistd::fchownat(
                    None,
                    &path,
                    Some(nix::unistd::Uid::from_raw(uid)),
                    Some(nix::unistd::Gid::from_raw(gid)),
                    nix::fcntl::AtFlags::AT_SYMLINK_NOFOLLOW,
                ) {
                    let io_err = std::io::Error::from_raw_os_error(e as i32);
                    let err = fs_err(scope, &io_err, &path_str);
                    scope.throw_exception(err);
                }
            }
            #[cfg(not(unix))]
            {
                let _ = (uid, gid);
            }
        },
    );

    // ── __fsFchownSync(fd, uid, gid) ──────────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsFchownSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let fd = args.get(0).int32_value(scope).unwrap_or(-1);
            let uid = args.get(1).uint32_value(scope).unwrap_or(0);
            let gid = args.get(2).uint32_value(scope).unwrap_or(0);
            #[cfg(unix)]
            {
                use std::os::unix::io::AsRawFd;
                let mut table = fdt().lock().unwrap();
                if let Some(file) = table.get_mut(fd) {
                    if let Err(e) = nix::unistd::fchown(
                        file.as_raw_fd(),
                        Some(nix::unistd::Uid::from_raw(uid)),
                        Some(nix::unistd::Gid::from_raw(gid)),
                    ) {
                        let io_err = std::io::Error::from_raw_os_error(e as i32);
                        let err = fs_err(scope, &io_err, &format!("fd {}", fd));
                        scope.throw_exception(err);
                    }
                } else {
                    let err = js_err(scope, format!("EBADF: bad file descriptor: {}", fd));
                    scope.throw_exception(err);
                }
            }
            #[cfg(not(unix))]
            {
                let _ = (fd, uid, gid);
            }
        },
    );

    // ── __fsUtimesSync(path, atime, mtime) ────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsUtimesSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let atime = args.get(1).number_value(scope).unwrap_or(0.0);
            let mtime = args.get(2).number_value(scope).unwrap_or(0.0);
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileWrite(path.clone())) {
                let err = perm_err(scope, "write", &path);
                scope.throw_exception(err);
                return;
            }
            let atime_ft =
                filetime::FileTime::from_unix_time(atime as i64, ((atime.fract()) * 1e9) as u32);
            let mtime_ft =
                filetime::FileTime::from_unix_time(mtime as i64, ((mtime.fract()) * 1e9) as u32);
            if let Err(e) = filetime::set_file_times(&path, atime_ft, mtime_ft) {
                let err = fs_err(scope, &e, &path_str);
                scope.throw_exception(err);
            }
        },
    );

    // ── __fsLutimesSync(path, atime, mtime) ───────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsLutimesSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let path_str = args.get(0).to_rust_string_lossy(scope);
            let atime = args.get(1).number_value(scope).unwrap_or(0.0);
            let mtime = args.get(2).number_value(scope).unwrap_or(0.0);
            let path = PathBuf::from(&path_str);
            if !perms().check(&Capability::FileWrite(path.clone())) {
                let err = perm_err(scope, "write", &path);
                scope.throw_exception(err);
                return;
            }
            let atime_ft =
                filetime::FileTime::from_unix_time(atime as i64, ((atime.fract()) * 1e9) as u32);
            let mtime_ft =
                filetime::FileTime::from_unix_time(mtime as i64, ((mtime.fract()) * 1e9) as u32);
            if let Err(e) = filetime::set_symlink_file_times(&path, atime_ft, mtime_ft) {
                let err = fs_err(scope, &e, &path_str);
                scope.throw_exception(err);
            }
        },
    );

    // ── __fsFutimesSync(fd, atime, mtime) ─────────────────────────────────────
    set_fn(
        scope,
        globals,
        "__fsFutimesSync",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let fd = args.get(0).int32_value(scope).unwrap_or(-1);
            let atime = args.get(1).number_value(scope).unwrap_or(0.0);
            let mtime = args.get(2).number_value(scope).unwrap_or(0.0);
            #[cfg(unix)]
            {
                let mut table = fdt().lock().unwrap();
                if let Some(file) = table.get_mut(fd) {
                    let atime_ft = filetime::FileTime::from_unix_time(
                        atime as i64,
                        ((atime.fract()) * 1e9) as u32,
                    );
                    let mtime_ft = filetime::FileTime::from_unix_time(
                        mtime as i64,
                        ((mtime.fract()) * 1e9) as u32,
                    );
                    if let Err(e) =
                        filetime::set_file_handle_times(file, Some(atime_ft), Some(mtime_ft))
                    {
                        let err = fs_err(scope, &e, &format!("fd {}", fd));
                        scope.throw_exception(err);
                    }
                } else {
                    let err = js_err(scope, format!("EBADF: bad file descriptor: {}", fd));
                    scope.throw_exception(err);
                }
            }
            #[cfg(not(unix))]
            {
                let _ = (fd, atime, mtime);
            }
        },
    );

    // ── JS wrapper: globalThis.fs ─────────────────────────────────────────────
    let js_src = r#"
    (function() {
        var F_OK = 0, R_OK = 4, W_OK = 2, X_OK = 1;

        function parseStat(json) {
            var s = JSON.parse(json);
            // Capture booleans from JSON before overwriting properties with methods
            var _isFile = s.isFile, _isDir = s.isDirectory, _isSym = s.isSymbolicLink;
            s.isFile         = function() { return _isFile; };
            s.isDirectory    = function() { return _isDir; };
            s.isSymbolicLink = function() { return _isSym; };
            s.isFIFO         = function() { return false; };
            s.isBlockDevice  = function() { return false; };
            s.isCharacterDevice = function() { return false; };
            s.isSocket       = function() { return false; };
            s.mtime      = new Date(s.mtimeMs);
            s.atime      = new Date(s.atimeMs);
            s.ctime      = new Date(s.ctimeMs);
            s.birthtime  = new Date(s.birthtimeMs);
            return s;
        }

        // Node's fs functions accept a path as string | Buffer | URL (file://).
        // Our native __fsXxx bridges only take strings, so coerce here once
        // instead of at every call site.
        function __fsPath(p) {
            if (typeof p === 'string') return p;
            if (typeof URL !== 'undefined' && p instanceof URL) {
                if (p.protocol !== 'file:') throw new TypeError('ERR_INVALID_URL_SCHEME: The URL must be of scheme file');
                return decodeURIComponent(p.pathname);
            }
            if (typeof Buffer !== 'undefined' && Buffer.isBuffer(p)) return p.toString();
            return p;
        }

        function wrapAsync(syncFn) {
            return function() {
                var args = Array.prototype.slice.call(arguments);
                var cb = typeof args[args.length - 1] === 'function' ? args.pop() : null;
                if (args.length) args[0] = __fsPath(args[0]);
                if (cb) {
                    // Invoke via setTimeout to avoid creating a rejected Promise
                    // that triggers spurious PromiseRejectWithNoHandler before
                    // any .catch() can be attached (V8 fires it immediately).
                    setTimeout(function() {
                        try { cb(null, syncFn.apply(null, args)); }
                        catch(e) { cb(e); }
                    }, 0);
                    return;
                }
                return new Promise(function(resolve, reject) {
                    try { resolve(syncFn.apply(null, args)); }
                    catch(e) { reject(e); }
                });
            };
        }

        // Node exposes fs.ReadStream / fs.WriteStream as constructors that
        // createReadStream / createWriteStream delegate to. Packages like
        // graceful-fs call fs.WriteStream.apply(this, args), so they must be
        // callable on a foreign `this` and carry a Writable/Readable prototype.
        function ReadStream(path, opts) {
            if (!(this instanceof ReadStream)) return new ReadStream(path, opts);
            var ropts = (typeof opts === 'object' && opts !== null) ? opts : {};
            var encoding = ropts.encoding;
            if (encoding === 'buffer' || encoding === null) encoding = undefined;
            var Readable = require('stream').Readable;
            Readable.call(this, {
                highWaterMark: ropts.highWaterMark || 65536,
                encoding: encoding,
            });
            this.path = path;
            this.bytesRead = 0;
            this.fd = undefined;
            this._done = false;
            // The permission-checked native open/read happens later, on a
            // setTimeout/interval tick driving this stream — by then the
            // require()-boundary scope wrapper has already reverted. Capture
            // the caller's scope now and re-apply it around every deferred
            // native call so the grant checked is still the stream creator's.
            var __streamScope = (typeof globalThis.__currentCallerScope === 'string')
                ? globalThis.__currentCallerScope : '.';
            var self = this;
            this._read = function(size) {
                if (self._done) return;
                var __prevScope = globalThis.__currentCallerScope;
                globalThis.__currentCallerScope = __streamScope;
                if (typeof __setCallerScope === 'function') __setCallerScope(__streamScope);
                try {
                if (self.fd === undefined) {
                    try {
                        self.fd = __fsFdOpen(self.path, 'r', null);
                        self.emit('open', self.fd);
                    } catch(e) {
                        self.emit('error', e);
                        self._done = true;
                        return;
                    }
                }
                try {
                    var chunkSize = size || 65536;
                    var pos = ropts.start !== undefined ? (ropts.start + self.bytesRead) : undefined;
                    if (ropts.end !== undefined && self.bytesRead > (ropts.end - (ropts.start || 0))) {
                        if (self.fd !== undefined) { try { __fsFdClose(self.fd); } catch(_) {} }
                        self._done = true;
                        self.push(null);
                        return;
                    }
                    var maxRead = ropts.end !== undefined ? Math.min(chunkSize, (ropts.end - (ropts.start || 0) - self.bytesRead + 1)) : chunkSize;
                    if (maxRead <= 0) {
                        if (self.fd !== undefined) { try { __fsFdClose(self.fd); } catch(_) {} }
                        self._done = true;
                        self.push(null);
                        return;
                    }
                    var bytes = __fsFdRead(self.fd, maxRead, pos != null ? pos : null);
                    self.bytesRead += bytes.length;
                    if (bytes.length === 0) {
                        if (self.fd !== undefined) { try { __fsFdClose(self.fd); } catch(_) {} }
                        self._done = true;
                        self.push(null);
                    } else {
                        self.push(Buffer.from(bytes));
                        if (self._readableState && self._readableState.flowing && !self._done) {
                            setTimeout(function() { self._read(65536); }, 0);
                        }
                    }
                } catch(e) {
                    self._done = true;
                    if (self.fd !== undefined) { try { __fsFdClose(self.fd); } catch(_) {} }
                    self.emit('error', e);
                }
                } finally {
                    globalThis.__currentCallerScope = __prevScope;
                    if (typeof __setCallerScope === 'function') __setCallerScope(__prevScope);
                }
            };
            var _onOrig = this.on.bind(this);
            this.on = function(event, fn) {
                _onOrig(event, fn);
                if (event === 'data' && !this._readableState.flowing) {
                    this._readableState.flowing = true;
                    this._read(65536);
                }
                return this;
            };
            this.destroy = function() {
                if (this.fd !== undefined) { try { __fsFdClose(this.fd); this.fd = undefined; } catch(_) {} }
                this._done = true;
                this.emit('close');
                return this;
            };
        }

        function WriteStream(path, opts) {
            if (!(this instanceof WriteStream)) return new WriteStream(path, opts);
            var wopts = (typeof opts === 'object' && opts !== null) ? opts : {};
            var flags = wopts.flags || 'w';
            var Writable = require('stream').Writable;
            Writable.call(this, { highWaterMark: wopts.highWaterMark || 16384 });
            this.path = path;
            this.flags = flags;
            this.mode = wopts.mode;
            this.autoClose = (wopts.autoClose !== undefined) ? wopts.autoClose : true;
            this.bytesWritten = 0;
            this.fd = undefined;
            var __streamScope = (typeof globalThis.__currentCallerScope === 'string')
                ? globalThis.__currentCallerScope : '.';
            var self = this;
            this._write = function(chunk, encoding, callback) {
                var __prevScope = globalThis.__currentCallerScope;
                globalThis.__currentCallerScope = __streamScope;
                if (typeof __setCallerScope === 'function') __setCallerScope(__streamScope);
                try {
                if (self.fd === undefined) {
                    try {
                        self.fd = __fsFdOpen(self.path, self.flags, null);
                        self.emit('open', self.fd);
                    } catch(e) {
                        callback(e);
                        return;
                    }
                }
                try {
                    var data;
                    if (Buffer.isBuffer(chunk)) {
                        data = Array.from(chunk);
                    } else if (chunk instanceof Uint8Array) {
                        data = Array.from(chunk);
                    } else {
                        data = Array.from(new TextEncoder().encode(String(chunk)));
                    }
                    __fsFdWrite(self.fd, data, null);
                    self.bytesWritten += data.length;
                    callback(null);
                } catch(e) {
                    callback(e);
                }
                } finally {
                    globalThis.__currentCallerScope = __prevScope;
                    if (typeof __setCallerScope === 'function') __setCallerScope(__prevScope);
                }
            };
            this._final = function(callback) {
                var __prevScope = globalThis.__currentCallerScope;
                globalThis.__currentCallerScope = __streamScope;
                if (typeof __setCallerScope === 'function') __setCallerScope(__streamScope);
                try {
                    if (this.fd !== undefined) {
                        try { __fsFdClose(this.fd); this.fd = undefined; } catch(_) {}
                    }
                    callback();
                } finally {
                    globalThis.__currentCallerScope = __prevScope;
                    if (typeof __setCallerScope === 'function') __setCallerScope(__prevScope);
                }
            };
            this.destroy = function(err) {
                if (this.fd !== undefined) { try { __fsFdClose(this.fd); this.fd = undefined; } catch(_) {} }
                if (err) this.emit('error', err);
                this.emit('close');
                return this;
            };
        }

        var fs = {
            constants: { F_OK: F_OK, R_OK: R_OK, W_OK: W_OK, X_OK: X_OK, COPYFILE_EXCL: 1 },

            // ── sync ────────────────────────────────────────────────────────────
            existsSync: function(p) { return __fsExistsSync(__fsPath(p)); },
            readFileSync: function(p, opts) {
                p = __fsPath(p);
                var enc = opts && (typeof opts === 'string' ? opts : opts.encoding);
                if (!enc) {
                    // No encoding → return Buffer (Node.js default)
                    var bytes = JSON.parse(__fsReadFileBytesSync(p));
                    return Buffer.from(bytes);
                }
                var encLow = enc.toLowerCase();
                if (encLow === 'utf8' || encLow === 'utf-8') return __fsReadFileSync(p);
                // Other encoding: read bytes and decode
                var bytes = JSON.parse(__fsReadFileBytesSync(p));
                return Buffer.from(bytes).toString(enc);
            },
            writeFileSync: function(p, data, opts) {
                p = __fsPath(p);
                if (typeof data === 'string') return __fsWriteFileSync(p, data);
                // Binary: write raw bytes without UTF-8 conversion
                var src = data instanceof Uint8Array ? data :
                    (data && data.type === 'Buffer' ? new Uint8Array(data.data) : new Uint8Array(0));
                var arr = [];
                for (var _i = 0; _i < src.length; _i++) arr.push(src[_i]);
                return __fsWriteFileBytesSync(p, arr);
            },
            appendFileSync: function(p, data) {
                p = __fsPath(p);
                if (typeof data === 'string') return __fsAppendFileSync(p, data);
                var bytes = data instanceof Uint8Array ? data : new Uint8Array(0);
                var fd = __fsFdOpen(p, 'a', null);
                try { __fsFdWrite(fd, bytes, null); }
                finally { try { __fsFdClose(fd); } catch(_) {} }
            },
            readdirSync: function(p, opts) {
                p = __fsPath(p);
                var names = JSON.parse(__fsReaddirSync(p));
                if (opts && opts.withFileTypes) {
                    return names.map(function(n) {
                        return { name: n, isFile: function() { try { return JSON.parse(__fsStatSync(p + '/' + n, 'true')).isFile; } catch(e) { return false; } }, isDirectory: function() { try { return JSON.parse(__fsStatSync(p + '/' + n, 'true')).isDirectory; } catch(e) { return false; } }, isSymbolicLink: function() { return false; } };
                    });
                }
                return names;
            },
            mkdirSync: function(p, opts) {
                p = __fsPath(p);
                try { __fsMkdirSync(p); }
                catch(e) {
                    if (!(opts && opts.recursive && e && (e.code === 'EEXIST' || (e.message && e.message.indexOf('exists') !== -1)))) throw e;
                }
            },
            rmSync: function(p, opts) {
                p = __fsPath(p);
                try { __fsRmSync(p); }
                catch(e) {
                    if (!(opts && opts.force && e && (e.message && e.message.indexOf('ENOENT') !== -1))) throw e;
                }
            },
            unlinkSync: function(p) { return __fsUnlinkSync(__fsPath(p)); },
            renameSync: function(f, t) { return __fsRenameSync(__fsPath(f), __fsPath(t)); },
            copyFileSync: function(s, d) { return __fsCopyFileSync(__fsPath(s), __fsPath(d)); },
            cpSync: function(s, d, _opts) { return __fsCpSync(__fsPath(s), __fsPath(d)); },
            chmodSync: function(p, m) { return __fsChmodSync(__fsPath(p), String(m)); },
            symlinkSync: function(target, p) { return __fsSymlinkSync(target, __fsPath(p)); },
            statSync: function(p) { return parseStat(__fsStatSync(__fsPath(p), 'true')); },
            lstatSync: function(p) { return parseStat(__fsStatSync(__fsPath(p), 'false')); },
            realpathSync: (function() { var fn = function(p) { return __fsRealpathSync(__fsPath(p)); }; fn.native = fn; return fn; })(),
            accessSync: function(p, mode) {
                var result = __fsAccessSync(__fsPath(p), String(mode === undefined ? 0 : mode));
                if (result !== 'ok') throw new Error(result);
            },

            // ── async (callback + promise) ──────────────────────────────────────
            readFile: function(p, opts, cb) {
                if (typeof opts === 'function') { cb = opts; opts = null; }
                if (cb) {
                    // Call callback directly via setTimeout to match Node.js async semantics.
                    // Avoids creating a rejected Promise that triggers spurious unhandled-rejection events.
                    setTimeout(function() {
                        try { cb(null, fs.readFileSync(p, opts || null)); } catch(e) { cb(e); }
                    }, 0);
                    return;
                }
                return new Promise(function(resolve, reject) {
                    try { resolve(fs.readFileSync(p, opts || null)); } catch(e) { reject(e); }
                });
            },
            writeFile: function(p, data, opts, cb) {
                if (typeof opts === 'function') { cb = opts; opts = {}; }
                if (cb) { setTimeout(function() { try { fs.writeFileSync(p, data, opts); cb(null); } catch(e) { cb(e); } }, 0); return; }
                return new Promise(function(resolve, reject) {
                    try { fs.writeFileSync(p, data, opts); resolve(); } catch(e) { reject(e); }
                });
            },
            appendFile: function(p, data, opts, cb) {
                if (typeof opts === 'function') { cb = opts; opts = {}; }
                if (cb) { setTimeout(function() { try { fs.appendFileSync(p, data); cb(null); } catch(e) { cb(e); } }, 0); return; }
                return new Promise(function(resolve, reject) {
                    try { fs.appendFileSync(p, data); resolve(); } catch(e) { reject(e); }
                });
            },
            readdir:     wrapAsync(function(p, opts) {
                var names = JSON.parse(__fsReaddirSync(p));
                if (opts && opts.withFileTypes) {
                    return names.map(function(n) {
                        var stat; try { stat = JSON.parse(__fsStatSync(p + '/' + n, 'false')); } catch(e) { stat = null; }
                        return { name: n, path: p, isFile: function() { return stat ? stat.isFile : true; }, isDirectory: function() { return stat ? stat.isDirectory : false; }, isSymbolicLink: function() { return stat ? stat.isSymbolicLink : false; }, isBlockDevice: function() { return false; }, isCharacterDevice: function() { return false; }, isFIFO: function() { return false; }, isSocket: function() { return false; } };
                    });
                }
                return names;
            }),
            mkdir: function(p, opts, cb) {
                if (typeof opts === 'function') { cb = opts; opts = null; }
                p = __fsPath(p);
                var recursive = opts && opts.recursive;
                if (cb) {
                    setTimeout(function() {
                        try { __fsMkdirSync(p); cb(null); }
                        catch(e) {
                            if (recursive && e && e.message && (e.message.indexOf('exists') !== -1 || e.message.indexOf('EEXIST') !== -1)) cb(null);
                            else cb(e);
                        }
                    }, 0);
                    return;
                }
                return new Promise(function(resolve, reject) {
                    try { __fsMkdirSync(p); resolve(); }
                    catch(e) {
                        if (recursive && e && e.message && (e.message.indexOf('exists') !== -1 || e.message.indexOf('EEXIST') !== -1)) resolve();
                        else reject(e);
                    }
                });
            },
            rm: function(p, opts, cb) {
                if (typeof opts === 'function') { cb = opts; opts = null; }
                p = __fsPath(p);
                var force = opts && opts.force;
                if (cb) {
                    setTimeout(function() {
                        try { __fsRmSync(p); cb(null); }
                        catch(e) {
                            if (force && e && e.message && e.message.indexOf('ENOENT') !== -1) cb(null);
                            else cb(e);
                        }
                    }, 0);
                    return;
                }
                return new Promise(function(resolve, reject) {
                    try { __fsRmSync(p); resolve(); }
                    catch(e) {
                        if (force && e && e.message && e.message.indexOf('ENOENT') !== -1) resolve();
                        else reject(e);
                    }
                });
            },
            unlink:      wrapAsync(function(p) { return __fsUnlinkSync(p); }),
            rename:      wrapAsync(function(f, t) { return __fsRenameSync(f, t); }),
            copyFile:    wrapAsync(function(s, d) { return __fsCopyFileSync(s, d); }),
            cp:          wrapAsync(function(s, d) { return __fsCpSync(s, d); }),
            chmod:       wrapAsync(function(p, m) { return __fsChmodSync(p, String(m)); }),
            symlink:     wrapAsync(function(target, p) { return __fsSymlinkSync(target, p); }),
            stat:        wrapAsync(function(p) { return parseStat(__fsStatSync(p, 'true')); }),
            lstat:       wrapAsync(function(p) { return parseStat(__fsStatSync(p, 'false')); }),
            realpath:    wrapAsync(function(p) { return __fsRealpathSync(p); }),
            access: function(p, mode, cb) {
                p = __fsPath(p);
                if (typeof mode === 'function') { cb = mode; mode = 0; }
                var result = __fsAccessSync(p, String(mode === undefined ? 0 : mode));
                var err = result === 'ok' ? null : new Error(result);
                if (cb) { setTimeout(function() { cb(err); }, 0); return; }
                return err ? Promise.reject(err) : Promise.resolve();
            },

            // ── fd-based operations ─────────────────────────────────────────────
            open: function(path, flags, mode, cb) {
                path = __fsPath(path);
                if (typeof mode === 'function') { cb = mode; mode = 0o666; }
                if (typeof flags === 'number') flags = ['r','w','r+','w','w','a','a+'][flags] || 'r';
                var result;
                try { result = __fsFdOpen(path, flags, mode); } catch(e) { if (cb) cb(e); return; }
                if (cb) setTimeout(function() { cb(null, result); }, 0);
                return result;
            },
            openSync: function(path, flags, mode) {
                path = __fsPath(path);
                if (typeof flags === 'number') flags = ['r','w','r+','w','w','a','a+'][flags] || 'r';
                return __fsFdOpen(path, flags, mode || 0o666);
            },
            close: function(fd, cb) {
                try { __fsFdClose(fd); } catch(e) { if (cb) cb(e); return; }
                if (cb) setTimeout(function() { cb(null); }, 0);
            },
            closeSync: function(fd) { if (fd > 2) __fsFdClose(fd); },
            read: function(fd, buffer, offset, length, position, cb) {
                try {
                    var bytes = __fsFdRead(fd, length, position >= 0 ? position : null);
                    var bytesRead = bytes.length;
                    if (buffer instanceof Uint8Array || buffer instanceof Buffer) {
                        for (var i = 0; i < bytesRead; i++) buffer[offset + i] = bytes[i];
                    }
                    if (cb) setTimeout(function() { cb(null, bytesRead, buffer); }, 0);
                } catch(e) { if (cb) cb(e); }
            },
            readSync: function(fd, buffer, offset, length, position) {
                var bytes = __fsFdRead(fd, length, position >= 0 ? position : null);
                var bytesRead = bytes.length;
                if (buffer instanceof Uint8Array || buffer instanceof Buffer) {
                    for (var i = 0; i < bytesRead; i++) buffer[offset + i] = bytes[i];
                }
                return bytesRead;
            },
            write: function(fd, buffer, offset, length, position, cb) {
                if (typeof offset === 'function') { cb = offset; offset = 0; length = null; position = null; }
                if (typeof length === 'function') { cb = length; length = null; position = null; }
                if (typeof position === 'function') { cb = position; position = null; }
                try {
                    var data;
                    if (typeof buffer === 'string') {
                        data = Array.from(new TextEncoder().encode(buffer));
                    } else {
                        var start = offset || 0;
                        var end   = length != null ? start + length : buffer.length;
                        data = Array.from(buffer.slice(start, end));
                    }
                    var written = __fsFdWrite(fd, data, position >= 0 ? position : null);
                    if (cb) setTimeout(function() { cb(null, written, buffer); }, 0);
                } catch(e) { if (cb) cb(e); }
            },
            writeSync: function(fd, buffer, offset, length, position) {
                var data;
                if (typeof buffer === 'string') {
                    data = new TextEncoder().encode(buffer);
                } else {
                    var start = offset || 0;
                    var end   = length != null ? start + length : buffer.length;
                    data = buffer.slice ? buffer.slice(start, end) : buffer;
                }
                if (fd <= 2) {
                    // stdin/stdout/stderr: decode and write to console
                    var text = typeof data === 'string' ? data : new TextDecoder().decode(data);
                    if (fd === 2) process.stderr.write ? process.stderr.write(text) : console.error(text);
                    else if (fd === 1) process.stdout.write ? process.stdout.write(text) : console.log(text);
                    return data.byteLength || data.length || 0;
                }
                return __fsFdWrite(fd, Array.from(data), position != null && position >= 0 ? position : null);
            },
            fstat: function(fd, cb) {
                if (fd <= 2) {
                    var s = {dev:0,mode:0x21a4,nlink:1,uid:0,gid:0,rdev:fd,blksize:4096,ino:fd,size:0,blocks:0,atime:new Date(0),mtime:new Date(0),ctime:new Date(0),birthtime:new Date(0),atimeMs:0,mtimeMs:0,ctimeMs:0,birthtimeMs:0,isFile:function(){return false;},isDirectory:function(){return false;},isBlockDevice:function(){return false;},isCharacterDevice:function(){return true;},isSymbolicLink:function(){return false;},isFIFO:function(){return false;},isSocket:function(){return false;}};
                    if (cb) setTimeout(function() { cb(null, s); }, 0); else return s;
                    return;
                }
                try {
                    var s = parseStat(__fsFdStat(fd));
                    if (cb) setTimeout(function() { cb(null, s); }, 0);
                    else return s;
                } catch(e) { if (cb) cb(e); }
            },
            fstatSync: function(fd) {
                if (fd <= 2) return {dev:0,mode:0x21a4,nlink:1,uid:0,gid:0,rdev:fd,blksize:4096,ino:fd,size:0,blocks:0,atime:new Date(0),mtime:new Date(0),ctime:new Date(0),birthtime:new Date(0),atimeMs:0,mtimeMs:0,ctimeMs:0,birthtimeMs:0,isFile:function(){return false;},isDirectory:function(){return false;},isBlockDevice:function(){return false;},isCharacterDevice:function(){return true;},isSymbolicLink:function(){return false;},isFIFO:function(){return false;},isSocket:function(){return false;}};
                return parseStat(__fsFdStat(fd));
            },
            fsync: function(fd, cb) { if (cb) setTimeout(function() { cb(null); }, 0); },
            fsyncSync: function(fd) {},
            fdatasync: function(fd, cb) { if (cb) setTimeout(function() { cb(null); }, 0); },
            fdatasyncSync: function(fd) {},
            ftruncate: function(fd, len, cb) {
                if (typeof len === 'function') { cb = len; len = 0; }
                if (cb) setTimeout(function() { cb(null); }, 0);
            },
            ftruncateSync: function(fd, len) {},

            // ── mkdtemp ─────────────────────────────────────────────────────────
            mkdtemp: function(prefix, opts, cb) {
                if (typeof opts === 'function') { cb = opts; }
                try {
                    var dir = __fsMkdtemp(prefix);
                    if (cb) setTimeout(function() { cb(null, dir); }, 0);
                    else return Promise.resolve(dir);
                } catch(e) {
                    if (cb) cb(e);
                    else return Promise.reject(e);
                }
            },
            mkdtempSync: function(prefix) { return __fsMkdtemp(prefix); },

            // ── truncate ────────────────────────────────────────────────────────
            truncate: wrapAsync(function(p, len) {
                var fd = __fsFdOpen(p, 'r+', 0o666);
                __fsFdClose(fd);
            }),
            truncateSync: function(p, _len) {},

            // ── lutimes / lchown / chown / utimes ─────────────────────────────
            lutimes: function(p, at, mt, cb) {
                try {
                    var atSec = (at instanceof Date) ? at.getTime() / 1000 : Number(at);
                    var mtSec = (mt instanceof Date) ? mt.getTime() / 1000 : Number(mt);
                    __fsLutimesSync(p, atSec, mtSec);
                    if (cb) setTimeout(function() { cb(null); }, 0);
                } catch(e) { if (cb) cb(e); }
            },
            lutimesSync: function(p, at, mt) {
                var atSec = (at instanceof Date) ? at.getTime() / 1000 : Number(at);
                var mtSec = (mt instanceof Date) ? mt.getTime() / 1000 : Number(mt);
                __fsLutimesSync(p, atSec, mtSec);
            },
            utimes: function(p, at, mt, cb) {
                try {
                    var atSec = (at instanceof Date) ? at.getTime() / 1000 : Number(at);
                    var mtSec = (mt instanceof Date) ? mt.getTime() / 1000 : Number(mt);
                    __fsUtimesSync(p, atSec, mtSec);
                    if (cb) setTimeout(function() { cb(null); }, 0);
                } catch(e) { if (cb) cb(e); }
            },
            utimesSync: function(p, at, mt) {
                var atSec = (at instanceof Date) ? at.getTime() / 1000 : Number(at);
                var mtSec = (mt instanceof Date) ? mt.getTime() / 1000 : Number(mt);
                __fsUtimesSync(p, atSec, mtSec);
            },
            futimes: function(fd, at, mt, cb) {
                try {
                    var atSec = (at instanceof Date) ? at.getTime() / 1000 : Number(at);
                    var mtSec = (mt instanceof Date) ? mt.getTime() / 1000 : Number(mt);
                    __fsFutimesSync(fd, atSec, mtSec);
                    if (cb) setTimeout(function() { cb(null); }, 0);
                } catch(e) { if (cb) cb(e); }
            },
            futimesSync: function(fd, at, mt) {
                var atSec = (at instanceof Date) ? at.getTime() / 1000 : Number(at);
                var mtSec = (mt instanceof Date) ? mt.getTime() / 1000 : Number(mt);
                __fsFutimesSync(fd, atSec, mtSec);
            },
            lchown: function(p, uid, gid, cb) {
                try { __fsLchownSync(p, uid, gid); if (cb) setTimeout(function() { cb(null); }, 0); }
                catch(e) { if (cb) cb(e); }
            },
            lchownSync: function(p, uid, gid) { __fsLchownSync(p, uid, gid); },
            chown: function(p, uid, gid, cb) {
                try { __fsChownSync(p, uid, gid); if (cb) setTimeout(function() { cb(null); }, 0); }
                catch(e) { if (cb) cb(e); }
            },
            chownSync: function(p, uid, gid) { __fsChownSync(p, uid, gid); },
            fchown: function(fd, uid, gid, cb) {
                try { __fsFchownSync(fd, uid, gid); if (cb) setTimeout(function() { cb(null); }, 0); }
                catch(e) { if (cb) cb(e); }
            },
            fchownSync: function(fd, uid, gid) { __fsFchownSync(fd, uid, gid); },
            fchmod: function(fd, mode, cb) { if (cb) setTimeout(function() { cb(null); }, 0); },
            fchmodSync: function() {},
            link: function(src, dest, cb) {
                try {
                    __fsCopyFileSync(src, dest);
                    if (cb) setTimeout(function() { cb(null); }, 0);
                } catch(e) { if (cb) cb(e); }
            },
            linkSync: function(src, dest) { __fsCopyFileSync(src, dest); },
            readlink: function(p, opts, cb) {
                if (typeof opts === 'function') { cb = opts; opts = {}; }
                try {
                    var result = __fsReadlinkSync(p);
                    if (cb) setTimeout(function() { cb(null, result); }, 0);
                    else return Promise.resolve(result);
                } catch(e) {
                    if (cb) cb(e);
                    else return Promise.reject(e);
                }
            },
            readlinkSync: function(p) { return __fsReadlinkSync(p); },

            // ── opendir ─────────────────────────────────────────────────────────
            opendir: function(p, opts, cb) {
                if (typeof opts === 'function') { cb = opts; }
                try {
                    var names = JSON.parse(__fsReaddirSync(p));
                    var idx = 0;
                    var dir = {
                        path: p,
                        read: function(cb2) {
                            var entry = idx < names.length ? { name: names[idx++], isFile: function() { return true; }, isDirectory: function() { return false; }, isSymbolicLink: function() { return false; } } : null;
                            if (cb2) setTimeout(function() { cb2(null, entry); }, 0);
                            else return Promise.resolve(entry);
                        },
                        close: function(cb2) { if (cb2) setTimeout(function() { cb2(null); }, 0); else return Promise.resolve(); },
                        [Symbol.asyncIterator]: function() {
                            var self = this;
                            return { next: function() {
                                return self.read().then(function(e) { return e ? { value: e, done: false } : { value: undefined, done: true }; });
                            }};
                        }
                    };
                    if (cb) setTimeout(function() { cb(null, dir); }, 0);
                    else return Promise.resolve(dir);
                } catch(e) {
                    if (cb) cb(e);
                    else return Promise.reject(e);
                }
            },
            opendirSync: function(p) {
                var names = JSON.parse(__fsReaddirSync(p));
                var idx = 0;
                return {
                    path: p,
                    readSync: function() {
                        if (idx >= names.length) return null;
                        var n = names[idx++];
                        return { name: n, isFile: function() { return true; }, isDirectory: function() { return false; }, isSymbolicLink: function() { return false; } };
                    },
                    closeSync: function() {},
                    [Symbol.iterator]: function() { var self = this; return { next: function() { var e = self.readSync(); return e ? { value: e, done: false } : { value: undefined, done: true }; } }; }
                };
            },

            // ── createReadStream (proper Readable) ──────────────────────────────
            createReadStream: function(path, opts) {
                return new ReadStream(path, opts);
            },

            // ── createWriteStream (proper Writable) ──────────────────────────────
            createWriteStream: function(path, opts) {
                return new WriteStream(path, opts);
            },

            // ── watch — backed by __fsWatchCreate / __fsWatchNext / __fsWatchClose ──
            watch: function(path, opts, listener) {
                if (typeof opts === 'function') { listener = opts; opts = {}; }
                opts = opts || {};
                var recursive = !!(opts.recursive);

                var EventEmitter = require('events');
                var watcher = new EventEmitter();
                watcher.filename = path;

                // Create OS-level watcher synchronously (returns an integer id).
                // Throws synchronously on permission errors or invalid paths,
                // matching Node.js behaviour.
                var watcherId = __fsWatchCreate(path, recursive);

                var closed = false;

                // __fsWatchNext is a non-blocking poll (returns null when no
                // event is ready), so drive it on an interval.
                var pollTimer = setInterval(function() {
                    if (closed) return;
                    var json;
                    while ((json = __fsWatchNext(watcherId)) !== null && json !== undefined) {
                        var ev;
                        try { ev = JSON.parse(json); } catch (e) { continue; }
                        if (typeof listener === 'function') {
                            listener(ev.eventType, ev.filename);
                        }
                        watcher.emit('change', ev.eventType, ev.filename);
                        if (closed) return;
                    }
                }, 10);

                watcher.close = function() {
                    if (closed) return;
                    closed = true;
                    clearInterval(pollTimer);
                    __fsWatchClose(watcherId);
                };

                return watcher;
            },

            // ── watchFile — inotify-backed (reuses __fsWatchCreate/__fsWatchNext) ──
            // Node.js watchFile uses stat polling, but OS notifications are both
            // faster and more reliable. We use the same watcher infrastructure as
            // fs.watch, comparing stats before and after each event so the
            // callback receives proper (curr, prev) stat objects.
            watchFile: function(filename, opts, listener) {
                if (typeof opts === 'function') { listener = opts; opts = {}; }
                var closed = false;

                // Capture initial stat for change comparison.
                var prevStat;
                try { prevStat = parseStat(__fsStatSync(filename, false)); }
                catch(e) { prevStat = { mtimeMs: 0, size: 0, isFile: function(){return false;}, isDirectory: function(){return false;} }; }

                var watcherId = __fsWatchCreate(filename, false);

                var pollTimer = setInterval(function() {
                    if (closed) return;
                    var json;
                    while ((json = __fsWatchNext(watcherId)) !== null && json !== undefined) {
                        var currStat;
                        try { currStat = parseStat(__fsStatSync(filename, false)); }
                        catch(e) { currStat = { mtimeMs: 0, size: 0, isFile: function(){return false;}, isDirectory: function(){return false;} }; }
                        if (typeof listener === 'function') listener(currStat, prevStat);
                        prevStat = currStat;
                        if (closed) return;
                    }
                }, 10);

                return {
                    stop: function() {
                        if (closed) return;
                        closed = true;
                        clearInterval(pollTimer);
                        __fsWatchClose(watcherId);
                    }
                };
            },

            // ── unwatchFile ───────────────────────────────────────────────────────
            unwatchFile: function(filename) {
                // No-op in this impl (watchFile handle tracks its own cleanup).
            },

            // ── promises API (mirror of async) ──────────────────────────────────
            promises: {}
        };

        // rmdir is a deprecated alias for rm (recursive dir removal)
        fs.rmdir = function(path, opts, cb) {
            if (typeof opts === 'function') { cb = opts; opts = {}; }
            fs.rm(path, Object.assign({ recursive: true, force: true }, opts || {}), cb);
        };
        fs.rmdirSync = function(path, opts) {
            fs.rmSync(path, Object.assign({ recursive: true, force: true }, opts || {}));
        };
        fs.ReadStream = ReadStream;
        fs.WriteStream = WriteStream;

        // ponytail: only start fsTrace interval if explicitly enabled — unconditional
        // setInterval keeps the event loop alive forever even when idle.
        if (process.env && process.env['3VA_FS_TRACE']) {
            process.__fsTrace = [];
            try {
                setInterval(function() {
                    if (process.__fsTrace && process.__fsTrace.length) {
                        console.error('[fsTrace] ' + process.__fsTrace.slice(-8).join(' | '));
                        process.__fsTrace.length = 0;
                    }
                }, 5000);
            } catch(e) {}
        }

        // Build fs.promises from fs async methods
        ['readFile','writeFile','appendFile','readdir','mkdir','rm','rmdir','unlink','rename',
         'copyFile','chmod','symlink','stat','lstat','realpath','access'].forEach(function(fn) {
            fs.promises[fn] = function() {
                var args = Array.prototype.slice.call(arguments);
                return new Promise(function(resolve, reject) {
                    args.push(function(err, val) { if (err) reject(err); else resolve(val); });
                    fs[fn].apply(fs, args);
                });
            };
        });
        // node:fs/promises re-exports the same `constants` as node:fs.
        fs.promises.constants = fs.constants;

        globalThis.fs = fs;
        // Re-register in require cache so require('fs') reflects the full object
        if (globalThis.__requireCache) {
            globalThis.__requireCache['fs'] = fs;
            globalThis.__requireCache['node:fs'] = fs;
            globalThis.__requireCache['fs/promises'] = fs.promises;
            globalThis.__requireCache['node:fs/promises'] = fs.promises;
        }
    })();
    "#;
    let source = v8::String::new(scope, js_src).unwrap();
    let script = v8::Script::compile(scope, source, None).unwrap();
    let _ = script.run(scope);

    Ok(())
}
