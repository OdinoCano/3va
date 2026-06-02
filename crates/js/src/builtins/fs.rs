use rquickjs::{Ctx, Function, Result, function::Rest};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use vvva_permissions::{Capability, PermissionState};

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

fn js_err<'js>(ctx: &Ctx<'js>, msg: String) -> rquickjs::Error {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    match ctx.eval::<rquickjs::Value, _>(format!("new Error(\"{}\")", escaped).as_str()) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
    }
}

fn perm_err<'js>(ctx: &Ctx<'js>, flag: &str, path: &std::path::Path) -> rquickjs::Error {
    js_err(
        ctx,
        format!(
            "Permission denied: --allow-{}={} is required",
            flag,
            path.display()
        ),
    )
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
    let mode: u32 = if meta.permissions().readonly() { 0o444 } else { 0o644 };
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

pub fn inject_fs(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();
    let fd_table: Arc<Mutex<FdTable>> = Arc::new(Mutex::new(FdTable::new()));

    // ── __fsFdOpen(path, flags_str) -> fd ─────────────────────────────────────
    {
        let perms = permissions.clone();
        let fdt = fd_table.clone();
        globals.set(
            "__fsFdOpen",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>,
                      path_str: String,
                      flags: String,
                      _mode: Option<i32>|
                      -> Result<i32> {
                    let path = PathBuf::from(&path_str);
                    let needs_write = matches!(
                        flags.as_str(),
                        "w" | "w+" | "wx" | "wx+" | "a" | "a+" | "ax" | "ax+" | "r+" | "rs+"
                    );
                    if needs_write {
                        if !perms.check(&Capability::FileWrite(path.clone())) {
                            return Err(perm_err(&ctx, "write", &path));
                        }
                    } else if !perms.check(&Capability::FileRead(path.clone())) {
                        return Err(perm_err(&ctx, "read", &path));
                    }
                    let file = flags_to_open_options(&flags)
                        .open(&path)
                        .map_err(|e| js_err(&ctx, format!("ENOENT: open '{}': {}", path_str, e)))?;
                    Ok(fdt.lock().unwrap().insert(file))
                },
            )?,
        )?;
    }

    // ── __fsFdClose(fd) ───────────────────────────────────────────────────────
    {
        let fdt = fd_table.clone();
        globals.set(
            "__fsFdClose",
            Function::new(ctx.clone(), move |ctx: Ctx<'_>, fd: i32| -> Result<()> {
                if !fdt.lock().unwrap().remove(fd) {
                    return Err(js_err(&ctx, format!("EBADF: bad file descriptor {fd}")));
                }
                Ok(())
            })?,
        )?;
    }

    // ── __fsFdRead(fd, length, position) -> Vec<u8> ───────────────────────────
    {
        let fdt = fd_table.clone();
        globals.set(
            "__fsFdRead",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>,
                      fd: i32,
                      length: usize,
                      position: Option<i64>|
                      -> Result<Vec<u8>> {
                    let mut table = fdt.lock().unwrap();
                    let file = table
                        .get_mut(fd)
                        .ok_or_else(|| js_err(&ctx, format!("EBADF: bad file descriptor {fd}")))?;
                    if let Some(pos) = position {
                        file.seek(SeekFrom::Start(pos as u64))
                            .map_err(|e| js_err(&ctx, e.to_string()))?;
                    }
                    let mut buf = vec![0u8; length.min(65536)];
                    let n = file
                        .read(&mut buf)
                        .map_err(|e| js_err(&ctx, e.to_string()))?;
                    buf.truncate(n);
                    Ok(buf)
                },
            )?,
        )?;
    }

    // ── __fsFdWrite(fd, data, position) -> bytes_written ─────────────────────
    {
        let fdt = fd_table.clone();
        globals.set(
            "__fsFdWrite",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>,
                      fd: i32,
                      data: Vec<u8>,
                      position: Option<i64>|
                      -> Result<usize> {
                    let mut table = fdt.lock().unwrap();
                    let file = table
                        .get_mut(fd)
                        .ok_or_else(|| js_err(&ctx, format!("EBADF: bad file descriptor {fd}")))?;
                    if let Some(pos) = position {
                        file.seek(SeekFrom::Start(pos as u64))
                            .map_err(|e| js_err(&ctx, e.to_string()))?;
                    }
                    file.write_all(&data)
                        .map_err(|e| js_err(&ctx, e.to_string()))?;
                    Ok(data.len())
                },
            )?,
        )?;
    }

    // ── __fsFdStat(fd) -> JSON stat ───────────────────────────────────────────
    {
        let fdt = fd_table.clone();
        globals.set(
            "__fsFdStat",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, fd: i32| -> Result<String> {
                    let mut table = fdt.lock().unwrap();
                    let file = table
                        .get_mut(fd)
                        .ok_or_else(|| js_err(&ctx, format!("EBADF: bad file descriptor {fd}")))?;
                    let meta = file.metadata().map_err(|e| js_err(&ctx, e.to_string()))?;
                    let json = stat_meta_to_json(&meta);
                    Ok(json)
                },
            )?,
        )?;
    }

    // ── __fsMkdtemp(prefix) -> String ─────────────────────────────────────────
    {
        let perms = permissions.clone();
        globals.set(
            "__fsMkdtemp",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, prefix: String| -> Result<String> {
                    let path = PathBuf::from(&prefix);
                    let parent = path.parent().unwrap_or(std::path::Path::new("/tmp"));
                    if !perms.check(&Capability::FileWrite(parent.to_path_buf())) {
                        return Err(perm_err(&ctx, "write", parent));
                    }
                    // Create a temp dir with a random suffix
                    let unique = format!("{}{}", prefix, std::process::id());
                    std::fs::create_dir_all(&unique).map_err(|e| js_err(&ctx, e.to_string()))?;
                    Ok(unique)
                },
            )?,
        )?;
    }

    // ── __fsReadFileSync(path) -> String ──────────────────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsReadFileSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<String> {
                let path_str = args
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsReadFileSync() requires a path".into()))?;
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileRead(path.clone())) {
                    return Err(perm_err(&ctx, "read", &path));
                }
                std::fs::read_to_string(&path)
                    .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))
            },
        )?,
    )?;

    // ── __fsReadFileBytesSync(path) -> JSON array of byte values ──────────────
    let perms = permissions.clone();
    globals.set(
        "__fsReadFileBytesSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<String> {
                let path_str = args.0.into_iter().next().ok_or_else(|| {
                    js_err(&ctx, "__fsReadFileBytesSync() requires a path".into())
                })?;
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileRead(path.clone())) {
                    return Err(perm_err(&ctx, "read", &path));
                }
                let bytes = std::fs::read(&path)
                    .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))?;
                let arr: Vec<u8> = bytes;
                Ok(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
            },
        )?,
    )?;

    // ── __fsWriteFileSync(path, content) ──────────────────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsWriteFileSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<()> {
                let mut it = args.0.into_iter();
                let path_str = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsWriteFileSync() requires a path".into()))?;
                let content = it.next().unwrap_or_default();
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileWrite(path.clone())) {
                    return Err(perm_err(&ctx, "write", &path));
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                std::fs::write(&path, content)
                    .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))
            },
        )?,
    )?;

    // ── __fsAppendFileSync(path, content) ─────────────────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsAppendFileSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<()> {
                let mut it = args.0.into_iter();
                let path_str = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsAppendFileSync() requires a path".into()))?;
                let content = it.next().unwrap_or_default();
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileWrite(path.clone())) {
                    return Err(perm_err(&ctx, "write", &path));
                }
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))?;
                file.write_all(content.as_bytes())
                    .map_err(|e| js_err(&ctx, format!("{}: '{}'", e, path_str)))
            },
        )?,
    )?;

    // ── __fsExistsSync(path) -> bool ──────────────────────────────────────────
    globals.set(
        "__fsExistsSync",
        Function::new(ctx.clone(), |args: Rest<String>| -> bool {
            args.0
                .into_iter()
                .next()
                .map(|p| PathBuf::from(p).exists())
                .unwrap_or(false)
        })?,
    )?;

    // ── __fsStatSync(path, follow_symlinks) -> JSON ───────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsStatSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<String> {
                let mut it = args.0.into_iter();
                let path_str = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsStatSync() requires a path".into()))?;
                let follow = it.next().map(|s| s != "false").unwrap_or(true);
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileRead(path.clone())) {
                    return Err(perm_err(&ctx, "read", &path));
                }
                let meta = if follow {
                    std::fs::metadata(&path)
                } else {
                    std::fs::symlink_metadata(&path)
                }
                .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))?;
                Ok(stat_meta_to_json(&meta))
            },
        )?,
    )?;

    // ── __fsAccessSync(path, mode) -> "ok" | error message ───────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsAccessSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<String> {
                let mut it = args.0.into_iter();
                let path_str = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsAccessSync() requires a path".into()))?;
                let mode_str = it.next().unwrap_or_else(|| "0".to_string());
                let mode: u32 = mode_str.parse().unwrap_or(0);
                let path = PathBuf::from(&path_str);

                if !path.exists() {
                    return Ok(format!("ENOENT: no such file or directory: '{}'", path_str));
                }
                // R_OK = 4, W_OK = 2 — check sandbox permissions
                if mode & 4 != 0 && !perms.check(&Capability::FileRead(path.clone())) {
                    return Ok(format!("EACCES: permission denied: '{}'", path_str));
                }
                if mode & 2 != 0 && !perms.check(&Capability::FileWrite(path.clone())) {
                    return Ok(format!("EACCES: permission denied: '{}'", path_str));
                }
                Ok("ok".to_string())
            },
        )?,
    )?;

    // ── __fsRealpathSync(path) -> String ──────────────────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsRealpathSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<String> {
                let path_str = args
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsRealpathSync() requires a path".into()))?;
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileRead(path.clone())) {
                    return Err(perm_err(&ctx, "read", &path));
                }
                std::fs::canonicalize(&path)
                    .map(|p| p.to_string_lossy().to_string())
                    .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))
            },
        )?,
    )?;

    // ── __fsReaddirSync(path) -> String (JSON array of filenames) ─────────────
    let perms = permissions.clone();
    globals.set(
        "__fsReaddirSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<String> {
                let path_str = args
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsReaddirSync() requires a path".into()))?;
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileRead(path.clone())) {
                    return Err(perm_err(&ctx, "read", &path));
                }
                let entries = std::fs::read_dir(&path)
                    .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))?;
                let names: Vec<String> = entries
                    .flatten()
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect();
                Ok(serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_string()))
            },
        )?,
    )?;

    // ── __fsMkdirSync(path) ───────────────────────────────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsMkdirSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<()> {
                let path_str = args
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsMkdirSync() requires a path".into()))?;
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileWrite(path.clone())) {
                    return Err(perm_err(&ctx, "write", &path));
                }
                std::fs::create_dir_all(&path)
                    .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))
            },
        )?,
    )?;

    // ── __fsRmSync(path) ──────────────────────────────────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsRmSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<()> {
                let path_str = args
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsRmSync() requires a path".into()))?;
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileWrite(path.clone())) {
                    return Err(perm_err(&ctx, "write", &path));
                }
                let result = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                result.map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))
            },
        )?,
    )?;

    // ── __fsUnlinkSync(path) ──────────────────────────────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsUnlinkSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<()> {
                let path_str = args
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsUnlinkSync() requires a path".into()))?;
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileWrite(path.clone())) {
                    return Err(perm_err(&ctx, "write", &path));
                }
                std::fs::remove_file(&path)
                    .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))
            },
        )?,
    )?;

    // ── __fsRenameSync(from, to) ──────────────────────────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsRenameSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<()> {
                let mut it = args.0.into_iter();
                let from_str = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsRenameSync() requires from, to".into()))?;
                let to_str = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsRenameSync() requires to path".into()))?;
                let from = PathBuf::from(&from_str);
                let to = PathBuf::from(&to_str);
                if !perms.check(&Capability::FileWrite(from.clone())) {
                    return Err(perm_err(&ctx, "write", &from));
                }
                if !perms.check(&Capability::FileWrite(to.clone())) {
                    return Err(perm_err(&ctx, "write", &to));
                }
                std::fs::rename(&from, &to).map_err(|e| js_err(&ctx, format!("ENOENT: {}", e)))
            },
        )?,
    )?;

    // ── __fsCopyFileSync(src, dest) ───────────────────────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsCopyFileSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<()> {
                let mut it = args.0.into_iter();
                let src_str = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsCopyFileSync() requires src, dest".into()))?;
                let dest_str = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsCopyFileSync() requires dest path".into()))?;
                let src = PathBuf::from(&src_str);
                let dest = PathBuf::from(&dest_str);
                if !perms.check(&Capability::FileRead(src.clone())) {
                    return Err(perm_err(&ctx, "read", &src));
                }
                if !perms.check(&Capability::FileWrite(dest.clone())) {
                    return Err(perm_err(&ctx, "write", &dest));
                }
                std::fs::copy(&src, &dest)
                    .map(|_| ())
                    .map_err(|e| js_err(&ctx, format!("ENOENT: {}", e)))
            },
        )?,
    )?;

    // ── __fsChmodSync(path, mode) ─────────────────────────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsChmodSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<()> {
                let mut it = args.0.into_iter();
                let path_str = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsChmodSync() requires a path".into()))?;
                let mode_str = it.next().unwrap_or_else(|| "0o644".to_string());
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileWrite(path.clone())) {
                    return Err(perm_err(&ctx, "write", &path));
                }
                let mode = u32::from_str_radix(mode_str.trim_start_matches("0o"), 8)
                    .or_else(|_| mode_str.parse::<u32>())
                    .unwrap_or(0o644);
                #[cfg(unix)]
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                    .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))?;
                #[cfg(not(unix))]
                {
                    let mut perms_obj = std::fs::metadata(&path)
                        .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))?
                        .permissions();
                    perms_obj.set_readonly(mode & 0o200 == 0);
                    std::fs::set_permissions(&path, perms_obj)
                        .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))?;
                }
                Ok(())
            },
        )?,
    )?;

    // ── __fsSymlinkSync(target, path) ─────────────────────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsSymlinkSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<()> {
                let mut it = args.0.into_iter();
                let target_str = it.next().ok_or_else(|| {
                    js_err(&ctx, "__fsSymlinkSync() requires target, path".into())
                })?;
                let path_str = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsSymlinkSync() requires path".into()))?;
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileWrite(path.clone())) {
                    return Err(perm_err(&ctx, "write", &path));
                }
                #[cfg(unix)]
                return std::os::unix::fs::symlink(&target_str, &path)
                    .map_err(|e| js_err(&ctx, format!("EEXIST: {}: '{}'", e, path_str)));
                #[cfg(windows)]
                return std::os::windows::fs::symlink_file(&target_str, &path)
                    .map_err(|e| js_err(&ctx, format!("EEXIST: {}: '{}'", e, path_str)));
                #[cfg(not(any(unix, windows)))]
                return Err(js_err(&ctx, "symlink not supported on this platform".into()));
            },
        )?,
    )?;

    // ── fs.watch backend (inotify / kqueue / FSEvents via notify crate) ──────────
    {
        use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
        use std::sync::atomic::{AtomicU32, Ordering};
        use tokio::sync::mpsc;

        /// Each live watcher: holds the notify handle (dropping it stops watching)
        /// and an async receiver for incoming events.
        struct WatcherEntry {
            _watcher: RecommendedWatcher,
            rx: tokio::sync::Mutex<mpsc::Receiver<notify::Result<notify::Event>>>,
        }

        type WatchTable = Arc<Mutex<HashMap<u32, Arc<WatcherEntry>>>>;
        static NEXT_WATCHER_ID: AtomicU32 = AtomicU32::new(1);

        let watch_table: WatchTable = Arc::new(Mutex::new(HashMap::new()));

        // __fsWatchCreate(path, recursive) → watcher_id  (synchronous)
        {
            let table = watch_table.clone();
            let perms = permissions.clone();
            globals.set(
                "__fsWatchCreate",
                Function::new(
                    ctx.clone(),
                    move |ctx: Ctx<'_>, path_str: String, recursive: bool| -> Result<u32> {
                        let path = PathBuf::from(&path_str);
                        if !perms.check(&Capability::FileRead(path.clone())) {
                            return Err(perm_err(&ctx, "watch", &path));
                        }

                        let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>(256);
                        let mut watcher = RecommendedWatcher::new(
                            move |res: notify::Result<notify::Event>| {
                                let _ = tx.blocking_send(res);
                            },
                            Config::default(),
                        )
                        .map_err(|e| js_err(&ctx, format!("fs.watch: {e}")))?;

                        let mode = if recursive {
                            RecursiveMode::Recursive
                        } else {
                            RecursiveMode::NonRecursive
                        };
                        watcher
                            .watch(&path, mode)
                            .map_err(|e| js_err(&ctx, format!("fs.watch '{}': {e}", path_str)))?;

                        let id = NEXT_WATCHER_ID.fetch_add(1, Ordering::Relaxed);
                        table.lock().unwrap().insert(
                            id,
                            Arc::new(WatcherEntry {
                                _watcher: watcher,
                                rx: tokio::sync::Mutex::new(rx),
                            }),
                        );
                        Ok(id)
                    },
                )?,
            )?;
        }

        // __fsWatchNext(id) → Promise<JSON>  awaits the next event
        //   JSON: {"eventType":"change"|"rename","filename":"foo.txt"}
        //   Rejects with ECLOSED when the watcher has been removed.
        {
            let table = watch_table.clone();
            globals.set(
                "__fsWatchNext",
                Function::new(
                    ctx.clone(),
                    rquickjs::function::Async(move |id: u32| {
                        let table = table.clone();
                        async move {
                            let entry = {
                                let guard = table.lock().unwrap();
                                guard.get(&id).cloned()
                            };
                            let entry = entry.ok_or_else(|| {
                                rquickjs::Error::new_from_js_message(
                                    "ECLOSED",
                                    "ECLOSED",
                                    "watcher closed".to_string(),
                                )
                            })?;

                            let event = {
                                let mut rx = entry.rx.lock().await;
                                rx.recv().await
                            };

                            match event {
                                None => Err(rquickjs::Error::new_from_js_message(
                                    "ECLOSED",
                                    "ECLOSED",
                                    "watcher channel closed".to_string(),
                                )),
                                Some(Err(e)) => Err(rquickjs::Error::new_from_js_message(
                                    "EIO",
                                    "EIO",
                                    e.to_string(),
                                )),
                                Some(Ok(ev)) => {
                                    use notify::event::{
                                        EventKind, ModifyKind, RenameMode,
                                    };
                                    let event_type = match ev.kind {
                                        EventKind::Create(_) => "rename",
                                        EventKind::Remove(_) => "rename",
                                        EventKind::Modify(ModifyKind::Name(
                                            RenameMode::From | RenameMode::To | RenameMode::Both,
                                        )) => "rename",
                                        _ => "change",
                                    };
                                    // Pick the first path; fall back to empty string.
                                    let filename = ev
                                        .paths
                                        .first()
                                        .and_then(|p| {
                                            p.file_name()
                                                .and_then(|n| n.to_str())
                                                .map(|s| s.to_string())
                                        })
                                        .unwrap_or_default();
                                    Ok(format!(
                                        "{{\"eventType\":\"{event_type}\",\"filename\":\"{filename}\"}}",
                                    ))
                                }
                            }
                        }
                    }),
                )?,
            )?;
        }

        // __fsWatchClose(id) → void  (synchronous; drops the watcher entry)
        {
            let table = watch_table.clone();
            globals.set(
                "__fsWatchClose",
                Function::new(ctx.clone(), move |id: u32| {
                    table.lock().unwrap().remove(&id);
                })?,
            )?;
        }
    }

    // ── __fsWatchPollStat(path, interval_ms) → Promise<stat JSON> ─────────────
    // Tokio-async stat poller for fs.watchFile.  Sleeps `interval_ms`, then
    // returns the current stat as JSON so the JS layer can detect changes.
    // Uses rquickjs::function::Async — no Ctx<'_> in the closure, matching
    // the pattern used by all other async bindings (e.g. __cryptoPbkdf2).
    {
        let perms = permissions.clone();
        globals.set(
            "__fsWatchPollStat",
            Function::new(
                ctx.clone(),
                rquickjs::function::Async(move |path_str: String, interval_ms: u64| {
                    let perms = perms.clone();
                    async move {
                        let path = PathBuf::from(&path_str);
                        if !perms.check(&Capability::FileRead(path.clone())) {
                            return Err(rquickjs::Error::new_from_js_message(
                                "EACCES",
                                "EACCES",
                                format!("permission denied: '{path_str}'"),
                            ));
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
                        let meta = std::fs::metadata(&path).map_err(|e| {
                            rquickjs::Error::new_from_js_message("ENOENT", "ENOENT", e.to_string())
                        })?;
                        Ok::<String, rquickjs::Error>(stat_meta_to_json(&meta))
                    }
                }),
            )?,
        )?;
    }

    // ── JS wrapper: globalThis.fs ─────────────────────────────────────────────
    ctx.eval::<(), _>(r#"
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

        function wrapAsync(syncFn) {
            return function() {
                var args = Array.prototype.slice.call(arguments);
                var cb = typeof args[args.length - 1] === 'function' ? args.pop() : null;
                var p = new Promise(function(resolve, reject) {
                    try { resolve(syncFn.apply(null, args)); }
                    catch(e) { reject(e); }
                });
                if (cb) {
                    p.then(function(v) { cb(null, v); }).catch(function(e) { cb(e); });
                    return;
                }
                return p;
            };
        }

        var fs = {
            constants: { F_OK: F_OK, R_OK: R_OK, W_OK: W_OK, X_OK: X_OK, COPYFILE_EXCL: 1 },

            // ── sync ────────────────────────────────────────────────────────────
            existsSync: function(p) { return __fsExistsSync(p); },
            readFileSync: function(p, opts) {
                var enc = opts && (typeof opts === 'string' ? opts : opts.encoding);
                if (enc && enc !== 'utf8' && enc !== 'utf-8') {
                    // Binary: return Buffer
                    var bytes = JSON.parse(__fsReadFileBytesSync(p));
                    return Buffer.from(bytes);
                }
                return __fsReadFileSync(p);
            },
            writeFileSync: function(p, data, opts) { return __fsWriteFileSync(p, typeof data === 'string' ? data : data.toString()); },
            appendFileSync: function(p, data) { return __fsAppendFileSync(p, typeof data === 'string' ? data : data.toString()); },
            readdirSync: function(p, opts) {
                var names = JSON.parse(__fsReaddirSync(p));
                if (opts && opts.withFileTypes) {
                    return names.map(function(n) {
                        return { name: n, isFile: function() { try { return JSON.parse(__fsStatSync(p + '/' + n, 'true')).isFile; } catch(e) { return false; } }, isDirectory: function() { try { return JSON.parse(__fsStatSync(p + '/' + n, 'true')).isDirectory; } catch(e) { return false; } }, isSymbolicLink: function() { return false; } };
                    });
                }
                return names;
            },
            mkdirSync: function(p, opts) { return __fsMkdirSync(p); },
            rmSync: function(p) { return __fsRmSync(p); },
            unlinkSync: function(p) { return __fsUnlinkSync(p); },
            renameSync: function(f, t) { return __fsRenameSync(f, t); },
            copyFileSync: function(s, d) { return __fsCopyFileSync(s, d); },
            chmodSync: function(p, m) { return __fsChmodSync(p, String(m)); },
            symlinkSync: function(target, p) { return __fsSymlinkSync(target, p); },
            statSync: function(p) { return parseStat(__fsStatSync(p, 'true')); },
            lstatSync: function(p) { return parseStat(__fsStatSync(p, 'false')); },
            realpathSync: function(p) { return __fsRealpathSync(p); },
            accessSync: function(p, mode) {
                var result = __fsAccessSync(p, String(mode === undefined ? 0 : mode));
                if (result !== 'ok') throw new Error(result);
            },

            // ── async (callback + promise) ──────────────────────────────────────
            readFile: function(p, opts, cb) {
                if (typeof opts === 'function') { cb = opts; opts = {}; }
                var self = this;
                var p2 = new Promise(function(resolve, reject) {
                    try { resolve(self.readFileSync(p, opts)); } catch(e) { reject(e); }
                });
                if (cb) { p2.then(function(v) { cb(null, v); }).catch(function(e) { cb(e); }); return; }
                return p2;
            },
            writeFile:   wrapAsync(function(p, data, opts) { return __fsWriteFileSync(p, typeof data === 'string' ? data : data.toString()); }),
            appendFile:  wrapAsync(function(p, data) { return __fsAppendFileSync(p, typeof data === 'string' ? data : data.toString()); }),
            readdir:     wrapAsync(function(p, opts) { return JSON.parse(__fsReaddirSync(p)); }),
            mkdir:       wrapAsync(function(p, opts) { return __fsMkdirSync(p); }),
            rm:          wrapAsync(function(p) { return __fsRmSync(p); }),
            unlink:      wrapAsync(function(p) { return __fsUnlinkSync(p); }),
            rename:      wrapAsync(function(f, t) { return __fsRenameSync(f, t); }),
            copyFile:    wrapAsync(function(s, d) { return __fsCopyFileSync(s, d); }),
            chmod:       wrapAsync(function(p, m) { return __fsChmodSync(p, String(m)); }),
            symlink:     wrapAsync(function(target, p) { return __fsSymlinkSync(target, p); }),
            stat:        wrapAsync(function(p) { return parseStat(__fsStatSync(p, 'true')); }),
            lstat:       wrapAsync(function(p) { return parseStat(__fsStatSync(p, 'false')); }),
            realpath:    wrapAsync(function(p) { return __fsRealpathSync(p); }),
            access: function(p, mode, cb) {
                if (typeof mode === 'function') { cb = mode; mode = 0; }
                var result = __fsAccessSync(p, String(mode === undefined ? 0 : mode));
                var err = result === 'ok' ? null : new Error(result);
                if (cb) { setTimeout(function() { cb(err); }, 0); return; }
                return err ? Promise.reject(err) : Promise.resolve();
            },

            // ── fd-based operations ─────────────────────────────────────────────
            open: function(path, flags, mode, cb) {
                if (typeof mode === 'function') { cb = mode; mode = 0o666; }
                if (typeof flags === 'number') flags = ['r','w','r+','w','w','a','a+'][flags] || 'r';
                var result;
                try { result = __fsFdOpen(path, flags, mode); } catch(e) { if (cb) cb(e); return; }
                if (cb) setTimeout(function() { cb(null, result); }, 0);
                return result;
            },
            openSync: function(path, flags, mode) {
                if (typeof flags === 'number') flags = ['r','w','r+','w','w','a','a+'][flags] || 'r';
                return __fsFdOpen(path, flags, mode || 0o666);
            },
            close: function(fd, cb) {
                try { __fsFdClose(fd); } catch(e) { if (cb) cb(e); return; }
                if (cb) setTimeout(function() { cb(null); }, 0);
            },
            closeSync: function(fd) { __fsFdClose(fd); },
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
                    data = Array.from(new TextEncoder().encode(buffer));
                } else {
                    var start = offset || 0;
                    var end   = length != null ? start + length : buffer.length;
                    data = Array.from(buffer.slice(start, end));
                }
                return __fsFdWrite(fd, data, position >= 0 ? position : null);
            },
            fstat: function(fd, cb) {
                try {
                    var s = parseStat(__fsFdStat(fd));
                    if (cb) setTimeout(function() { cb(null, s); }, 0);
                    else return s;
                } catch(e) { if (cb) cb(e); }
            },
            fstatSync: function(fd) { return parseStat(__fsFdStat(fd)); },
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

            // ── lutimes / lchown (stubs — rarely needed at JS level) ─────────────
            lutimes: function(p, at, mt, cb) { if (cb) setTimeout(function() { cb(null); }, 0); },
            lutimesSync: function() {},
            lchown: function(p, uid, gid, cb) { if (cb) setTimeout(function() { cb(null); }, 0); },
            lchownSync: function() {},
            chown: function(p, uid, gid, cb) { if (cb) setTimeout(function() { cb(null); }, 0); },
            chownSync: function() {},
            fchown: function(fd, uid, gid, cb) { if (cb) setTimeout(function() { cb(null); }, 0); },
            fchownSync: function() {},
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
                if (typeof opts === 'function') { cb = opts; }
                if (cb) setTimeout(function() { cb(new Error('EINVAL: readlink not fully supported')); }, 0);
                else return Promise.reject(new Error('EINVAL: readlink not fully supported'));
            },
            readlinkSync: function(p) { throw new Error('EINVAL: readlink not fully supported'); },

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
                var ropts = (typeof opts === 'object' && opts !== null) ? opts : {};
                var encoding = ropts.encoding;
                if (encoding === 'buffer' || encoding === null) encoding = undefined;
                var Readable = require('stream').Readable;
                var stream = new Readable({
                    highWaterMark: ropts.highWaterMark || 65536,
                    encoding: encoding,
                });
                stream.path = path;
                stream.bytesRead = 0;
                stream._fd = undefined;
                stream._done = false;
                stream._read = function(size) {
                    var self = this;
                    if (self._done) return;
                    if (self._fd === undefined) {
                        try {
                            self._fd = __fsFdOpen(path, 'r', null);
                            self.emit('open', self._fd);
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
                            if (self._fd !== undefined) { try { __fsFdClose(self._fd); } catch(_) {} }
                            self._done = true;
                            self.push(null);
                            return;
                        }
                        var maxRead = ropts.end !== undefined ? Math.min(chunkSize, (ropts.end - (ropts.start || 0) - self.bytesRead + 1)) : chunkSize;
                        if (maxRead <= 0) {
                            if (self._fd !== undefined) { try { __fsFdClose(self._fd); } catch(_) {} }
                            self._done = true;
                            self.push(null);
                            return;
                        }
                        var bytes = __fsFdRead(self._fd, maxRead, pos != null ? pos : null);
                        self.bytesRead += bytes.length;
                        if (bytes.length === 0) {
                            if (self._fd !== undefined) { try { __fsFdClose(self._fd); } catch(_) {} }
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
                        if (self._fd !== undefined) { try { __fsFdClose(self._fd); } catch(_) {} }
                        self.emit('error', e);
                    }
                };
                var _onOrig = stream.on.bind(stream);
                stream.on = function(event, fn) {
                    _onOrig(event, fn);
                    if (event === 'data' && !this._readableState.flowing) {
                        this._readableState.flowing = true;
                        this._read(65536);
                    }
                    return this;
                };
                stream.destroy = function() {
                    if (this._fd !== undefined) { try { __fsFdClose(this._fd); this._fd = undefined; } catch(_) {} }
                    this._done = true;
                    this.emit('close');
                    return this;
                };
                return stream;
            },

            // ── createWriteStream (proper Writable) ──────────────────────────────
            createWriteStream: function(path, opts) {
                var wopts = (typeof opts === 'object' && opts !== null) ? opts : {};
                var flags = wopts.flags || 'w';
                var Writable = require('stream').Writable;
                var stream = new Writable({
                    highWaterMark: wopts.highWaterMark || 16384,
                });
                stream.path = path;
                stream.bytesWritten = 0;
                stream._fd = undefined;
                stream._write = function(chunk, encoding, callback) {
                    var self = this;
                    if (self._fd === undefined) {
                        try {
                            self._fd = __fsFdOpen(path, flags, null);
                            self.emit('open', self._fd);
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
                        __fsFdWrite(self._fd, data, null);
                        self.bytesWritten += data.length;
                        callback(null);
                    } catch(e) {
                        callback(e);
                    }
                };
                stream._final = function(callback) {
                    if (this._fd !== undefined) {
                        try { __fsFdClose(this._fd); this._fd = undefined; } catch(_) {}
                    }
                    callback();
                };
                stream.destroy = function(err) {
                    if (this._fd !== undefined) { try { __fsFdClose(this._fd); this._fd = undefined; } catch(_) {} }
                    if (err) this.emit('error', err);
                    this.emit('close');
                    return this;
                };
                return stream;
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

                // Poll loop: each __fsWatchNext call awaits ONE event then recurses.
                function poll() {
                    if (closed) return;
                    __fsWatchNext(watcherId).then(function(json) {
                        if (closed) return;
                        var ev = JSON.parse(json);
                        if (typeof listener === 'function') {
                            listener(ev.eventType, ev.filename);
                        }
                        watcher.emit('change', ev.eventType, ev.filename);
                        poll();
                    }).catch(function(err) {
                        if (!closed) {
                            // ECLOSED = we called close() — don't surface as an error.
                            if (!err || (err.message !== 'watcher closed' && err.message !== 'watcher channel closed')) {
                                watcher.emit('error', err);
                            }
                        }
                    });
                }

                poll();

                watcher.close = function() {
                    if (closed) return;
                    closed = true;
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

                function poll() {
                    if (closed) return;
                    __fsWatchNext(watcherId).then(function() {
                        if (closed) return;
                        var currStat;
                        try { currStat = parseStat(__fsStatSync(filename, false)); }
                        catch(e) { currStat = { mtimeMs: 0, size: 0, isFile: function(){return false;}, isDirectory: function(){return false;} }; }
                        if (typeof listener === 'function') listener(currStat, prevStat);
                        prevStat = currStat;
                        poll();
                    }).catch(function() { /* watcher closed or error — stop silently */ });
                }

                poll();

                return {
                    stop: function() {
                        if (closed) return;
                        closed = true;
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

        // Build fs.promises from fs async methods
        ['readFile','writeFile','appendFile','readdir','mkdir','rm','unlink','rename',
         'copyFile','chmod','symlink','stat','lstat','realpath','access'].forEach(function(fn) {
            fs.promises[fn] = function() {
                var args = Array.prototype.slice.call(arguments);
                return new Promise(function(resolve, reject) {
                    args.push(function(err, val) { if (err) reject(err); else resolve(val); });
                    fs[fn].apply(fs, args);
                });
            };
        });

        globalThis.fs = fs;
        // Re-register in require cache so require('fs') reflects the full object
        if (globalThis.__requireCache) {
            globalThis.__requireCache['fs'] = fs;
            globalThis.__requireCache['node:fs'] = fs;
            globalThis.__requireCache['fs/promises'] = fs.promises;
            globalThis.__requireCache['node:fs/promises'] = fs.promises;
        }
    })();
    "#)?;

    Ok(())
}
