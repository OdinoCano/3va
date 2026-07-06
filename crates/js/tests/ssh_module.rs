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

// ── Real SSH server (russh::server), used to verify the client end-to-end ──

use russh::keys::{Algorithm, PrivateKey};
use russh::server::{Auth, ChannelOpenHandle, Handler as ServerHandler, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{Attrs, FileAttributes, StatusCode};
use std::collections::HashMap;
use tokio::sync::Mutex as AsyncMutex;

#[derive(Default)]
struct StatSftpHandler;

impl russh_sftp::server::Handler for StatSftpHandler {
    type Error = StatusCode;

    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }

    async fn stat(&mut self, id: u32, _path: String) -> Result<Attrs, Self::Error> {
        let attrs = FileAttributes {
            size: Some(4096),
            mtime: Some(1_700_000_000),
            permissions: Some(0o100644),
            ..Default::default()
        };
        Ok(Attrs { id, attrs })
    }
}

#[derive(Clone)]
struct TestSshServer {
    channels: Arc<AsyncMutex<HashMap<ChannelId, Channel<Msg>>>>,
}

impl russh::server::Server for TestSshServer {
    type Handler = Self;
    fn new_client(&mut self, _addr: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

impl ServerHandler for TestSshServer {
    type Error = anyhow::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "testuser" && password == "testpass" {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.lock().await.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).to_string();
        session.data(channel, format!("echo: {command}").into_bytes())?;
        session.exit_status_request(channel, 0)?;
        session.eof(channel)?;
        session.close(channel)?;
        session.channel_success(channel)?;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if name == "sftp" {
            let channel = self.channels.lock().await.remove(&channel_id).unwrap();
            session.channel_success(channel_id)?;
            tokio::spawn(async move {
                russh_sftp::server::run(channel.into_stream(), StatSftpHandler).await;
            });
        } else {
            session.channel_failure(channel_id)?;
        }
        Ok(())
    }
}

fn start_fake_sshd() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let tokio_listener = tokio::net::TcpListener::from_std(listener).unwrap();

    let config = Arc::new(russh::server::Config {
        auth_rejection_time: std::time::Duration::from_millis(0),
        auth_rejection_time_initial: Some(std::time::Duration::from_millis(0)),
        keys: vec![PrivateKey::random(&mut rand10::rng(), Algorithm::Ed25519).unwrap()],
        ..Default::default()
    });
    let mut server = TestSshServer {
        channels: Arc::new(AsyncMutex::new(HashMap::new())),
    };

    tokio::spawn(async move {
        let running = server.run_on_socket(config, &tokio_listener);
        let _ = running.await;
    });

    port
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

#[tokio::test]
async fn ssh_real_connect_and_exec_over_the_wire() {
    let port = start_fake_sshd();
    let e = engine_with_net("127.0.0.1").await;
    e.eval(
        format!(
            r#"
        var ready = false, execOut = null, execCode = null;
        var client = new ssh.Client();
        client.on('ready', function() {{ ready = true; }});
        client.on('error', function(e) {{ execOut = 'ERR:' + e.message; }});
        client.connect({{ host: '127.0.0.1', port: {port}, username: 'testuser', password: 'testpass' }});
        "#
        )
        .as_str(),
    )
    .await
    .unwrap();

    let mut is_ready = false;
    for _ in 0..300 {
        e.idle().await;
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        if e.eval_to_string("String(ready)").await.unwrap() == "true" {
            is_ready = true;
            break;
        }
    }
    assert!(
        is_ready,
        "client never emitted 'ready' against the real fake sshd"
    );

    e.eval(
        r#"
        var execOut = null;
        client.exec('uptime', function(err, ch) {
            if (err) { execOut = 'ERR:' + err.message; return; }
            ch.stdout.on('data', function(d) { execOut = d.toString(); });
        });
        "#,
    )
    .await
    .unwrap();

    let mut got_output = false;
    for _ in 0..300 {
        e.idle().await;
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        if e.eval_to_string("String(execOut !== null)").await.unwrap() == "true" {
            got_output = true;
            break;
        }
    }
    assert!(
        got_output,
        "exec() never delivered real stdout from the fake sshd"
    );
    let out = e.eval_to_string("String(execOut)").await.unwrap();
    // Confirms real data crossed the wire (the fake server echoes the command).
    assert_eq!(out, "echo: uptime");
}

#[tokio::test]
async fn ssh_real_sftp_stat_returns_real_attrs_not_fallback_zeros() {
    let port = start_fake_sshd();
    let e = engine_with_net("127.0.0.1").await;
    e.eval(
        format!(
            r#"
        var statResult = null;
        var client = new ssh.Client();
        client.on('ready', function() {{
            client.stat('/whatever', function(err, attrs) {{
                statResult = err ? ('ERR:' + err.message) : JSON.stringify(attrs);
            }});
        }});
        client.connect({{ host: '127.0.0.1', port: {port}, username: 'testuser', password: 'testpass' }});
        "#
        )
        .as_str(),
    )
    .await
    .unwrap();

    let mut done = false;
    for _ in 0..300 {
        e.idle().await;
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        if e.eval_to_string("String(statResult !== null)")
            .await
            .unwrap()
            == "true"
        {
            done = true;
            break;
        }
    }
    assert!(
        done,
        "stat() never resolved against the real fake sshd's SFTP subsystem"
    );
    let result = e.eval_to_string("String(statResult)").await.unwrap();
    // The old (broken) JS glue always returned {"size":0,"mtime":0,"mode":0}
    // because it tried to JSON.parse a Promise object. This proves real data
    // now comes back from the fake server's stat() handler.
    assert!(
        result.contains("4096"),
        "expected real size 4096, got: {result}"
    );
    assert!(
        result.contains("1700000000"),
        "expected real mtime, got: {result}"
    );
    assert!(
        !result.contains("\"size\":0"),
        "still returning fallback zeros: {result}"
    );
}
