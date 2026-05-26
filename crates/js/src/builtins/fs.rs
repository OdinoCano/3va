use rquickjs::{Ctx, Function, Result, function::Rest};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use vvva_permissions::{Capability, PermissionState};

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

pub fn inject_fs(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();

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
                let path_str = it.next()
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
                }.map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))?;

                let mtime_ms = meta.modified().ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as f64)
                    .unwrap_or(0.0);
                let atime_ms = meta.accessed().ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as f64)
                    .unwrap_or(0.0);
                let ctime_ms = meta.created().ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as f64)
                    .unwrap_or(mtime_ms);

                let mode = meta.permissions().mode();
                let is_dir = meta.is_dir();
                let is_file = meta.is_file();
                let is_symlink = meta.file_type().is_symlink();

                Ok(format!(
                    r#"{{"size":{},"mode":{},"isFile":{},"isDirectory":{},"isSymbolicLink":{},"mtimeMs":{},"atimeMs":{},"ctimeMs":{},"birthtimeMs":{},"nlink":1,"uid":0,"gid":0,"ino":0,"dev":0,"rdev":0}}"#,
                    meta.len(), mode, is_file, is_dir, is_symlink,
                    mtime_ms, atime_ms, ctime_ms, ctime_ms
                ))
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
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                    .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))
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
                std::os::unix::fs::symlink(&target_str, &path)
                    .map_err(|e| js_err(&ctx, format!("EEXIST: {}: '{}'", e, path_str)))
            },
        )?,
    )?;

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

            // ── createReadStream ────────────────────────────────────────────────
            createReadStream: function(path, opts) {
                var EventEmitter = require('events');
                var stream = new EventEmitter();
                stream.readable = true;
                stream.path = path;
                stream.bytesRead = 0;
                stream._flowing = false;
                stream._doRead = function() {
                    var self = this;
                    setTimeout(function() {
                        try {
                            var data = __fsReadFileSync(path);
                            self.bytesRead = data.length;
                            self.emit('data', data);
                            self.emit('end');
                        } catch(e) {
                            self.emit('error', e);
                        }
                    }, 0);
                };
                // Override on() to auto-start when a 'data' listener is added (Node.js flowing mode)
                var _origOn = stream.on.bind(stream);
                stream.on = function(event, fn) {
                    _origOn(event, fn);
                    if (event === 'data' && !this._flowing) {
                        this._flowing = true;
                        this._doRead();
                    }
                    return this;
                };
                stream.pipe = function(dest) {
                    var self = this;
                    setTimeout(function() {
                        try {
                            var data = __fsReadFileSync(path);
                            self.bytesRead = data.length;
                            if (dest && dest.write) dest.write(data);
                            self.emit('data', data);
                            self.emit('end');
                            if (dest && dest.end) dest.end();
                        } catch(e) {
                            self.emit('error', e);
                        }
                    }, 0);
                    return dest;
                };
                stream.resume = function() {
                    if (!this._flowing) {
                        this._flowing = true;
                        this._doRead();
                    }
                    return this;
                };
                stream.destroy = function() { this.emit('close'); };
                return stream;
            },

            // ── createWriteStream ───────────────────────────────────────────────
            createWriteStream: function(path, opts) {
                var EventEmitter = require('events');
                var stream = new EventEmitter();
                stream.writable = true;
                stream.path = path;
                stream.bytesWritten = 0;
                stream._buf = '';
                stream.write = function(chunk) {
                    var s = typeof chunk === 'string' ? chunk : chunk.toString();
                    this._buf += s;
                    this.bytesWritten += s.length;
                    return true;
                };
                stream.end = function(chunk) {
                    if (chunk) this.write(chunk);
                    try {
                        __fsWriteFileSync(path, this._buf);
                        this.emit('finish');
                        this.emit('close');
                    } catch(e) {
                        this.emit('error', e);
                    }
                };
                stream.destroy = function() { this.emit('close'); };
                // Auto-flush flag for append mode
                if (opts && opts.flags === 'a') {
                    stream._append = true;
                }
                return stream;
            },

            // ── watch (stub — no inotify in sandbox) ────────────────────────────
            watch: function(path, opts, cb) {
                if (typeof opts === 'function') { cb = opts; }
                var EventEmitter = require('events');
                var watcher = new EventEmitter();
                watcher.close = function() {};
                return watcher;
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
