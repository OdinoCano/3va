// Tests for the FTP builtin.
// Run: cargo test -p vvva_js --test ftp_module

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

/// Starts a minimal local FTP daemon: greets, does USER/PASS, then serves one
/// PASV+RETR round (download) followed by one PASV+STOR round (upload).
/// Returns the control port and a handle yielding the bytes it received via
/// STOR, so the test can confirm put() actually put something on the wire.
fn start_fake_ftpd() -> (u16, std::thread::JoinHandle<Vec<u8>>) {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    let control_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let control_port = control_listener.local_addr().unwrap().port();

    let handle = std::thread::spawn(move || {
        let (control, _) = control_listener.accept().unwrap();
        let mut cw = control.try_clone().unwrap();
        let mut cr = BufReader::new(control);
        let mut line = String::new();

        cw.write_all(b"220 fake ftpd ready\r\n").unwrap();
        cr.read_line(&mut line).unwrap(); // USER
        cw.write_all(b"331 need password\r\n").unwrap();
        line.clear();
        cr.read_line(&mut line).unwrap(); // PASS
        cw.write_all(b"230 logged in\r\n").unwrap();

        // RETR round
        line.clear();
        cr.read_line(&mut line).unwrap(); // PASV
        let data_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let data_port = data_listener.local_addr().unwrap().port();
        cw.write_all(
            format!(
                "227 Entering Passive Mode (127,0,0,1,{},{})\r\n",
                data_port >> 8,
                data_port & 0xff
            )
            .as_bytes(),
        )
        .unwrap();
        line.clear();
        cr.read_line(&mut line).unwrap(); // RETR <path>
        cw.write_all(b"150 opening data connection\r\n").unwrap();
        let (mut data, _) = data_listener.accept().unwrap();
        data.write_all(b"hello from ftp").unwrap();
        drop(data);
        cw.write_all(b"226 transfer complete\r\n").unwrap();

        // STOR round
        line.clear();
        cr.read_line(&mut line).unwrap(); // PASV
        let data_listener2 = TcpListener::bind("127.0.0.1:0").unwrap();
        let data_port2 = data_listener2.local_addr().unwrap().port();
        cw.write_all(
            format!(
                "227 Entering Passive Mode (127,0,0,1,{},{})\r\n",
                data_port2 >> 8,
                data_port2 & 0xff
            )
            .as_bytes(),
        )
        .unwrap();
        line.clear();
        cr.read_line(&mut line).unwrap(); // STOR <path>
        cw.write_all(b"150 ok to send data\r\n").unwrap();
        let (mut data2, _) = data_listener2.accept().unwrap();
        let mut received = Vec::new();
        data2.read_to_end(&mut received).unwrap();
        cw.write_all(b"226 transfer complete\r\n").unwrap();

        received
    });

    (control_port, handle)
}

#[tokio::test]
async fn ftp_real_login_get_and_put_over_the_wire() {
    let (port, server) = start_fake_ftpd();
    let e = engine_with_net("127.0.0.1").await;
    e.eval(
        format!(
            r#"
        var gotData = null, putErr = 'notset';
        var ftp = require('ftp');
        var client = new ftp.Client();
        client.connect({{ host: '127.0.0.1', port: {port}, username: 'u', password: 'p' }});
        client.on('ready', function() {{
            client.get('/file.txt', function(err, data) {{
                gotData = err ? ('ERR:' + err.message) : new TextDecoder().decode(data);
                client.put('upload content', '/up.txt', function(err2) {{
                    putErr = err2 ? err2.message : null;
                }});
            }});
        }});
        "#
        )
        .as_str(),
    )
    .await
    .unwrap();

    // Every step below is fast in wall-clock terms, but `run_event_loop()`
    // stays busy for a while per call as long as the control connection is
    // open and its EAGAIN-backoff poll keeps rescheduling itself — so this
    // waits on a generous overall budget rather than expecting any single
    // `run_event_loop()` call to return quickly.
    let ok = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            tokio::select! {
                _ = e.idle() => {},
                _ = tokio::time::sleep(std::time::Duration::from_millis(2)) => {},
            }
            let _ = e.run_event_loop().await;
            tokio::task::yield_now().await;
            if e.eval_to_string("String(putErr !== 'notset')")
                .await
                .unwrap()
                == "true"
            {
                break;
            }
        }
    })
    .await
    .is_ok();
    assert!(
        ok,
        "get()/put() never resolved against the real FTP round-trip within 10s"
    );

    let data = e.eval_to_string("String(gotData)").await.unwrap();
    // Confirms get() is no longer the old no-op that just echoed the path back.
    assert_eq!(data, "hello from ftp");
    let put_err = e.eval_to_string("String(putErr)").await.unwrap();
    assert_eq!(put_err, "null", "put() reported an error: {put_err}");

    let uploaded = tokio::task::spawn_blocking(move || server.join().unwrap())
        .await
        .unwrap();
    // Confirms put() actually wrote the given bytes to the data connection,
    // not the old no-op that never touched the socket at all.
    assert_eq!(String::from_utf8(uploaded).unwrap(), "upload content");
}

#[tokio::test]
async fn ftp_control_reply_times_out_instead_of_hanging_the_engine() {
    use std::net::TcpListener;
    // Accepts the connection but never sends anything — not even the 220
    // greeting — simulating a stalled/hung server.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let _keep = listener.accept().unwrap();
        std::thread::sleep(std::time::Duration::from_secs(10));
    });

    let e = engine_with_net("127.0.0.1").await;
    e.eval(
        format!(
            r#"
        var errCode = null;
        var ftp = require('ftp');
        var client = new ftp.Client();
        client.connect({{ host: '127.0.0.1', port: {port}, timeout: 200 }});
        client.on('error', function(e) {{ errCode = e.code; }});
        "#
        )
        .as_str(),
    )
    .await
    .unwrap();

    // The whole point: this loop (and the engine driving it) must keep making
    // progress on its own — no per-iteration wall-clock budget larger than a
    // couple hundred ms — instead of blocking inside a native call for the
    // full 10s the fake server stays silent.
    let mut timed_out = false;
    for _ in 0..500 {
        tokio::select! {
            _ = e.idle() => {},
            _ = tokio::time::sleep(std::time::Duration::from_millis(2)) => {},
        }
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        if e.eval_to_string("String(errCode)").await.unwrap() == "ETIMEDOUT" {
            timed_out = true;
            break;
        }
    }
    assert!(
        timed_out,
        "connect() never timed out — engine may be blocked in a native retry loop"
    );
}

// ── API shape ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ftp_global_exists() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string("String(typeof ftp === 'object')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn ftp_client_constructor_exists() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string("String(typeof ftp.Client === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn ftp_client_has_expected_methods() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ftp.Client({ host: 'example.com' });
                var methods = ['connect', 'login', 'list', 'get', 'put',
                              'mkdir', 'rmdir', 'delete', 'rename',
                              'pwd', 'cwd', 'disconnect'];
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
async fn ftp_client_has_event_emitter() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ftp.Client({ host: 'example.com' });
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
async fn ftp_connect_blocked_without_net_grant() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                try {
                    var client = new ftp.Client({ host: 'example.com', port: 21 });
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
async fn ftp_constructor_options() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client1 = new ftp.Client({ host: 'ftp.example.com' });
                var client2 = new ftp.Client({ host: 'ftp.example.com', port: 21 });
                var client3 = new ftp.Client({ host: 'ftp.example.com', tls: true });
                var client4 = new ftp.Client({ host: 'ftp.example.com', user: 'anonymous', password: 'guest' });
                var client5 = new ftp.Client({ host: 'ftp.example.com', secure: true });
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn ftp_list_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ftp.Client({ host: 'example.com' });
                try {
                    var result = client.list('/');
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
async fn ftp_get_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ftp.Client({ host: 'example.com' });
                try {
                    var stream = client.get('file.txt');
                    return String(stream !== null);
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
async fn ftp_put_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ftp.Client({ host: 'example.com' });
                try {
                    var stream = client.put('remote.txt');
                    return String(stream !== null);
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
async fn ftp_mkdir_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ftp.Client({ host: 'example.com' });
                try {
                    client.mkdir('/newdir');
                    client.mkdir('/newdir/subdir', true);
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
async fn ftp_rmdir_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ftp.Client({ host: 'example.com' });
                try {
                    client.rmdir('/olddir');
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
async fn ftp_delete_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ftp.Client({ host: 'example.com' });
                try {
                    client.delete('/file.txt');
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
async fn ftp_rename_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ftp.Client({ host: 'example.com' });
                try {
                    client.rename('/old.txt', '/new.txt');
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
async fn ftp_pwd_api() {
    let e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new ftp.Client({ host: 'example.com' });
                try {
                    var dir = client.pwd();
                    return String(dir !== null);
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
