// Tests for the SSH builtin.
// Run: cargo test -p vvva_js --test ssh_module

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

#[allow(dead_code)]
async fn engine_with_net(host: &str) -> JsEngine {
    let state = PermissionState::new();
    state.grant(Capability::Network(host.to_string()));
    JsEngine::new(Arc::new(state)).await.unwrap()
}

async fn engine_no_net() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

// ── API shape ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ssh_global_exists() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string("String(typeof ssh === 'object')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn ssh_client_constructor_exists() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string("String(typeof ssh.Client === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn ssh_client_has_expected_methods() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ssh.Client({ host: 'example.com' });
                var methods = ['connect', 'disconnect', 'exec', 'sftp',
                              'readFile', 'writeFile', 'stat', 'mkdir',
                              'rmdir', 'unlink', 'rename', 'readdir'];
                var missing = [];
                for (var i = 0; i < methods.length; i++) {
                    if (typeof client[methods[i]] !== 'function') {
                        missing.push(methods[i]);
                    }
                }
                return missing.length === 0 ? 'true' : 'missing: ' + missing.join(',');
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn ssh_client_has_event_emitter() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ssh.Client({ host: 'example.com' });
                return String(
                    typeof client.on === 'function' &&
                    typeof client.off === 'function' &&
                    typeof client.emit === 'function'
                );
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

// ── Permission enforcement ───────────────────────────────────────────────────

#[tokio::test]
async fn ssh_connect_blocked_without_net_grant() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                try {
                    var client = new ssh.Client({ host: 'example.com', port: 22 });
                    return 'created';
                } catch(e) {
                    return 'threw:' + e.code;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "created");
}

#[tokio::test]
async fn ssh_constructor_options() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client1 = new ssh.Client({ host: 'ssh.example.com' });
                var client2 = new ssh.Client({ host: 'ssh.example.com', port: 22 });
                var client3 = new ssh.Client({ host: 'ssh.example.com', username: 'user' });
                var client4 = new ssh.Client({ host: 'ssh.example.com', password: 'pass' });
                var client5 = new ssh.Client({ host: 'ssh.example.com', privateKey: 'key' });
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn ssh_exec_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ssh.Client({ host: 'example.com' });
                try {
                    var channel = client.exec('ls');
                    return String(channel !== null);
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn ssh_sftp_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ssh.Client({ host: 'example.com' });
                try {
                    var sftp = client.sftp();
                    return String(sftp !== null);
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn ssh_readfile_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ssh.Client({ host: 'example.com' });
                try {
                    var content = client.readFile('/path/to/file');
                    return 'ok';
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn ssh_writefile_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ssh.Client({ host: 'example.com' });
                try {
                    client.writeFile('/path/to/file', 'content');
                    return 'ok';
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn ssh_stat_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ssh.Client({ host: 'example.com' });
                try {
                    var stat = client.stat('/path/to/file');
                    return String(stat !== null);
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn ssh_mkdir_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ssh.Client({ host: 'example.com' });
                try {
                    client.mkdir('/new/dir');
                    return 'ok';
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn ssh_rmdir_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ssh.Client({ host: 'example.com' });
                try {
                    client.rmdir('/old/dir');
                    return 'ok';
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn ssh_unlink_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ssh.Client({ host: 'example.com' });
                try {
                    client.unlink('/path/to/file');
                    return 'ok';
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn ssh_rename_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ssh.Client({ host: 'example.com' });
                try {
                    client.rename('/old/path', '/new/path');
                    return 'ok';
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn ssh_readdir_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ssh.Client({ host: 'example.com' });
                try {
                    var entries = client.readdir('/path');
                    return 'ok';
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn ssh_events_registerable() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ssh.Client({ host: 'example.com' });
                var events = ['connect', 'disconnect', 'error', 'ready', 'close'];
                var registered = 0;
                events.forEach(function(ev) {
                    try {
                        client.on(ev, function() {});
                        registered++;
                    } catch(e) {}
                });
                return String(registered === events.length);
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "true");
}
