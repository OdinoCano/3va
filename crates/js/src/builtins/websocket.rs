use rquickjs::{Ctx, Function, Result, function::Rest};
use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket as TungsteniteWs, connect};
use vvva_permissions::{Capability, PermissionState};

type WsConn = TungsteniteWs<MaybeTlsStream<TcpStream>>;

/// Shared pool of active outgoing WebSocket connections keyed by numeric ID.
///
/// The pool is created by [`inject_websocket`] and held by [`JsEngine`] so the engine
/// can drain it gracefully on shutdown via [`drain_ws_pool`].
pub type WsPool = Arc<Mutex<HashMap<u32, WsConn>>>;

/// Gracefully close every connection in `pool` and block until all are gone or
/// `max_wait` elapses.
///
/// ## Algorithm
///
/// For each open connection the function:
/// 1. Sleeps a random **jitter** of 100–500 ms (bounded by remaining budget) so that
///    remote peers restart in a staggered pattern — avoiding a thundering-herd
///    reconnect storm against the upstream service.
/// 2. Sends a `Close(1001 Going Away)` frame with reason `"Server shutting down"`.
/// 3. Flushes the underlying TCP stream so the frame is delivered even if the kernel
///    buffer has not been flushed yet.
/// 4. Removes the entry from the pool.
///
/// If `max_wait` runs out before every connection is processed, the remaining entries
/// are force-removed from the pool (the TCP stack will send a RST on drop).
///
/// ## Example
///
/// ```rust,ignore
/// use std::{collections::HashMap, sync::{Arc, Mutex}, time::Duration};
/// use vvva_js::builtins::websocket::{WsPool, drain_ws_pool};
///
/// let pool: WsPool = Arc::new(Mutex::new(HashMap::new()));
/// // … populate pool with tungstenite connections …
/// drain_ws_pool(&pool, Duration::from_secs(30));
/// assert!(pool.lock().unwrap().is_empty());
/// ```
pub fn drain_ws_pool(pool: &WsPool, max_wait: std::time::Duration) {
    use rand::Rng;
    let deadline = std::time::Instant::now() + max_wait;
    let mut rng = rand::thread_rng();

    let ids: Vec<u32> = pool.lock().unwrap().keys().copied().collect();
    for id in ids {
        let now = std::time::Instant::now();
        if now >= deadline {
            break;
        }
        let remaining = deadline - now;
        let jitter = std::time::Duration::from_millis(rng.gen_range(100..=500));
        std::thread::sleep(jitter.min(remaining));

        let mut guard = pool.lock().unwrap();
        if let Some(ws) = guard.get_mut(&id) {
            let _ = ws.close(Some(tungstenite::protocol::CloseFrame {
                code: tungstenite::protocol::frame::coding::CloseCode::Away,
                reason: "Server shutting down".into(),
            }));
            let _ = ws.flush();
        }
        guard.remove(&id);
    }
    pool.lock().unwrap().clear();
}

fn js_err<'js>(ctx: &Ctx<'js>, msg: String) -> rquickjs::Error {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    match ctx.eval::<rquickjs::Value, _>(format!("new Error(\"{}\")", escaped).as_str()) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
    }
}

fn host_from_url(url: &str) -> Option<String> {
    let after_scheme = url.find("://")?;
    let rest = &url[after_scheme + 3..];
    let host_part = rest.split('/').next().unwrap_or(rest);
    let host = host_part.split(':').next().unwrap_or(host_part);
    if host.is_empty() {
        None
    } else {
        Some(host.to_lowercase())
    }
}

pub fn inject_websocket(ctx: &Ctx, permissions: Arc<PermissionState>, pool: WsPool) -> Result<()> {
    let globals = ctx.globals();

    let next_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));

    // __wsConnect(url) -> id | throws
    let perms = permissions.clone();
    let pool2 = pool.clone();
    let nid = next_id.clone();
    globals.set(
        "__wsConnect",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<u32> {
                let url = args
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__wsConnect() requires a URL".into()))?;
                let host = host_from_url(&url)
                    .ok_or_else(|| js_err(&ctx, format!("Invalid WebSocket URL: {}", url)))?;
                if !perms.check(&Capability::Network(host.clone())) {
                    return Err(js_err(
                        &ctx,
                        format!("Network access denied. Run with --allow-net={}", host),
                    ));
                }
                let (ws, _) = connect(&url)
                    .map_err(|e| js_err(&ctx, format!("WebSocket connect failed: {}", e)))?;
                let id = {
                    let mut n = nid.lock().unwrap();
                    let id = *n;
                    *n = n.wrapping_add(1);
                    id
                };
                pool2.lock().unwrap().insert(id, ws);
                Ok(id)
            },
        ),
    )?;

    // __wsSend(id, data) -> undefined | throws
    let pool2 = pool.clone();
    globals.set(
        "__wsSend",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<()> {
                let mut it = args.0.into_iter();
                let id: u32 = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__wsSend() requires id".into()))?
                    .parse()
                    .map_err(|_| js_err(&ctx, "invalid id".into()))?;
                let data = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__wsSend() requires message".into()))?;
                let mut guard = pool2.lock().unwrap();
                let ws = guard
                    .get_mut(&id)
                    .ok_or_else(|| js_err(&ctx, format!("No WS {}", id)))?;
                ws.send(Message::Text(data.as_str().into()))
                    .map_err(|e| js_err(&ctx, format!("send failed: {}", e)))
            },
        ),
    )?;

    // __wsRecv(id) -> string | "@@CLOSED" (blocking)
    let pool2 = pool.clone();
    globals.set(
        "__wsRecv",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<String> {
                let id: u32 = args
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__wsRecv() requires id".into()))?
                    .parse()
                    .map_err(|_| js_err(&ctx, "invalid id".into()))?;
                let mut guard = pool2.lock().unwrap();
                let ws = guard
                    .get_mut(&id)
                    .ok_or_else(|| js_err(&ctx, format!("No WS {}", id)))?;
                loop {
                    match ws.read() {
                        Ok(Message::Text(t)) => return Ok(t.to_string()),
                        Ok(Message::Binary(b)) => {
                            return Ok(String::from_utf8_lossy(&b).into_owned());
                        }
                        Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {
                            continue;
                        }
                        Ok(Message::Close(_)) => {
                            drop(guard);
                            pool2.lock().unwrap().remove(&id);
                            return Ok("@@CLOSED".into());
                        }
                        Err(e) => {
                            drop(guard);
                            pool2.lock().unwrap().remove(&id);
                            return Err(js_err(&ctx, format!("recv failed: {}", e)));
                        }
                    }
                }
            },
        ),
    )?;

    // __wsClose(id) -> undefined
    let pool2 = pool.clone();
    globals.set(
        "__wsClose",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<()> {
                let id: u32 = args
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__wsClose() requires id".into()))?
                    .parse()
                    .map_err(|_| js_err(&ctx, "invalid id".into()))?;
                let mut guard = pool2.lock().unwrap();
                if let Some(mut ws) = guard.remove(&id) {
                    let _ = ws.close(None);
                }
                Ok(())
            },
        ),
    )?;

    ctx.eval::<(), _>(WS_JS_SHIM)?;
    Ok(())
}

const WS_JS_SHIM: &str = r#"
(function() {
  var CONNECTING = 0, OPEN = 1, CLOSING = 2, CLOSED = 3;

  function WebSocket(url) {
    this.url = url;
    this.readyState = CONNECTING;
    this.onopen    = null;
    this.onmessage = null;
    this.onerror   = null;
    this.onclose   = null;
    this._id = null;
    try {
      this._id = __wsConnect(url);
      this.readyState = OPEN;
      if (typeof this.onopen === 'function') this.onopen({ target: this });
    } catch (e) {
      this.readyState = CLOSED;
      if (typeof this.onerror === 'function') this.onerror({ target: this, error: e });
    }
  }

  WebSocket.prototype.send = function(data) {
    if (this.readyState !== OPEN) throw new Error('WebSocket is not open');
    __wsSend(String(this._id), String(data));
  };

  // recv() — blocking: returns next message string, or null if connection closed.
  WebSocket.prototype.recv = function() {
    if (this.readyState !== OPEN) return null;
    var msg = __wsRecv(String(this._id));
    if (msg === '@@CLOSED') {
      this.readyState = CLOSED;
      if (typeof this.onclose === 'function') this.onclose({ target: this });
      return null;
    }
    if (typeof this.onmessage === 'function') this.onmessage({ target: this, data: msg });
    return msg;
  };

  WebSocket.prototype.close = function() {
    if (this._id !== null) {
      __wsClose(String(this._id));
      this._id = null;
    }
    this.readyState = CLOSED;
    if (typeof this.onclose === 'function') this.onclose({ target: this });
  };

  WebSocket.CONNECTING = CONNECTING;
  WebSocket.OPEN       = OPEN;
  WebSocket.CLOSING    = CLOSING;
  WebSocket.CLOSED     = CLOSED;

  globalThis.WebSocket = WebSocket;
})();
"#;
