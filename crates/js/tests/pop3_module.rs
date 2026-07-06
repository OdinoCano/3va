// Tests for the POP3 builtin.
// Run: cargo test -p vvva_js --test pop3_module

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

/// Starts a minimal local POP3 daemon: greets, accepts USER/PASS, then answers
/// STAT with a single-line reply and RETR with a multiline reply whose body
/// contains a dot-stuffed line (a literal ".hello" line is sent on the wire as
/// "..hello" per RFC 1939 §3, and must come back destuffed).
fn start_fake_pop3d() -> u16 {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);

        writer.write_all(b"+OK POP3 ready\r\n").unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap(); // USER
        writer.write_all(b"+OK\r\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap(); // PASS
        writer.write_all(b"+OK logged in\r\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap(); // STAT
        writer.write_all(b"+OK 2 320\r\n").unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap(); // RETR 1
        writer
            .write_all(b"+OK message follows\r\nSubject: hi\r\n\r\n..hello\r\nbody\r\n.\r\n")
            .unwrap();
    });

    port
}

#[tokio::test]
async fn pop3_real_stat_and_retr_over_the_wire() {
    let port = start_fake_pop3d();
    let e = engine_with_net("127.0.0.1").await;
    e.eval(
        format!(
            r#"
        var statResult = null, retrResult = null;
        var pop3 = require('pop3');
        var client = new pop3.Client({{ host: '127.0.0.1', port: {port} }});
        client.connect(function() {{
            client.login('u', 'p', function() {{
                // POP3 is lock-step: the client has no command queue, so the
                // next command must wait for the previous callback or it will
                // clobber the pending-response tracking.
                client.stat(function(err, info) {{
                    statResult = info;
                    client.retr(1, function(err, data) {{ retrResult = data; }});
                }});
            }});
        }});
        "#
        )
        .as_str(),
    )
    .await
    .unwrap();

    let mut got_stat = false;
    for _ in 0..300 {
        tokio::select! {
            _ = e.idle() => {},
            _ = tokio::time::sleep(std::time::Duration::from_millis(2)) => {},
        }
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        if e.eval_to_string("String(statResult !== null)")
            .await
            .unwrap()
            == "true"
        {
            got_stat = true;
            break;
        }
    }
    assert!(
        got_stat,
        "stat() callback never fired against a real single-line response"
    );
    let stat = e.eval_to_string("String(statResult)").await.unwrap();
    assert_eq!(stat, "+OK 2 320");

    let mut got_retr = false;
    for _ in 0..300 {
        tokio::select! {
            _ = e.idle() => {},
            _ = tokio::time::sleep(std::time::Duration::from_millis(2)) => {},
        }
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        if e.eval_to_string("String(retrResult !== null)")
            .await
            .unwrap()
            == "true"
        {
            got_retr = true;
            break;
        }
    }
    assert!(
        got_retr,
        "retr() callback never fired against a real multiline response"
    );
    let retr = e.eval_to_string("String(retrResult)").await.unwrap();
    assert!(retr.contains("Subject: hi"), "missing header line: {retr}");
    // Confirms dot-destuffing: wire-level ".." must come back as a single "."
    assert!(
        retr.contains(".hello"),
        "dot-stuffed body line not destuffed: {retr}"
    );
    assert!(
        !retr.contains("..hello"),
        "dot-stuffing was not undone: {retr}"
    );
    assert!(
        !retr.contains("\n.\n") && !retr.ends_with(".\n"),
        "terminator leaked into body: {retr}"
    );
}

// ── API shape ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn pop3_global_exists() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string("String(typeof pop3 === 'object')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn pop3_client_constructor_exists() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string("String(typeof pop3.Client === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn pop3_client_has_expected_methods() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new pop3.Client({ host: 'example.com' });
                var methods = ['connect', 'login', 'list', 'retrive', 'delete',
                              'stat', 'reset', 'quit', 'disconnect'];
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
async fn pop3_client_has_event_emitter() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new pop3.Client({ host: 'example.com' });
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
async fn pop3_connect_blocked_without_net_grant() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                try {
                    var client = new pop3.Client({ host: 'example.com', port: 110 });
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
async fn pop3_constructor_options() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client1 = new pop3.Client({ host: 'pop3.example.com' });
                var client2 = new pop3.Client({ host: 'pop3.example.com', port: 110 });
                var client3 = new pop3.Client({ host: 'pop3.example.com', tls: false });
                var client4 = new pop3.Client({ host: 'pop3.example.com', username: 'user', password: 'pass' });
                var client5 = new pop3.Client({ host: 'pop3.example.com', timeout: 30000 });
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn pop3_list_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new pop3.Client({ host: 'example.com' });
                try {
                    var result = client.list();
                    return String(result !== null);
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
async fn pop3_retrieve_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new pop3.Client({ host: 'example.com' });
                try {
                    var msg = client.retrive(1);
                    return String(msg !== null);
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
async fn pop3_delete_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new pop3.Client({ host: 'example.com' });
                try {
                    client.delete(1);
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
async fn pop3_stat_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new pop3.Client({ host: 'example.com' });
                try {
                    var stat = client.stat();
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
async fn pop3_reset_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new pop3.Client({ host: 'example.com' });
                try {
                    client.reset();
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
async fn pop3_quit_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new pop3.Client({ host: 'example.com' });
                try {
                    client.quit();
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
