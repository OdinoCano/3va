// Tests for high-priority Node.js compatibility items.
// Run: cargo test -p vvva_js --test compat_priority

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

async fn engine_with_read(path: &str) -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::FileRead(std::path::PathBuf::from(path)));
    perms.grant(Capability::FileWrite(std::path::PathBuf::from(path)));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}

async fn eval_async(e: &JsEngine, setup: &str, result_global: &str) -> String {
    e.eval(setup).await.unwrap();
    for _ in 0..100 {
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

// ── fs/promises ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn fs_promises_readfile() {
    // Write a temp file first using std::fs, then read with fs/promises
    let tmp = std::env::temp_dir().join("3va_test_fsp.txt");
    std::fs::write(&tmp, "hello promises").unwrap();
    let path = tmp.to_str().unwrap().to_string();
    let e = engine_with_read(&path).await;
    let r = eval_async(
        &e,
        &format!(
            r#"
        globalThis.__result = undefined;
        var fs = require('fs/promises');
        fs.readFile({path:?}, 'utf8').then(function(data) {{
            globalThis.__result = data === 'hello promises' ? 'ok' : 'bad:' + data;
        }}).catch(function(e) {{ globalThis.__result = 'err:' + e.message; }});
        "#,
            path = path
        ),
        "__result",
    )
    .await;
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(r, "ok", "fs/promises.readFile failed: {r}");
}

#[tokio::test]
async fn fs_promises_module_has_all_methods() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var fs = require('fs/promises');
            var methods = ['readFile','writeFile','mkdir','readdir','stat','unlink','rm','rename','copyFile','access','realpath'];
            methods.filter(function(m) { return typeof fs[m] !== 'function'; }).join(',') || 'ok'
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok", "missing fs/promises methods: {r}");
}

#[tokio::test]
async fn node_fs_promises_prefix() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var a = require('fs/promises');
            var b = require('node:fs/promises');
            String(a === b)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── url.fileURLToPath / pathToFileURL ─────────────────────────────────────────

#[tokio::test]
async fn url_file_url_to_path() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var { fileURLToPath } = require('url');
            fileURLToPath('file:///var/www/html/index.js')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "/var/www/html/index.js");
}

#[tokio::test]
async fn url_path_to_file_url() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var { pathToFileURL } = require('url');
            var u = pathToFileURL('/var/www/html/index.js');
            u.href
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "file:///var/www/html/index.js");
}

#[tokio::test]
async fn url_file_url_roundtrip() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var url = require('url');
            var path = '/tmp/test file.js';
            var fileUrl = url.pathToFileURL(path);
            var back = url.fileURLToPath(fileUrl.href);
            back === path ? 'ok' : 'got:' + back
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

// ── stream with _write/_transform/_flush hooks ────────────────────────────────

#[tokio::test]
async fn stream_writable_write_hook() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var { Writable } = require('stream');
            var util = require('util');
            var collected = [];
            function MyWritable(opts) { Writable.call(this, opts); }
            util.inherits(MyWritable, Writable);
            MyWritable.prototype._write = function(chunk, enc, cb) {
                collected.push(chunk.toString());
                cb();
            };
            var w = new MyWritable();
            w.write('hello');
            w.write(' world');
            collected.join('')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "hello world");
}

#[tokio::test]
async fn stream_transform_hook() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var { Transform } = require('stream');
            var util = require('util');
            function Upper(opts) { Transform.call(this, opts); }
            util.inherits(Upper, Transform);
            Upper.prototype._transform = function(chunk, enc, cb) {
                cb(null, chunk.toString().toUpperCase());
            };
            var t = new Upper();
            var out = [];
            t.on('data', function(c) { out.push(c.toString()); });
            t.write('hello');
            t.write(' world');
            out.join('')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "HELLO WORLD");
}

#[tokio::test]
async fn stream_transform_flush_hook() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var { Transform } = require('stream');
            var util = require('util');
            function Suffix(opts) { Transform.call(this, opts); }
            util.inherits(Suffix, Transform);
            Suffix.prototype._transform = function(chunk, enc, cb) { cb(null, chunk); };
            Suffix.prototype._flush = function(cb) { cb(null, '!'); };
            var t = new Suffix();
            var out = [];
            t.on('data', function(c) { out.push(c.toString()); });
            t.write('hello');
            t.end();
            out.join('')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "hello!");
}

#[tokio::test]
async fn stream_passthrough_works() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var { PassThrough } = require('stream');
            var pt = new PassThrough();
            var out = [];
            pt.on('data', function(c) { out.push(c.toString()); });
            pt.write('a');
            pt.write('b');
            out.join('')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ab");
}

#[tokio::test]
async fn stream_options_constructor() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var stream = require('stream');
            var out = [];
            var w = new stream.Writable({
                write: function(chunk, enc, cb) { out.push(chunk.toString()); cb(); }
            });
            w.write('x');
            w.write('y');
            out.join('')
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "xy");
}

// ── crypto pbkdf2Sync / createCipheriv ────────────────────────────────────────

#[tokio::test]
async fn crypto_pbkdf2_sync() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var crypto = require('crypto');
            var key = crypto.pbkdf2Sync('password', 'salt', 1000, 32, 'sha256');
            key instanceof Uint8Array && key.length === 32 ? 'ok' : 'bad'
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn crypto_create_cipheriv_aes_gcm() {
    let e = engine().await;
    let r = e
        .eval_to_string(
            r#"
            var crypto = require('crypto');
            var key = crypto.randomBytes(32);
            var iv  = crypto.randomBytes(12);
            var cipher  = crypto.createCipheriv('aes-256-gcm', key, iv);
            cipher.update('hello world');
            var ct = cipher.final();
            var tag = cipher.getAuthTag();
            var decipher = crypto.createDecipheriv('aes-256-gcm', key, iv);
            decipher.setAuthTag(tag);
            decipher.update(ct);
            var pt = decipher.final();
            new TextDecoder().decode(pt)
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "hello world");
}
