use std::collections::HashMap;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket as TungsteniteWs, connect};
use v8::{Function, FunctionCallbackArguments, PinScope, ReturnValue, Script};
use vvva_permissions::{Capability, PermissionState};

type WsConn = TungsteniteWs<MaybeTlsStream<TcpStream>>;

pub type WsPool = Arc<Mutex<HashMap<u32, WsConn>>>;

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

fn throw_js_error(scope: &mut PinScope, msg: String) {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!("new Error(\"{}\")", escaped);
    if let Some(source) = v8::String::new(scope, &script)
        && let Some(compiled) = Script::compile(scope, source, None)
        && let Some(result) = compiled.run(scope)
    {
        scope.throw_exception(result);
    }
}

// Thread-local, not a process-wide static — see the identical fix (and
// rationale) in fs.rs's FS_PERMISSIONS: a `OnceLock` here only keeps the
// *first* engine's permissions ever created in the process, so every later
// `JsEngine` (every other test, or a second engine in a long-lived process)
// silently inherits the first one's grants instead of its own.
thread_local! {
    static WS_PERMISSIONS: std::cell::RefCell<Option<Arc<PermissionState>>> =
        const { std::cell::RefCell::new(None) };
}
fn perms() -> Arc<PermissionState> {
    WS_PERMISSIONS.with(|p| {
        p.borrow()
            .clone()
            .expect("inject_websocket not called on this thread")
    })
}
static WS_POOL: std::sync::OnceLock<WsPool> = std::sync::OnceLock::new();
fn pool2() -> &'static WsPool {
    WS_POOL.get().unwrap()
}
static WS_NEXT_ID: std::sync::OnceLock<Arc<Mutex<u32>>> = std::sync::OnceLock::new();
fn nid() -> &'static Arc<Mutex<u32>> {
    WS_NEXT_ID.get().unwrap()
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

pub fn inject_websocket(
    scope: &mut PinScope,
    permissions_param: Arc<PermissionState>,
    pool_param: WsPool,
) -> anyhow::Result<()> {
    WS_PERMISSIONS.with(|p| *p.borrow_mut() = Some(permissions_param));
    WS_POOL.set(pool_param).ok();
    WS_NEXT_ID.set(Arc::new(Mutex::new(0))).ok();
    let context = scope.get_current_context();

    // __wsConnect(url) -> id | throws
    {
        let native_fn = Function::new(
            scope,
            move |scope: &mut PinScope<'_, '_>,
                  args: FunctionCallbackArguments,
                  mut rv: ReturnValue| {
                let url = args.get(0).to_rust_string_lossy(scope);

                let host = match host_from_url(&url) {
                    Some(h) => h,
                    None => {
                        throw_js_error(scope, format!("Invalid WebSocket URL: {}", url));
                        return;
                    }
                };

                if !perms().check(&Capability::Network(host.clone())) {
                    throw_js_error(
                        scope,
                        format!("Network access denied. Run with --allow-net={}", host),
                    );
                    return;
                }

                let ws = match connect(&url) {
                    Ok((ws, _)) => ws,
                    Err(e) => {
                        throw_js_error(scope, format!("WebSocket connect failed: {}", e));
                        return;
                    }
                };

                let id = {
                    let mut n = nid().lock().unwrap();
                    let id = *n;
                    *n = n.wrapping_add(1);
                    id
                };

                pool2().lock().unwrap().insert(id, ws);
                rv.set(v8::Integer::new_from_unsigned(scope, id).into());
            },
        )
        .unwrap();

        let global = context.global(scope);
        global.set(
            scope,
            v8::String::new(scope, "__wsConnect").unwrap().into(),
            native_fn.into(),
        );
    }

    // __wsSend(id, data) -> undefined | throws
    {
        let native_fn = Function::new(
            scope,
            move |scope: &mut PinScope<'_, '_>,
                  args: FunctionCallbackArguments,
                  mut _rv: ReturnValue| {
                let id_val = args.get(0);
                let data = args.get(1).to_rust_string_lossy(scope);

                let id = match id_val.uint32_value(scope) {
                    Some(id) => id,
                    None => {
                        throw_js_error(scope, "__wsSend() requires id".to_string());
                        return;
                    }
                };

                let mut guard = pool2().lock().unwrap();
                let ws = match guard.get_mut(&id) {
                    Some(ws) => ws,
                    None => {
                        throw_js_error(scope, format!("No WS {}", id));
                        return;
                    }
                };

                if let Err(e) = ws.send(Message::Text(data)) {
                    throw_js_error(scope, format!("send failed: {}", e));
                }
            },
        )
        .unwrap();

        let global = context.global(scope);
        global.set(
            scope,
            v8::String::new(scope, "__wsSend").unwrap().into(),
            native_fn.into(),
        );
    }

    // __wsRecv(id) -> string | "@@CLOSED" (blocking)
    {
        let native_fn = Function::new(
            scope,
            move |scope: &mut PinScope<'_, '_>,
                  args: FunctionCallbackArguments,
                  mut rv: ReturnValue| {
                let id_val = args.get(0);

                let id = match id_val.uint32_value(scope) {
                    Some(id) => id,
                    None => {
                        throw_js_error(scope, "__wsRecv() requires id".to_string());
                        return;
                    }
                };

                let mut guard = pool2().lock().unwrap();
                let ws = match guard.get_mut(&id) {
                    Some(ws) => ws,
                    None => {
                        throw_js_error(scope, format!("No WS {}", id));
                        return;
                    }
                };

                loop {
                    match ws.read() {
                        Ok(Message::Text(t)) => {
                            rv.set(v8::String::new(scope, &t.to_string()).unwrap().into());
                            return;
                        }
                        Ok(Message::Binary(b)) => {
                            rv.set(
                                v8::String::new(scope, &String::from_utf8_lossy(&b))
                                    .unwrap()
                                    .into(),
                            );
                            return;
                        }
                        Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {
                            continue;
                        }
                        Ok(Message::Close(_)) => {
                            pool2().lock().unwrap().remove(&id);
                            rv.set(v8::String::new(scope, "@@CLOSED").unwrap().into());
                            return;
                        }
                        Err(e) => {
                            pool2().lock().unwrap().remove(&id);
                            throw_js_error(scope, format!("recv failed: {}", e));
                            return;
                        }
                    }
                }
            },
        )
        .unwrap();

        let global = context.global(scope);
        global.set(
            scope,
            v8::String::new(scope, "__wsRecv").unwrap().into(),
            native_fn.into(),
        );
    }

    // __wsClose(id) -> undefined
    {
        let native_fn = Function::new(
            scope,
            move |scope: &mut PinScope<'_, '_>,
                  args: FunctionCallbackArguments,
                  mut _rv: ReturnValue| {
                let id_val = args.get(0);

                let id = match id_val.uint32_value(scope) {
                    Some(id) => id,
                    None => {
                        throw_js_error(scope, "__wsClose() requires id".to_string());
                        return;
                    }
                };

                let mut guard = pool2().lock().unwrap();
                if let Some(mut ws) = guard.remove(&id) {
                    let _ = ws.close(None);
                }
            },
        )
        .unwrap();

        let global = context.global(scope);
        global.set(
            scope,
            v8::String::new(scope, "__wsClose").unwrap().into(),
            native_fn.into(),
        );
    }

    let js_code = r#"
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

    let source = v8::String::new(scope, js_code).unwrap();
    let script = Script::compile(scope, source, None).unwrap();
    let _ = script.run(scope);

    Ok(())
}
