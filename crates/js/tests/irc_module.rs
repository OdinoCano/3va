// Tests for the IRC builtin.
// Run: cargo test -p vvva_js --test irc_module

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

/// Starts a minimal local IRC daemon on a random port: reads the NICK/USER
/// registration lines, replies with 001 (RPL_WELCOME), sends a PING the real
/// client must PONG back, then sends a PRIVMSG. Returns (port, handle) where
/// the handle's `join()` yields the raw bytes the server received from the
/// client (so the test can assert the real PONG went out over the wire).
fn start_fake_ircd() -> (u16, std::thread::JoinHandle<Vec<u8>>) {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let handle = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);

        // Drain registration (PASS/NICK/USER) — real clients send these first.
        let mut saw_user = false;
        while !saw_user {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                return Vec::new();
            }
            if line.starts_with("USER ") {
                saw_user = true;
            }
        }

        writer
            .write_all(b":test.local 001 nick :Welcome to the test network\r\n")
            .unwrap();
        writer.write_all(b"PING :handshake-token\r\n").unwrap();

        // Real client must answer the PING with a real PONG over the wire.
        let mut pong_line = String::new();
        reader.read_line(&mut pong_line).unwrap();

        writer
            .write_all(b":friend!u@host PRIVMSG nick :hello from the wire\r\n")
            .unwrap();

        pong_line.into_bytes()
    });

    (port, handle)
}

#[tokio::test]
async fn irc_real_connect_registers_and_pongs_over_the_wire() {
    let (port, server) = start_fake_ircd();
    let mut e = engine_with_net("127.0.0.1").await;
    e.eval(
        format!(
            r#"
        var registered = false;
        var pmFrom = null, pmText = null;
        var client = new irc.Client({{ host: '127.0.0.1', port: {port}, nick: 'nick' }});
        client.on('registered', function() {{ registered = true; }});
        client.on('message', function(from, to, text) {{ pmFrom = from; pmText = text; }});
        client.connect();
        "#
        )
        .as_str(),
    )
    .await
    .unwrap();

    // connect()/​_startPoll drive via setTimeout, which needs run_event_loop()
    // pumped alongside idle() (see crates/js/tests/net_server.rs::drive_forever)
    // — idle() alone only advances Promise-based async, not JS timers.
    let mut registered = false;
    for _ in 0..500 {
        tokio::select! {
            _ = e.idle() => {},
            _ = tokio::time::sleep(std::time::Duration::from_millis(2)) => {},
        }
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        if e.eval_to_string("String(registered)").await.unwrap() == "true" {
            registered = true;
            break;
        }
    }
    assert!(
        registered,
        "client never emitted 'registered' (no real 001 handling)"
    );

    let mut got_message = false;
    for _ in 0..500 {
        tokio::select! {
            _ = e.idle() => {},
            _ = tokio::time::sleep(std::time::Duration::from_millis(2)) => {},
        }
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
        if e.eval_to_string("String(pmFrom !== null)").await.unwrap() == "true" {
            got_message = true;
            break;
        }
    }
    assert!(
        got_message,
        "client never parsed the real PRIVMSG from the wire"
    );

    let from = e.eval_to_string("String(pmFrom)").await.unwrap();
    let text = e.eval_to_string("String(pmText)").await.unwrap();
    assert_eq!(from, "friend");
    assert_eq!(text, "hello from the wire");

    // Proves the PING->PONG round-trip actually crossed the real socket,
    // not just JS-internal event plumbing.
    let pong_line = tokio::task::spawn_blocking(move || server.join().unwrap())
        .await
        .unwrap();
    let pong_line = String::from_utf8(pong_line).unwrap();
    assert_eq!(pong_line.trim_end(), "PONG :handshake-token");
}

// ── API shape ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn irc_global_exists() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string("String(typeof irc === 'object')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn irc_client_constructor_exists() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string("String(typeof irc.Client === 'function')")
        .await
        .unwrap();
    assert_eq!(r, "true");
}

#[tokio::test]
async fn irc_client_has_expected_methods() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new irc.Client({ host: 'example.com' });
                var methods = ['connect', 'nick', 'user', 'join', 'part',
                              'quit', 'privmsg', 'notice', 'kick', 'mode',
                              'raw', 'disconnect'];
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
async fn irc_client_has_event_emitter() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new irc.Client({ host: 'example.com' });
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
async fn irc_connect_blocked_without_net_grant() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                try {
                    var client = new irc.Client({ host: 'example.com', port: 6667 });
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
async fn irc_constructor_options() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client1 = new irc.Client({ host: 'irc.example.com' });
                var client2 = new irc.Client({ host: 'irc.example.com', port: 6697 });
                var client3 = new irc.Client({ host: 'irc.example.com', tls: true });
                var client4 = new irc.Client({ host: 'irc.example.com', nick: 'mybot', username: 'mybot', realname: 'My Bot' });
                return 'ok';
            })()
            "#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok");
}

#[tokio::test]
async fn irc_events_registerable() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new irc.Client({ host: 'example.com' });
                var events = ['connect', 'disconnect', 'error', 'nick', 'join', 'part',
                             'quit', 'message', 'notice', 'kick', 'mode', 'privmsg'];
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
async fn irc_privmsg_api() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new irc.Client({ host: 'example.com' });
                try {
                    client.privmsg('#channel', 'Hello world');
                    client.privmsg('nickname', 'Hello');
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
async fn irc_join_part_api() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new irc.Client({ host: 'example.com' });
                try {
                    client.join('#channel');
                    client.join('#channel', 'password');
                    client.part('#channel');
                    client.part('#channel', 'part message');
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
async fn irc_raw_api() {
    let mut e = engine_no_net().await;
    let r = e
        .eval_to_string(
            r#"
            (function() {
                var client = new irc.Client({ host: 'example.com' });
                try {
                    client.raw('PING :12345');
                    client.raw('NOTICE #channel :Hello');
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
