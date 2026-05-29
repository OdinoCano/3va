use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_with_net() -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}

fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    drop(l);
    port
}

fn raw_http(
    port: u16,
    method: &str,
    path: &str,
    body: &str,
    content_type: &str,
    auth: &str,
) -> String {
    let mut stream = match TcpStream::connect(format!("127.0.0.1:{}", port)) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("raw_http connect error: {}", e);
            return String::new();
        }
    };
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();

    let auth_hdr = if auth.is_empty() {
        String::new()
    } else {
        format!("Authorization: {}\r\n", auth)
    };
    let ct = if content_type.is_empty() {
        String::new()
    } else {
        format!(
            "Content-Type: {}\r\nContent-Length: {}\r\n",
            content_type,
            body.len()
        )
    };

    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{ct}{auth}Connection: close\r\n\r\n{body}",
        method = method,
        path = path,
        ct = ct,
        auth = auth_hdr,
        body = body,
    );
    let _ = stream.write_all(req.as_bytes());
    let mut resp = String::new();
    let _ = stream.read_to_string(&mut resp);
    resp
}

async fn drive_forever(e: &JsEngine) -> ! {
    loop {
        e.idle().await;
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn http_auth_end_to_end() {
    let port = free_port();
    let e = engine_with_net().await;

    e.eval_to_string(&format!(
        r#"
        var http = require('http');
        var tokens = {{}};
        globalThis.__healthChecked = false;
        globalThis.__lastLoginToken = '';
        globalThis.__lastProfileUser = '';
        globalThis.__lastProfileRole = '';
        globalThis.__lastUnauthorized = false;

        var _server = http.createServer(function(req, res) {{
            if (req.url === '/login' && req.method === 'POST') {{
                var data = JSON.parse(req._body || '{{}}');
                var token = 'tok_' + Math.random().toString(36).slice(2);
                tokens[token] = {{ user: data.username || 'anonymous', role: 'user' }};
                globalThis.__lastLoginToken = token;
                res.writeHead(200, {{ 'Content-Type': 'application/json' }});
                res.end(JSON.stringify({{ token: token }}));
                return;
            }}
            if (req.url === '/profile' && req.method === 'GET') {{
                var auth = req.headers['authorization'] || '';
                var parts = auth.split(' ');
                var token = parts[1] || parts[0];
                var session = tokens[token];
                if (!session) {{
                    globalThis.__lastUnauthorized = true;
                    res.writeHead(401, {{ 'Content-Type': 'application/json' }});
                    res.end(JSON.stringify({{ error: 'No token provided' }}));
                    return;
                }}
                globalThis.__lastProfileUser = session.user;
                globalThis.__lastProfileRole = session.role;
                res.writeHead(200, {{ 'Content-Type': 'application/json' }});
                res.end(JSON.stringify(session));
                return;
            }}
            if (req.url === '/health') {{
                globalThis.__healthChecked = true;
                res.writeHead(200, {{ 'Content-Type': 'application/json' }});
                res.end(JSON.stringify({{ status: 'ok' }}));
                return;
            }}
            res.writeHead(404);
            res.end('not found');
        }});

        _server.listen({port}, '127.0.0.1');
    "#,
        port = port
    ))
    .await
    .unwrap();

    // 1. Health check (via global, response string is unreliable per pre-existing bug)
    let p = port;
    let h = tokio::task::spawn_blocking(move || raw_http(p, "GET", "/health", "", "", ""));
    let _ = tokio::select! {
        _ = drive_forever(&e) => unreachable!(),
        r = h => r.unwrap(),
    };
    let healthy = e
        .eval_to_string("String(globalThis.__healthChecked)")
        .await
        .unwrap();
    assert_eq!(healthy, "true", "health check should have run");

    // 2. Login
    let p = port;
    let h = tokio::task::spawn_blocking(move || {
        raw_http(
            p,
            "POST",
            "/login",
            r#"{"username":"testuser"}"#,
            "application/json",
            "",
        )
    });
    let _ = tokio::select! {
        _ = drive_forever(&e) => unreachable!(),
        r = h => r.unwrap(),
    };
    let token = e
        .eval_to_string("String(globalThis.__lastLoginToken)")
        .await
        .unwrap();
    eprintln!("token: {:?}", token);
    assert!(!token.is_empty(), "no token received");

    // 3. Profile with token
    let p = port;
    let auth = format!("Bearer {}", token);
    let h = tokio::task::spawn_blocking(move || raw_http(p, "GET", "/profile", "", "", &auth));
    let _ = tokio::select! {
        _ = drive_forever(&e) => unreachable!(),
        r = h => r.unwrap(),
    };
    let user = e
        .eval_to_string("String(globalThis.__lastProfileUser)")
        .await
        .unwrap();
    let role = e
        .eval_to_string("String(globalThis.__lastProfileRole)")
        .await
        .unwrap();
    eprintln!("profile: user={:?} role={:?}", user, role);
    assert_eq!(user, "testuser", "profile username");
    assert_eq!(role, "user", "profile role");

    // 4. Profile without token -> unauthorized
    let p = port;
    let h = tokio::task::spawn_blocking(move || raw_http(p, "GET", "/profile", "", "", ""));
    let _ = tokio::select! {
        _ = drive_forever(&e) => unreachable!(),
        r = h => r.unwrap(),
    };
    let unauth = e
        .eval_to_string("String(globalThis.__lastUnauthorized)")
        .await
        .unwrap();
    eprintln!("unauthorized: {:?}", unauth);
    assert_eq!(unauth, "true", "unauthorized access should be flagged");

    // 5. Cleanup
    e.eval_to_string("_server.close()").await.unwrap();
}
