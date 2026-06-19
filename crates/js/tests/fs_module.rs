// Tests for the fs builtin: stat, symlink, rename, copyFile, appendFile,
// realpath, access, createReadStream, readdirSync with withFileTypes, fs.promises.
// Run: cargo test -p vvva_js --test fs_module

use std::sync::Arc;
use tempfile::TempDir;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_rw(dir: &TempDir) -> JsEngine {
    let p = dir.path().to_path_buf();
    let state = PermissionState::new();
    state.grant(Capability::FileRead(p.clone()));
    state.grant(Capability::FileWrite(p));
    JsEngine::new(Arc::new(state)).await.unwrap()
}

fn path_str(dir: &TempDir, name: &str) -> String {
    // Escape backslashes so Windows paths survive JS string interpolation
    // (e.g. \t in C:\Users\telma would be interpreted as a tab otherwise).
    dir.path()
        .join(name)
        .to_string_lossy()
        .replace('\\', "\\\\")
}

/// Drive async Promises to completion.
async fn eval_async(e: &JsEngine, setup: &str, result_global: &str) -> String {
    e.eval(setup).await.unwrap();
    for _ in 0..50 {
        e.idle().await;
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        let r = e
            .eval_to_string(&format!(
                "typeof {result_global} !== 'undefined' ? String({result_global}) : ''"
            ))
            .await
            .unwrap();
        if !r.is_empty() {
            return r;
        }
    }
    String::new()
}

// ── readFileSync / writeFileSync ──────────────────────────────────────────────

#[tokio::test]
async fn fs_write_and_read_sync() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let p = path_str(&dir, "hello.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            fs.writeFileSync('{p}', 'hello world');
            fs.readFileSync('{p}', 'utf8')
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "hello world");
}

// ── appendFileSync ────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_append_file_sync() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let p = path_str(&dir, "append.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            fs.writeFileSync('{p}', 'foo');
            fs.appendFileSync('{p}', 'bar');
            fs.readFileSync('{p}', 'utf8')
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "foobar");
}

// ── statSync ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_stat_sync_file() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let p = path_str(&dir, "stat.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            fs.writeFileSync('{p}', 'content');
            var s = fs.statSync('{p}');
            [s.isFile(), s.isDirectory(), s.size > 0].join(',')
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "true,false,true");
}

#[tokio::test]
async fn fs_stat_sync_directory() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let p = dir.path().to_string_lossy().replace('\\', "\\\\");

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            var s = fs.statSync('{p}');
            [s.isFile(), s.isDirectory()].join(',')
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "false,true");
}

// ── existsSync ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_exists_sync() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let exists = path_str(&dir, "exists.txt");
    let missing = path_str(&dir, "missing.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            fs.writeFileSync('{exists}', 'x');
            [fs.existsSync('{exists}'), fs.existsSync('{missing}')].join(',')
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "true,false");
}

// ── renameSync ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_rename_sync() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let src = path_str(&dir, "before.txt");
    let dst = path_str(&dir, "after.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            fs.writeFileSync('{src}', 'data');
            fs.renameSync('{src}', '{dst}');
            [fs.existsSync('{src}'), fs.existsSync('{dst}')].join(',')
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "false,true");
}

// ── copyFileSync ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_copy_file_sync() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let src = path_str(&dir, "orig.txt");
    let dst = path_str(&dir, "copy.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            fs.writeFileSync('{src}', 'original');
            fs.copyFileSync('{src}', '{dst}');
            fs.readFileSync('{dst}', 'utf8')
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "original");
}

// ── unlinkSync ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_unlink_sync() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let p = path_str(&dir, "todelete.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            fs.writeFileSync('{p}', 'bye');
            fs.unlinkSync('{p}');
            String(fs.existsSync('{p}'))
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "false");
}

// ── mkdirSync / readdirSync ───────────────────────────────────────────────────

#[tokio::test]
async fn fs_mkdir_and_readdir_sync() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let sub = path_str(&dir, "mydir");
    let f = path_str(&dir, "mydir/file.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            fs.mkdirSync('{sub}');
            fs.writeFileSync('{f}', 'x');
            var entries = fs.readdirSync('{sub}');
            entries.join(',')
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "file.txt");
}

#[tokio::test]
async fn fs_readdir_with_file_types() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let sub = path_str(&dir, "typed");
    let f = path_str(&dir, "typed/a.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            fs.mkdirSync('{sub}');
            fs.writeFileSync('{f}', 'y');
            var entries = fs.readdirSync('{sub}', {{ withFileTypes: true }});
            var d = entries[0];
            [d.name, d.isFile(), d.isDirectory()].join(',')
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "a.txt,true,false");
}

// ── realpathSync ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_realpath_sync() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let p = path_str(&dir, "real.txt");
    let canonical = std::fs::canonicalize(dir.path()).unwrap();
    let expected = canonical.join("real.txt").to_string_lossy().into_owned();

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            fs.writeFileSync('{p}', '');
            fs.realpathSync('{p}')
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, expected);
}

// ── accessSync ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_access_sync_existing_file() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let p = path_str(&dir, "access.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            fs.writeFileSync('{p}', '');
            var ok = false;
            try {{ fs.accessSync('{p}'); ok = true; }} catch(e) {{}}
            String(ok)
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn fs_access_sync_missing_throws() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let p = path_str(&dir, "nope.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            var threw = false;
            try {{ fs.accessSync('{p}'); }} catch(e) {{ threw = true; }}
            String(threw)
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── symlinkSync / lstatSync ───────────────────────────────────────────────────
// Creating symlinks on Windows requires elevated privileges (or Developer Mode).

#[cfg(not(windows))]
#[tokio::test]
async fn fs_symlink_and_lstat() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let target = path_str(&dir, "target.txt");
    let link = path_str(&dir, "link.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            fs.writeFileSync('{target}', 'data');
            fs.symlinkSync('{target}', '{link}');
            var ls = fs.lstatSync('{link}');
            var ss = fs.statSync('{link}');
            [ls.isSymbolicLink(), ss.isFile()].join(',')
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "true,true");
}

// ── fs.promises ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_promises_readfile_writefile() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let p = path_str(&dir, "promise.txt");

    let r = eval_async(
        &e,
        &format!(
            r#"
            var fs = require('fs');
            globalThis.__result = undefined;
            fs.promises.writeFile('{p}', 'async content')
                .then(function() {{ return fs.promises.readFile('{p}', 'utf8'); }})
                .then(function(data) {{ globalThis.__result = data; }})
                .catch(function(e) {{ globalThis.__result = 'err:' + e; }});
            "#
        ),
        "__result",
    )
    .await;
    assert_eq!(r, "async content");
}

#[tokio::test]
async fn fs_promises_stat() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let p = path_str(&dir, "stat2.txt");

    let r = eval_async(
        &e,
        &format!(
            r#"
            var fs = require('fs');
            globalThis.__result = undefined;
            fs.writeFileSync('{p}', 'hello');
            fs.promises.stat('{p}')
                .then(function(s) {{ globalThis.__result = String(s.isFile()); }})
                .catch(function(e) {{ globalThis.__result = 'err'; }});
            "#
        ),
        "__result",
    )
    .await;
    assert_eq!(r, "true");
}

// ── createReadStream ─────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_create_read_stream() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let p = path_str(&dir, "stream.txt");

    let r = eval_async(
        &e,
        &format!(
            r#"
            var fs = require('fs');
            globalThis.__result = undefined;
            fs.writeFileSync('{p}', 'streamed');
            var chunks = [];
            var s = fs.createReadStream('{p}');
            s.on('data', function(chunk) {{
                chunks.push(typeof chunk === 'string' ? chunk : new TextDecoder().decode(chunk));
            }});
            s.on('end', function() {{
                globalThis.__result = chunks.join('');
            }});
            s.on('error', function(e) {{ globalThis.__result = 'err'; }});
            "#
        ),
        "__result",
    )
    .await;
    assert_eq!(r, "streamed");
}

#[tokio::test]
async fn fs_create_write_stream() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let p = path_str(&dir, "writestream.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
        var Writable = require('stream').Writable;
        var s = require('fs').createWriteStream('{p}');
        var ok = s instanceof Writable;
        s.write('hello ');
        s.write('world');
        s.end();
        var content = require('fs').readFileSync('{p}', 'utf8');
        String(s.bytesWritten) + '|' + content
        "#
        ))
        .await
        .unwrap();
    assert_eq!(r, "11|hello world");
}

#[tokio::test]
async fn fs_create_write_stream_sync() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let p = path_str(&dir, "writestream2.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
        var Writable = require('stream').Writable;
        var s = require('fs').createWriteStream('{p}');
        var ok = s instanceof Writable;
        s.write('hello ');
        s.write('world');
        s.end();
        var content = require('fs').readFileSync('{p}', 'utf8');
        String(s.bytesWritten) + '|' + content
        "#
        ))
        .await
        .unwrap();
    assert_eq!(r, "11|hello world");
}

#[tokio::test]
async fn fs_create_read_stream_instanceof() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let p = path_str(&dir, "istest.txt");
    std::fs::write(&p, "data").unwrap();

    let r = e
        .eval_to_string(
            r#"
            var fs = require('fs');
            var stream = require('stream');
            var s = fs.createReadStream('/nonexistent');
            var w = fs.createWriteStream('/nonexistent');
            String(s instanceof stream.Readable) + '|' + String(w instanceof stream.Writable)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true|true");
}

// ── fs.constants ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_constants() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;

    let r = e
        .eval_to_string(
            r#"
            var c = require('fs').constants;
            [c.F_OK, c.R_OK, c.W_OK, c.X_OK].join(',')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "0,4,2,1");
}

// ── node:fs alias ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_node_prefix_alias() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;

    let r = e
        .eval_to_string(
            r#"
            var fs1 = require('fs');
            var fs2 = require('node:fs');
            String(typeof fs1.readFileSync === 'function' && typeof fs2.readFileSync === 'function')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── fd-based operations ───────────────────────────────────────────────────────

#[tokio::test]
async fn fs_fd_open_read_close() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let file_path = path_str(&dir, "test_fd.txt");
    std::fs::write(&file_path, "hello world").unwrap();

    let r = e
        .eval_to_string(&format!(
            r#"
        var fs = require('fs');
        var fd = fs.openSync({:?}, 'r');
        var buf = new Uint8Array(5);
        var n = fs.readSync(fd, buf, 0, 5, 0);
        fs.closeSync(fd);
        new TextDecoder().decode(buf.slice(0, n))
    "#,
            file_path
        ))
        .await
        .unwrap();
    assert_eq!(r, "hello");
}

#[tokio::test]
async fn fs_fd_write_sync() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let file_path = path_str(&dir, "test_fd_write.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
        var fs = require('fs');
        var fd = fs.openSync({:?}, 'w');
        var data = new TextEncoder().encode('written');
        var n = fs.writeSync(fd, data, 0, data.length, null);
        fs.closeSync(fd);
        String(n)
    "#,
            file_path
        ))
        .await
        .unwrap();
    assert_eq!(r, "7");
    assert_eq!(std::fs::read_to_string(&file_path).unwrap(), "written");
}

#[tokio::test]
async fn fs_mkdtemp_creates_directory() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let prefix = path_str(&dir, "tmp-");

    let r = e
        .eval_to_string(&format!(
            r#"
        var fs = require('fs');
        var tmpDir = fs.mkdtempSync({:?});
        fs.existsSync(tmpDir) ? 'ok' : 'fail'
    "#,
            prefix
        ))
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn fs_opendir_iterates_entries() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    std::fs::write(dir.path().join("a.txt"), "").unwrap();
    std::fs::write(dir.path().join("b.txt"), "").unwrap();
    let dir_path = dir.path().to_string_lossy().into_owned();

    let r = e
        .eval_to_string(&format!(
            r#"
        var fs = require('fs');
        var d = fs.opendirSync({:?});
        var names = [];
        var entry;
        while ((entry = d.readSync()) !== null) names.push(entry.name);
        d.closeSync();
        names.sort().join(',')
    "#,
            dir_path
        ))
        .await
        .unwrap();
    assert_eq!(r, "a.txt,b.txt");
}

// ── cpSync ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_cp_sync_file() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let src = path_str(&dir, "src.txt");
    let dst = path_str(&dir, "dst.txt");

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            fs.writeFileSync('{src}', 'copied');
            fs.cpSync('{src}', '{dst}');
            fs.readFileSync('{dst}', 'utf8')
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "copied");
}

#[tokio::test]
async fn fs_cp_sync_directory_recursive() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let src_dir = path_str(&dir, "mydir");
    let dst_dir = path_str(&dir, "copydir");

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            fs.mkdirSync('{src_dir}');
            fs.writeFileSync('{src_dir}/a.txt', 'file_a');
            fs.writeFileSync('{src_dir}/b.txt', 'file_b');
            fs.cpSync('{src_dir}', '{dst_dir}');
            var files = fs.readdirSync('{dst_dir}').sort().join(',');
            var contentA = fs.readFileSync('{dst_dir}/a.txt', 'utf8');
            var contentB = fs.readFileSync('{dst_dir}/b.txt', 'utf8');
            files + '|' + contentA + '|' + contentB
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "a.txt,b.txt|file_a|file_b");
}

#[tokio::test]
async fn fs_cp_async_file() {
    let dir = TempDir::new().unwrap();
    let e = engine_rw(&dir).await;
    let src = path_str(&dir, "async_src.txt");
    let dst = path_str(&dir, "async_dst.txt");
    std::fs::write(&src, "async_copy").unwrap();

    let r = eval_async(
        &e,
        &format!(
            r#"
            var fs = require('fs');
            globalThis.__result = undefined;
            fs.cp('{src}', '{dst}', function(err) {{
                if (err) {{ globalThis.__result = 'err:' + err; return; }}
                globalThis.__result = fs.readFileSync('{dst}', 'utf8');
            }});
            "#,
        ),
        "__result",
    )
    .await;
    assert_eq!(r, "async_copy");
}
