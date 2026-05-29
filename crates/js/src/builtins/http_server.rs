//! HTTP/1.1 server backend for `http.createServer()`.
//!
//! Rust primitives exposed to JS:
//!   __httpListen(port, host) → server_id   synchronous; binds immediately via std + Tokio
//!   __httpAcceptAsync(server_id) → `Promise<JSON>`   awaits next connection
//!   __httpRespond(conn_id, status, status_text, headers_json, body) → void  (sync)
//!   __httpClose(server_id) → void  (sync)
//!
//! The sync bind means the port is available the moment `server.listen()` returns in JS,
//! so tests and user code don't need to wait for an async bind promise to resolve.

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream as StdTcpStream;
use std::sync::{Arc, Mutex};

use rquickjs::function::Async;
use rquickjs::{Ctx, Function, Result};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::net::TcpListener;

use vvva_permissions::{Capability, PermissionState};

struct ConnEntry {
    stream: StdTcpStream,
}

fn js_err(ctx: &Ctx<'_>, msg: String) -> rquickjs::Error {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    match ctx.eval::<rquickjs::Value<'_>, _>(format!("new Error(\"{}\")", escaped)) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
    }
}

fn js_code_err(ctx: &Ctx<'_>, code: &str, msg: &str) -> rquickjs::Error {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    let src = format!(
        "(function(){{var e=new Error(\"{msg}\");e.code=\"{code}\";return e;}})()",
        msg = escaped,
        code = code,
    );
    match ctx.eval::<rquickjs::Value<'_>, _>(src) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

/// Parse an HTTP/1.1 request from a Tokio TcpStream.
async fn parse_request(
    stream: tokio::net::TcpStream,
) -> std::result::Result<(String, String, Vec<(String, String)>, Vec<u8>, StdTcpStream), String> {
    let mut reader = BufReader::new(stream);

    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .await
        .map_err(|e| e.to_string())?;
    let request_line = request_line.trim_end_matches(['\r', '\n']);

    let mut parts = request_line.splitn(3, ' ');
    let method = parts.next().unwrap_or("GET").to_string();
    let path = parts.next().unwrap_or("/").to_string();

    let mut headers: Vec<(String, String)> = Vec::new();
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .map_err(|e| e.to_string())?;
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some(colon) = trimmed.find(':') {
            let name = trimmed[..colon].trim().to_lowercase();
            let value = trimmed[colon + 1..].trim().to_string();
            if name == "content-length" {
                content_length = value.parse::<usize>().unwrap_or(0).min(100 * 1024 * 1024);
            }
            headers.push((name, value));
        }
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader
            .read_exact(&mut body)
            .await
            .map_err(|e| e.to_string())?;
    }

    let tokio_stream = reader.into_inner();
    let std_stream = tokio_stream.into_std().map_err(|e| e.to_string())?;

    Ok((method, path, headers, body, std_stream))
}

pub fn inject_http_server(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    // server_id → Arc<TcpListener> (Arc lets accept close share it)
    let servers: Arc<Mutex<HashMap<u32, Arc<TcpListener>>>> = Arc::new(Mutex::new(HashMap::new()));
    let conns: Arc<Mutex<HashMap<u32, ConnEntry>>> = Arc::new(Mutex::new(HashMap::new()));
    let next_server_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let next_conn_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(1));

    // ── __httpListen(port, host) → server_id  (synchronous) ──────────────────
    // Binds immediately using std, then registers with Tokio I/O driver.
    {
        let perms = permissions.clone();
        let servers = servers.clone();
        let nid = next_server_id.clone();
        ctx.globals().set(
            "__httpListen",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, port: u16, host: String| -> Result<u32> {
                    if !perms.check(&Capability::Network(host.clone())) {
                        return Err(js_code_err(
                            &ctx,
                            "EACCES",
                            &format!("Network access denied. Run with --allow-net={}", host),
                        ));
                    }

                    // Synchronous bind via std — the OS immediately starts queuing connections.
                    let std_listener = std::net::TcpListener::bind(format!("{}:{}", host, port))
                        .map_err(|e| js_code_err(&ctx, "EADDRINUSE", &e.to_string()))?;
                    // Non-blocking is required by tokio::net::TcpListener::from_std.
                    std_listener
                        .set_nonblocking(true)
                        .map_err(|e| js_err(&ctx, e.to_string()))?;

                    // Register with Tokio's I/O driver (must be called from inside a Tokio runtime).
                    let tokio_listener = TcpListener::from_std(std_listener)
                        .map_err(|e| js_err(&ctx, format!("TcpListener::from_std: {}", e)))?;

                    let id = {
                        let mut n = nid.lock().unwrap();
                        let id = *n;
                        *n = n.wrapping_add(1);
                        id
                    };
                    servers.lock().unwrap().insert(id, Arc::new(tokio_listener));
                    Ok(id)
                },
            ),
        )?;
    }

    // ── __httpAcceptAsync(server_id) → Promise<JSON string> ──────────────────
    {
        let servers = servers.clone();
        let conns = conns.clone();
        let nid = next_conn_id.clone();
        ctx.globals().set(
            "__httpAcceptAsync",
            Function::new(
                ctx.clone(),
                Async(move |server_id: u32| {
                    let servers = servers.clone();
                    let conns = conns.clone();
                    let nid = nid.clone();
                    async move {
                        let listener = {
                            let guard = servers.lock().unwrap();
                            guard.get(&server_id).cloned()
                        };
                        let listener = listener.ok_or_else(|| {
                            rquickjs::Error::new_from_js_message(
                                "ENOENT",
                                "ENOENT",
                                "unknown server id".to_string(),
                            )
                        })?;

                        let (stream, _addr) = listener.accept().await.map_err(|e| {
                            rquickjs::Error::new_from_js_message(
                                "ECONNRESET",
                                "ECONNRESET",
                                e.to_string(),
                            )
                        })?;

                        let (method, url, headers, body, std_stream) =
                            parse_request(stream).await.map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e)
                            })?;

                        let conn_id = {
                            let mut n = nid.lock().unwrap();
                            let id = *n;
                            *n = n.wrapping_add(1);
                            id
                        };
                        conns
                            .lock()
                            .unwrap()
                            .insert(conn_id, ConnEntry { stream: std_stream });

                        let hdr_pairs: Vec<String> = headers
                            .iter()
                            .map(|(k, v)| {
                                format!("\"{}\":\"{}\"", json_escape(k), json_escape(v))
                            })
                            .collect();
                        let body_str = String::from_utf8_lossy(&body);
                        let json = format!(
                            "{{\"method\":\"{method}\",\"url\":\"{url}\",\"headers\":{{{headers}}},\"body\":\"{body}\",\"conn_id\":{conn_id}}}",
                            method = json_escape(&method),
                            url = json_escape(&url),
                            headers = hdr_pairs.join(","),
                            body = json_escape(&body_str),
                            conn_id = conn_id,
                        );

                        Ok::<String, rquickjs::Error>(json)
                    }
                }),
            ),
        )?;
    }

    // ── __httpRespond(conn_id, status, status_text, headers_json, body) ───────
    {
        let conns = conns.clone();
        ctx.globals().set(
            "__httpRespond",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>,
                      conn_id: u32,
                      status: u32,
                      status_text: String,
                      headers_json: String,
                      body: String|
                      -> Result<()> {
                    let body_bytes = body.as_bytes();

                    let extra_headers: Vec<(String, String)> = serde_json::from_str(&headers_json)
                        .ok()
                        .and_then(|v: serde_json::Value| {
                            v.as_object().map(|obj| {
                                obj.iter()
                                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                                    .collect()
                            })
                        })
                        .unwrap_or_default();

                    let mut response = format!("HTTP/1.1 {} {}\r\n", status, status_text);
                    response.push_str(&format!("Content-Length: {}\r\n", body_bytes.len()));
                    response.push_str("Connection: close\r\n");

                    for (k, v) in &extra_headers {
                        let kl = k.to_lowercase();
                        if kl != "content-length" && kl != "connection" {
                            response.push_str(&format!("{}: {}\r\n", k, v));
                        }
                    }
                    response.push_str("\r\n");

                    let mut guard = conns.lock().unwrap();
                    let conn = guard
                        .get_mut(&conn_id)
                        .ok_or_else(|| js_code_err(&ctx, "ENOENT", "unknown conn_id"))?;

                    conn.stream
                        .write_all(response.as_bytes())
                        .map_err(|e| js_err(&ctx, e.to_string()))?;
                    conn.stream
                        .write_all(body_bytes)
                        .map_err(|e| js_err(&ctx, e.to_string()))?;
                    conn.stream.flush().ok();

                    drop(guard);
                    conns.lock().unwrap().remove(&conn_id);
                    Ok(())
                },
            ),
        )?;
    }

    // ── __httpClose(server_id) ────────────────────────────────────────────────
    {
        let servers = servers.clone();
        ctx.globals().set(
            "__httpClose",
            Function::new(
                ctx.clone(),
                move |_ctx: Ctx<'_>, server_id: u32| -> Result<()> {
                    servers.lock().unwrap().remove(&server_id);
                    Ok(())
                },
            ),
        )?;
    }

    Ok(())
}
