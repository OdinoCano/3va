// Tests for fs.watch (inotify-backed real file watching).
// Run: cargo test -p vvva_js --test fs_watch

use std::sync::Arc;
use std::time::Duration;
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
    dir.path().join(name).to_string_lossy().into_owned()
}

/// Drive the engine until `result_global` is set or we time out (5 s).
/// A trigger closure runs in a blocking thread after 120 ms so the watcher fires.
async fn drive_until_watch_event(
    e: &mut JsEngine,
    result_global: &str,
    trigger: impl FnOnce() + Send + 'static,
) -> String {
    // Spawn the trigger; ignore errors in the trigger thread (TempDir might
    // already be cleaned up if the drive loop timed out earlier).
    tokio::task::spawn_blocking(move || {
        std::thread::sleep(Duration::from_millis(120));
        trigger();
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        e.idle().await;
        // Timers (setInterval, used by fs.watch's __fsWatchNext poll loop)
        // only fire inside run_event_loop(), not idle() — see the identical
        // pattern in compat_priority.rs's eval_async().
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;

        let r = e
            .eval_to_string(&format!(
                "typeof {result_global} !== 'undefined' ? String({result_global}) : ''"
            ))
            .await
            .unwrap_or_default();
        if !r.is_empty() {
            return r;
        }
        if tokio::time::Instant::now() > deadline {
            return String::new();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

// ── fs.watch detects file modification ───────────────────────────────────────

#[tokio::test]
async fn watch_detects_file_change() {
    let dir = TempDir::new().unwrap();
    let file = path_str(&dir, "watched.txt");
    std::fs::write(&file, "initial").unwrap();

    let mut e = engine_rw(&dir).await;

    e.eval(&format!(
        r#"
        var fs = require('fs');
        globalThis.__watchEvent = null;
        var w = fs.watch({file:?}, function(eventType, filename) {{
            globalThis.__watchEvent = eventType + ':' + filename;
            w.close();
        }});
        "#,
        file = file,
    ))
    .await
    .unwrap();

    let file_clone = file.clone();
    let result = drive_until_watch_event(&mut e, "__watchEvent", move || {
        std::fs::write(&file_clone, "updated").unwrap();
    })
    .await;

    assert!(!result.is_empty(), "expected a watch event but got nothing");
    // eventType is either 'change' or 'rename' depending on the OS
    assert!(
        result.starts_with("change:") || result.starts_with("rename:"),
        "unexpected event: {result}"
    );
}

// ── fs.watch detects file creation ───────────────────────────────────────────

#[tokio::test]
async fn watch_detects_file_creation() {
    let dir = TempDir::new().unwrap();
    let dir_path = path_str(&dir, "");
    // Remove trailing slash
    let dir_path = dir_path.trim_end_matches('/').to_string();

    let mut e = engine_rw(&dir).await;

    e.eval(&format!(
        r#"
        var fs = require('fs');
        globalThis.__watchEvent = null;
        var w = fs.watch({dir:?}, function(eventType, filename) {{
            if (filename === 'newfile.txt') {{
                globalThis.__watchEvent = eventType + ':' + filename;
                w.close();
            }}
        }});
        "#,
        dir = dir_path,
    ))
    .await
    .unwrap();

    let new_file = dir.path().join("newfile.txt");
    let result = drive_until_watch_event(&mut e, "__watchEvent", move || {
        std::fs::write(&new_file, "hello").unwrap();
    })
    .await;

    assert!(
        !result.is_empty(),
        "expected a rename event on file creation"
    );
    assert!(
        result.contains("newfile.txt"),
        "expected filename in event, got: {result}"
    );
}

// ── fs.watch emits on the EventEmitter too ────────────────────────────────────

#[tokio::test]
async fn watch_emits_change_event_on_emitter() {
    let dir = TempDir::new().unwrap();
    let file = path_str(&dir, "emitter.txt");
    std::fs::write(&file, "init").unwrap();

    let mut e = engine_rw(&dir).await;

    e.eval(&format!(
        r#"
        var fs = require('fs');
        globalThis.__emitterFired = null;
        var w = fs.watch({file:?});
        w.on('change', function(eventType, filename) {{
            globalThis.__emitterFired = 'yes';
            w.close();
        }});
        "#,
        file = file,
    ))
    .await
    .unwrap();

    let file_clone = file.clone();
    let result = drive_until_watch_event(&mut e, "__emitterFired", move || {
        std::fs::write(&file_clone, "changed").unwrap();
    })
    .await;

    assert_eq!(result, "yes", "EventEmitter 'change' event not fired");
}

// ── fs.watch requires FileRead permission ─────────────────────────────────────

#[tokio::test]
async fn watch_requires_read_permission() {
    let dir = TempDir::new().unwrap();
    let file = path_str(&dir, "secret.txt");
    std::fs::write(&file, "data").unwrap();

    // Engine with NO permissions
    let mut e = JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap();

    let r = e
        .eval_to_string(&format!(
            r#"
            var fs = require('fs');
            var errFired = false;
            try {{
                var w = fs.watch({file:?}, function() {{}});
            }} catch(e) {{
                errFired = true;
            }}
            String(errFired)
            "#,
            file = file,
        ))
        .await
        .unwrap();

    assert_eq!(r, "true", "watch should throw without FileRead permission");
}

// ── fs.watch close() stops events ────────────────────────────────────────────

#[tokio::test]
async fn watch_close_stops_events() {
    let dir = TempDir::new().unwrap();
    let file = path_str(&dir, "closeme.txt");
    std::fs::write(&file, "v0").unwrap();

    let mut e = engine_rw(&dir).await;

    // Open a watcher, close it immediately, then write — should get no events.
    e.eval(&format!(
        r#"
        var fs = require('fs');
        globalThis.__closedEvent = null;
        var w = fs.watch({file:?}, function(t, fn) {{
            globalThis.__closedEvent = t;
        }});
        w.close();
        "#,
        file = file,
    ))
    .await
    .unwrap();

    let file_clone = file.clone();
    // Write the file after close.
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(&file_clone, "v1").unwrap();

    // Drive the event loop briefly — no event should arrive.
    for _ in 0..20 {
        e.idle().await;
        // Timers (setInterval, used by fs.watch's __fsWatchNext poll loop)
        // only fire inside run_event_loop(), not idle() — see the identical
        // pattern in compat_priority.rs's eval_async().
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let r = e
        .eval_to_string("String(globalThis.__closedEvent)")
        .await
        .unwrap();
    assert_eq!(r, "null", "events should not fire after close(): got {r}");
}

// ── fs.watchFile (inotify-backed) ────────────────────────────────────────────

#[tokio::test]
async fn watch_file_detects_modification() {
    let dir = TempDir::new().unwrap();
    let file = path_str(&dir, "polled.txt");
    std::fs::write(&file, "v0").unwrap();

    let mut e = engine_rw(&dir).await;

    e.eval(&format!(
        r#"
        var fs = require('fs');
        globalThis.__polledEvent = null;
        var wh = fs.watchFile({file:?}, function(curr, prev) {{
            globalThis.__polledEvent = 'changed';
            wh.stop();
        }});
        "#,
        file = file,
    ))
    .await
    .unwrap();

    let file_clone = file.clone();
    let result = drive_until_watch_event(&mut e, "__polledEvent", move || {
        std::fs::write(&file_clone, "v1").unwrap();
    })
    .await;

    assert_eq!(
        result, "changed",
        "watchFile should detect file modification"
    );
}
