// When the caller asks for TLS and the handshake fails, the IRC, POP3, MQTT
// and FTP clients must fail. They used to fall back to a plaintext
// connection on the same socket, so an attacker who broke the handshake then
// received the credentials in clear.
// Run: cargo test -p vvva_js --test tls_no_plaintext_fallback

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

/// Accepts one connection, answers the ClientHello with garbage, then
/// records whatever else the client sends.
fn fake_server() -> (u16, std::thread::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        conn.set_read_timeout(Some(std::time::Duration::from_millis(500)))
            .unwrap();
        let mut hello = [0u8; 5];
        let _ = conn.read_exact(&mut hello);
        let _ = conn.write_all(b"this is not TLS\r\n");
        let mut rest = Vec::new();
        let _ = conn.read_to_end(&mut rest);
        rest
    });
    (port, handle)
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_tls_handshake_never_falls_back_to_plaintext() {
    let state = PermissionState::new();
    state.grant(Capability::Network("127.0.0.1".to_string()));
    let mut e = JsEngine::new(Arc::new(state)).await.unwrap();

    for (proto, create) in [
        ("irc", "__ircCreate()"),
        ("pop3", "__pop3Create()"),
        ("mqtt", "__mqttCreate('c')"),
        ("ftp", "__ftpCreate()"),
    ] {
        let (port, server) = fake_server();
        let connect = match proto {
            "irc" => "__ircConnect",
            "pop3" => "__pop3Connect",
            "mqtt" => "__mqttConnect",
            _ => "__ftpConnect",
        };
        // Natives either return or throw their Error; accept both.
        let result = e
            .eval_to_string(&format!(
                "(function() {{ try {{ var r = {connect}({create}, '127.0.0.1', {port}, true); \
                 return r instanceof Error ? 'error: ' + r.message : 'connected'; }} \
                 catch (err) {{ return 'error: ' + err.message; }} }})()"
            ))
            .await
            .unwrap();
        assert!(
            result.contains("TLS connection failed"),
            "{proto}: a broken handshake must fail, got {result}"
        );
        let after = server.join().unwrap();
        // The rest of the ClientHello is binary; what must never appear is a
        // protocol command or credential sent in clear.
        let text = String::from_utf8_lossy(&after);
        for marker in ["NICK", "USER", "PASS", "AUTH", "MQTT", "CAPA"] {
            assert!(
                !text.contains(marker),
                "{proto}: sent {marker} in plaintext after the failed handshake"
            );
        }
    }
}
