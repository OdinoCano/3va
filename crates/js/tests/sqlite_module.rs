// Tests for the node:sqlite builtin: DatabaseSync, StatementSync.
// Run: cargo test -p vvva_js --test sqlite_module

use std::sync::Arc;
use tempfile::TempDir;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_rw(dir: &TempDir) -> JsEngine {
    let p = dir.path().to_path_buf();
    let state = PermissionState::new();
    state.grant(Capability::FileWrite(p.clone()));
    state.grant(Capability::FileRead(p));
    JsEngine::new(Arc::new(state)).await.unwrap()
}

fn path_str(dir: &TempDir, name: &str) -> String {
    dir.path()
        .join(name)
        .to_string_lossy()
        .replace('\\', "\\\\")
}

// ── DatabaseSync: open / exec / close ─────────────────────────────────────────

#[tokio::test]
async fn sqlite_open_and_close() {
    let dir = TempDir::new().unwrap();
    let mut e = engine_rw(&dir).await;
    let db = path_str(&dir, "test.db");

    let r = e
        .eval_to_string(&format!(
            r#"
            var sqlite = require('node:sqlite');
            var db = new sqlite.DatabaseSync('{db}');
            db.exec('CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)');
            db.close();
            'ok'
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn sqlite_insert_and_get() {
    let dir = TempDir::new().unwrap();
    let mut e = engine_rw(&dir).await;
    let db = path_str(&dir, "test2.db");

    let r = e
        .eval_to_string(&format!(
            r#"
            var sqlite = require('node:sqlite');
            var db = new sqlite.DatabaseSync('{db}');
            db.exec('CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)');
            var stmt = db.prepare('INSERT INTO t (name) VALUES (?)');
            stmt.run('Alice');
            var getStmt = db.prepare('SELECT id, name FROM t WHERE name = ?');
            var row = getStmt.get('Alice');
            db.close();
            row.name
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "Alice");
}

#[tokio::test]
async fn sqlite_statement_all() {
    let dir = TempDir::new().unwrap();
    let mut e = engine_rw(&dir).await;
    let db = path_str(&dir, "test3.db");

    let r = e
        .eval_to_string(&format!(
            r#"
            var sqlite = require('node:sqlite');
            var db = new sqlite.DatabaseSync('{db}');
            db.exec('CREATE TABLE t (x INTEGER)');
            var ins = db.prepare('INSERT INTO t (x) VALUES (?)');
            ins.run(10);
            ins.run(20);
            ins.run(30);
            var sel = db.prepare('SELECT x FROM t ORDER BY x');
            var rows = sel.all();
            db.close();
            rows.map(function(r) {{ return r.x; }}).join(',')
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "10,20,30");
}

#[tokio::test]
async fn sqlite_statement_run_returns_changes() {
    let dir = TempDir::new().unwrap();
    let mut e = engine_rw(&dir).await;
    let db = path_str(&dir, "test4.db");

    let r = e
        .eval_to_string(&format!(
            r#"
            var sqlite = require('node:sqlite');
            var db = new sqlite.DatabaseSync('{db}');
            db.exec('CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT)');
            var ins = db.prepare('INSERT INTO t (v) VALUES (?)');
            var result = ins.run('hello');
            db.close();
            JSON.stringify(result)
            "#,
        ))
        .await
        .unwrap();
    assert!(
        r.contains("changes") || r.contains("lastInsertRowid"),
        "expected changes/lastInsertRowid in result, got: {r}"
    );
}

#[tokio::test]
async fn sqlite_get_returns_undefined_for_missing() {
    let dir = TempDir::new().unwrap();
    let mut e = engine_rw(&dir).await;
    let db = path_str(&dir, "test5.db");

    let r = e
        .eval_to_string(&format!(
            r#"
            var sqlite = require('node:sqlite');
            var db = new sqlite.DatabaseSync('{db}');
            db.exec('CREATE TABLE t (id INTEGER PRIMARY KEY)');
            var stmt = db.prepare('SELECT * FROM t WHERE id = ?');
            var result = stmt.get(999);
            db.close();
            String(result === undefined)
            "#,
        ))
        .await
        .unwrap();
    assert_eq!(r, "true");
}
