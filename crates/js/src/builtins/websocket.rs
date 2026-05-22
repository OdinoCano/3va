use rquickjs::{Ctx, Function, Result, function::Rest};
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::TcpStream;
use std::rc::Rc;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket as TungsteniteWs, connect};
use vvva_permissions::{Capability, PermissionState};

type WsConn = TungsteniteWs<MaybeTlsStream<TcpStream>>;

thread_local! {
    static WS_POOL: RefCell<HashMap<u32, WsConn>> = RefCell::new(HashMap::new());
    static WS_NEXT_ID: RefCell<u32> = const { RefCell::new(0) };
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

pub fn inject_websocket(ctx: &Ctx, permissions: Rc<RefCell<PermissionState>>) -> Result<()> {
    let globals = ctx.globals();

    // __wsConnect(url) -> id | throws
    let perms = permissions.clone();
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
                if !perms.borrow().check(&Capability::Network(host.clone())) {
                    return Err(js_err(
                        &ctx,
                        format!("Network access denied. Run with --allow-net={}", host),
                    ));
                }
                let (ws, _) = connect(&url)
                    .map_err(|e| js_err(&ctx, format!("WebSocket connect failed: {}", e)))?;
                let id = WS_NEXT_ID.with(|n| {
                    let mut n = n.borrow_mut();
                    let id = *n;
                    *n = n.wrapping_add(1);
                    id
                });
                WS_POOL.with(|pool| pool.borrow_mut().insert(id, ws));
                Ok(id)
            },
        ),
    )?;

    // __wsSend(id, data) -> undefined | throws
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
                WS_POOL.with(|pool| {
                    let mut pool = pool.borrow_mut();
                    let ws = pool
                        .get_mut(&id)
                        .ok_or_else(|| js_err(&ctx, format!("No WS {}", id)))?;
                    ws.send(Message::Text(data.as_str().into()))
                        .map_err(|e| js_err(&ctx, format!("send failed: {}", e)))
                })
            },
        ),
    )?;

    // __wsRecv(id) -> string | "@@CLOSED" (blocking)
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
                WS_POOL.with(|pool| {
                    let mut pool = pool.borrow_mut();
                    let ws = pool
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
                                pool.remove(&id);
                                return Ok("@@CLOSED".into());
                            }
                            Err(e) => {
                                pool.remove(&id);
                                return Err(js_err(&ctx, format!("recv failed: {}", e)));
                            }
                        }
                    }
                })
            },
        ),
    )?;

    // __wsClose(id) -> undefined
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
                WS_POOL.with(|pool| {
                    let mut pool = pool.borrow_mut();
                    if let Some(mut ws) = pool.remove(&id) {
                        let _ = ws.close(None);
                    }
                });
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
