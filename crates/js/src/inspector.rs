//! Chrome DevTools Protocol (CDP) inspector over WebSocket (port 9229 by default).
//!
//! When `--inspect[=host:port]` is passed, a WebSocket server is started before
//! the script runs. The `debugger;` statement is rewritten to `__3va_debugger__();`
//! at source-load time. That injected Rust function pauses JS execution via
//! `block_in_place` and emits a `Debugger.paused` CDP event to all connected clients.
//!
//! Implemented CDP subset:
//! - `Runtime.enable` / `Debugger.enable`
//! - `Debugger.paused` event (reason: `debugCommand`)
//! - `Debugger.resumed` event
//! - `Debugger.resume` request
//! - `disconnect`

use std::net::SocketAddr;
use std::sync::{Arc, Condvar, Mutex};

use tungstenite::Message;

/// Shared state between the CDP server and the `__3va_debugger__` injected function.
pub struct InspectorState {
    /// While `true`, JS execution is paused waiting for a `Debugger.resume` request.
    paused: Mutex<bool>,
    resume_cv: Condvar,
    /// Senders for active WebSocket connections. Each send pushes a JSON message.
    clients: Mutex<Vec<std::sync::mpsc::SyncSender<String>>>,
}

impl InspectorState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            paused: Mutex::new(false),
            resume_cv: Condvar::new(),
            clients: Mutex::new(Vec::new()),
        })
    }

    fn broadcast(&self, msg: &str) {
        let clients = self.clients.lock().unwrap();
        for tx in clients.iter() {
            let _ = tx.send(msg.to_string());
        }
    }

    /// Called by `__3va_debugger__` — pauses execution and sends `Debugger.paused`.
    pub fn pause(&self) {
        {
            let mut p = self.paused.lock().unwrap();
            *p = true;
        }
        let paused_event = r#"{"method":"Debugger.paused","params":{"callFrames":[],"reason":"debugCommand","hitBreakpoints":[]}}"#;
        self.broadcast(paused_event);

        // Block until a client sends Debugger.resume.
        let mut p = self.paused.lock().unwrap();
        while *p {
            p = self.resume_cv.wait(p).unwrap();
        }

        let resumed_event = r#"{"method":"Debugger.resumed","params":{}}"#;
        self.broadcast(resumed_event);
    }

    fn resume(&self) {
        let mut p = self.paused.lock().unwrap();
        *p = false;
        self.resume_cv.notify_all();
    }
}

/// Start the CDP WebSocket server and return the `InspectorState`.
///
/// Spawns a Tokio task; connects clients are handled on separate threads via
/// `std::net::TcpListener` (blocking) so WebSocket framing doesn't block the
/// async runtime.
pub fn start(addr: SocketAddr) -> Arc<InspectorState> {
    let state = InspectorState::new();
    let state2 = state.clone();

    std::thread::spawn(move || {
        let listener = match std::net::TcpListener::bind(addr) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[inspector] Failed to bind {addr}: {e}");
                return;
            }
        };
        eprintln!(
            "[inspector] CDP WebSocket server listening on ws://{addr}\n\
             [inspector] Open Chrome and navigate to: chrome://inspect"
        );

        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let state = state2.clone();
            std::thread::spawn(move || handle_client(stream, state));
        }
    });

    state
}

fn handle_client(stream: std::net::TcpStream, state: Arc<InspectorState>) {
    let mut ws = match tungstenite::accept(stream) {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("[inspector] WebSocket handshake failed: {e}");
            return;
        }
    };

    // Register an outbound channel so `broadcast` can reach this client.
    let (tx, rx) = std::sync::mpsc::sync_channel::<String>(64);
    {
        state.clients.lock().unwrap().push(tx);
    }

    // Send `Runtime.executionContextCreated` so Chrome knows we're alive.
    let _ = ws.send(Message::Text(
        r#"{"method":"Runtime.executionContextCreated","params":{"context":{"id":1,"origin":"","name":"3va","uniqueId":"1"}}}"#.to_string(),
    ));

    ws.get_ref()
        .set_read_timeout(Some(std::time::Duration::from_millis(50)))
        .ok();

    loop {
        // Drain any outbound messages first.
        while let Ok(msg) = rx.try_recv() {
            if ws.send(Message::Text(msg)).is_err() {
                break;
            }
        }

        match ws.read() {
            Ok(Message::Text(text)) => {
                handle_message(&text, &state, &mut ws);
            }
            Ok(Message::Close(_)) | Err(tungstenite::Error::ConnectionClosed) => break,
            Err(tungstenite::Error::Io(e))
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                // timeout on non-blocking read — loop back and drain outbound
            }
            Err(_) => break,
            _ => {}
        }
    }

    // Remove client on disconnect.
    let mut clients = state.clients.lock().unwrap();
    clients.retain(|c| c.send(String::new()).is_ok());
}

fn handle_message(
    text: &str,
    state: &InspectorState,
    ws: &mut tungstenite::WebSocket<std::net::TcpStream>,
) {
    let Ok(msg) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let id = msg.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
    let method = msg
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or_default();

    let result = match method {
        "Runtime.enable" | "Profiler.enable" | "HeapProfiler.enable" | "Log.enable" => {
            format!(r#"{{"id":{id},"result":{{}}}}"#)
        }
        "Debugger.enable" => {
            format!(r#"{{"id":{id},"result":{{"debuggerId":"3va-debugger-1"}}}}"#)
        }
        "Debugger.resume" => {
            state.resume();
            format!(r#"{{"id":{id},"result":{{}}}}"#)
        }
        "Debugger.setPauseOnExceptions" | "Debugger.setAsyncCallStackDepth" => {
            format!(r#"{{"id":{id},"result":{{}}}}"#)
        }
        "Runtime.getIsolateId" => {
            format!(r#"{{"id":{id},"result":{{"id":"3va-isolate-1"}}}}"#)
        }
        "Runtime.runIfWaitingForDebugger" => {
            format!(r#"{{"id":{id},"result":{{}}}}"#)
        }
        _ => format!(r#"{{"id":{id},"result":{{}}}}"#),
    };

    let _ = ws.send(Message::Text(result));
}

/// Replace standalone `debugger;` statements with `__3va_debugger__();`.
/// Only matches the keyword on its own line (with optional whitespace), so it
/// won't mangle strings or comments that happen to contain "debugger".
pub fn rewrite_debugger_statements(source: &str) -> std::borrow::Cow<'_, str> {
    // Fast path: avoid regex overhead when there's nothing to replace.
    if !source.contains("debugger") {
        return std::borrow::Cow::Borrowed(source);
    }
    // Line-by-line transform: replace `debugger;` (possibly `debugger` without `;`)
    // that appears as its own statement, preserving indentation.
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed == "debugger;" || trimmed == "debugger" {
            let indent_len = line.len() - line.trim_start().len();
            out.push_str(&line[..indent_len]);
            out.push_str("if (typeof __3va_debugger__ === 'function') __3va_debugger__();\n");
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    // Trim the trailing newline we always add if the original didn't have one.
    if !source.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    std::borrow::Cow::Owned(out)
}
