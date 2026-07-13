//! IMAP4rev1 client backend.
//!
//! Architecture:
//!   - Connections held in pool: `Arc<Mutex<HashMap<u32, ImapConnection>>>`
//!   - Streams for fetch operations: `Arc<Mutex<HashMap<u32, FetchStream>>>`

use base64::Engine;
use native_tls::TlsStream;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use vvva_permissions::{Capability, PermissionState};

const CRLF: &[u8] = b"\r\n";

enum ImapConn {
    Plain(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl ImapConn {
    fn read_byte(&mut self) -> io::Result<Option<u8>> {
        let mut buf = [0u8; 1];
        match self {
            ImapConn::Plain(s) => match s.read(&mut buf) {
                Ok(0) => Ok(None),
                Ok(_) => Ok(Some(buf[0])),
                Err(e) => Err(e),
            },
            ImapConn::Tls(s) => match s.read(&mut buf) {
                Ok(0) => Ok(None),
                Ok(_) => Ok(Some(buf[0])),
                Err(e) => Err(e),
            },
        }
    }

    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        match self {
            ImapConn::Plain(s) => s.write_all(data),
            ImapConn::Tls(s) => s.write_all(data),
        }
    }

    fn shutdown(&mut self) {
        match self {
            ImapConn::Plain(s) => {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
            ImapConn::Tls(s) => {
                let _ = s.shutdown();
            }
        }
    }
}

struct ImapConnection {
    conn: ImapConn,
    tag: u32,
    state: ImapState,
    capabilities: Vec<String>,
    authenticated: bool,
    selected_mailbox: Option<String>,
    read_only: bool,
}

#[derive(Clone, Debug, PartialEq)]
#[allow(dead_code)]
enum ImapState {
    NotAuthenticated,
    Authenticated,
    Selected,
    Logout,
}

impl ImapConnection {
    fn new(conn: ImapConn) -> Self {
        Self {
            conn,
            tag: 0,
            state: ImapState::NotAuthenticated,
            capabilities: Vec::new(),
            authenticated: false,
            selected_mailbox: None,
            read_only: false,
        }
    }

    fn next_tag(&mut self) -> String {
        self.tag += 1;
        format!("A{:04}", self.tag)
    }

    fn read_line(&mut self) -> io::Result<Vec<u8>> {
        let mut buf = Vec::new();
        loop {
            match self.conn.read_byte() {
                Ok(None) => break,
                Ok(Some(b'\n')) => break,
                Ok(Some(b'\r')) => {}
                Ok(Some(b)) => buf.push(b),
                Err(e) => return Err(e),
            }
        }
        Ok(buf)
    }
}

struct FetchStream {
    messages: Vec<FetchMessage>,
    current: usize,
}

#[derive(Debug, Clone)]
struct FetchMessage {
    seq: u32,
    attributes: HashMap<String, String>,
    bodies: HashMap<String, Vec<u8>>,
}

impl FetchStream {
    fn new() -> Self {
        Self {
            messages: Vec::new(),
            current: 0,
        }
    }

    fn add_message(&mut self, msg: FetchMessage) {
        self.messages.push(msg);
    }

    fn next(&mut self) -> Option<FetchMessage> {
        if self.current < self.messages.len() {
            let msg = self.messages[self.current].clone();
            self.current += 1;
            Some(msg)
        } else {
            None
        }
    }
}

struct ImapStateInner {
    connections: Arc<Mutex<HashMap<u32, ImapConnection>>>,
    fetch_streams: Arc<Mutex<HashMap<u32, FetchStream>>>,
    next_id: Arc<Mutex<u32>>,
    next_stream_id: Arc<Mutex<u32>>,
}

impl ImapStateInner {
    fn alloc_id(&self) -> u32 {
        let mut n = self.next_id.lock().unwrap();
        let id = *n;
        *n = n.wrapping_add(1);
        id
    }

    fn alloc_stream_id(&self) -> u32 {
        let mut n = self.next_stream_id.lock().unwrap();
        let id = *n;
        *n = n.wrapping_add(1);
        id
    }
}

fn parse_mailbox_list(response: &[String]) -> Vec<String> {
    let mut mailboxes = Vec::new();
    for line in response {
        let line = line.trim();
        if line.starts_with("* LIST ")
            && let Some(start) = line.find('(')
            && let Some(end) = line.rfind(')')
        {
            let _flags = &line[start..=end];
            let rest = line[end + 1..].trim();
            if let Some(rest) = rest.strip_prefix('"') {
                if let Some(end_quote) = rest.find('"') {
                    let _delimiter = &rest[1..=end_quote];
                    let mailbox = rest[end_quote + 2..].trim().to_string();
                    if !mailbox.is_empty() {
                        mailboxes.push(mailbox);
                    }
                }
            } else {
                let mailbox = rest.trim().to_string();
                if !mailbox.is_empty() {
                    mailboxes.push(mailbox);
                }
            }
        }
    }
    mailboxes
}

fn base64_encode(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

// Thread-local, not a process-wide static — see the identical fix (and
// rationale) in fs.rs's FS_PERMISSIONS: a `OnceLock` here only keeps the
// *first* engine's permissions ever created in the process, so every later
// `JsEngine` (every other test, or a second engine in a long-lived process)
// silently inherits the first one's grants instead of its own.
thread_local! {
    static IMAP_PERMISSIONS: std::cell::RefCell<Option<Arc<PermissionState>>> =
        const { std::cell::RefCell::new(None) };
}
fn perms() -> Arc<PermissionState> {
    IMAP_PERMISSIONS.with(|p| {
        p.borrow()
            .clone()
            .expect("inject_imap not called on this thread")
    })
}
static IMAP_INNER: std::sync::OnceLock<Arc<ImapStateInner>> = std::sync::OnceLock::new();
fn inner() -> &'static Arc<ImapStateInner> {
    IMAP_INNER.get().unwrap()
}

pub fn inject_imap(
    scope: &mut v8::ContextScope<v8::HandleScope>,
    permissions: Arc<PermissionState>,
) {
    IMAP_PERMISSIONS.with(|p| *p.borrow_mut() = Some(permissions));
    IMAP_INNER
        .set(Arc::new(ImapStateInner {
            connections: Arc::new(Mutex::new(HashMap::new())),
            fetch_streams: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(Mutex::new(0)),
            next_stream_id: Arc::new(Mutex::new(0)),
        }))
        .ok();

    let context = scope.get_current_context();
    let global = context.global(scope);

    let create_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              _args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let id = inner().alloc_id();
            rv.set(v8::Number::new(_scope, id as f64).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapCreate").unwrap().into(),
        create_fn.into(),
    );

    let connect_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let host = args.get(1).to_rust_string_lossy(_scope);
            let port = args.get(2).uint32_value(_scope).unwrap_or(143) as u16;
            let use_tls = args.get(3).boolean_value(_scope);

            let perms = perms().clone();
            let inner = inner().clone();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let result: Result<(), String> = (|| {
                    if !perms.check(&Capability::Network(host.clone())) {
                        return Err(format!(
                            "Network access denied. Run with --allow-net={}",
                            host
                        ));
                    }

                    let connect_host = host.clone();
                    let stream = rt.block_on(async {
                        tokio::task::spawn_blocking(move || {
                            TcpStream::connect(format!("{}:{}", connect_host, port))
                        })
                        .await
                        .map_err(|e| format!("Connection failed: {}", e))?
                        .map_err(|e: std::io::Error| e.to_string())
                    })?;

                    stream
                        .set_nonblocking(true)
                        .map_err(|e| format!("Set nonblocking failed: {}", e))?;

                    let conn = if use_tls {
                        let connector = native_tls::TlsConnector::new()
                            .map_err(|e| format!("TLS init failed: {}", e))?;
                        let tls_stream = connector
                            .connect(&host, stream)
                            .map_err(|e| format!("TLS handshake failed: {}", e))?;
                        ImapConn::Tls(tls_stream)
                    } else {
                        ImapConn::Plain(stream)
                    };

                    let mut imap_conn = ImapConnection::new(conn);
                    let tag = imap_conn.next_tag();
                    imap_conn
                        .conn
                        .write_all(format!("{} CAPABILITY\r\n", tag).as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;

                    loop {
                        let buf = imap_conn
                            .read_line()
                            .map_err(|e| format!("Read failed: {}", e))?;
                        if buf.starts_with(tag.as_bytes()) {
                            break;
                        }
                        if buf.starts_with(b"* CAPABILITY ") {
                            let caps = String::from_utf8_lossy(&buf[14..])
                                .split_whitespace()
                                .map(|s| s.to_string())
                                .collect();
                            imap_conn.capabilities = caps;
                        }
                    }

                    imap_conn.state = ImapState::NotAuthenticated;
                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP connect error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapConnect").unwrap().into(),
        connect_fn.into(),
    );

    let login_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let username = args.get(1).to_rust_string_lossy(_scope);
            let password = args.get(2).to_rust_string_lossy(_scope);

            let _perms = perms().clone();
            let inner = inner().clone();

            std::thread::spawn(move || {
                let _rt = tokio::runtime::Runtime::new().unwrap();
                let result: Result<(), String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} LOGIN \"{}\" \"{}\"", tag, username, password);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let buf = imap_conn
                        .read_line()
                        .map_err(|e| format!("Read failed: {}", e))?;
                    let response = String::from_utf8_lossy(&buf).to_string();

                    if response.contains("OK") {
                        imap_conn.authenticated = true;
                        imap_conn.state = ImapState::Authenticated;
                        inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                        Ok(())
                    } else {
                        Err(format!("Login failed: {}", response.trim()))
                    }
                })();

                if let Err(e) = result {
                    eprintln!("IMAP login error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapLogin").unwrap().into(),
        login_fn.into(),
    );

    let capability_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);

            let guard = inner().connections.lock().unwrap();
            if let Some(imap_conn) = guard.get(&imap_id) {
                let caps = imap_conn.capabilities.join(" ");
                rv.set(v8::String::new(_scope, &caps).unwrap().into());
            } else {
                rv.set(v8::undefined(_scope).into());
            }
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapCapability").unwrap().into(),
        capability_fn.into(),
    );

    let list_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let reference = args.get(1).to_rust_string_lossy(_scope);
            let mailbox = args.get(2).to_rust_string_lossy(_scope);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let _rt = tokio::runtime::Runtime::new().unwrap();
                let result: Result<String, String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} LIST \"{}\" \"{}\"", tag, reference, mailbox);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let mut response_lines = Vec::new();
                    loop {
                        let buf = imap_conn
                            .read_line()
                            .map_err(|e| format!("Read failed: {}", e))?;
                        if buf.starts_with(tag.as_bytes()) {
                            break;
                        }
                        response_lines.push(String::from_utf8_lossy(&buf).to_string());
                    }

                    let mailboxes = parse_mailbox_list(&response_lines);
                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(serde_json::json!(mailboxes).to_string())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP list error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapListMailboxes")
            .unwrap()
            .into(),
        list_fn.into(),
    );

    let status_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let mailbox = args.get(1).to_rust_string_lossy(_scope);
            let items = args.get(2).to_rust_string_lossy(_scope);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let _rt = tokio::runtime::Runtime::new().unwrap();
                let result: Result<String, String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} STATUS \"{}\" ({})", tag, mailbox, items);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let mut status_data = HashMap::new();
                    loop {
                        let buf = imap_conn
                            .read_line()
                            .map_err(|e| format!("Read failed: {}", e))?;
                        if buf.starts_with(tag.as_bytes()) {
                            break;
                        }
                        let line = String::from_utf8_lossy(&buf).to_string();
                        if line.contains("STATUS ") {
                            let parts: Vec<&str> = line.split_whitespace().collect();
                            for (i, part) in parts.iter().enumerate() {
                                match *part {
                                    "MESSAGES" if i + 1 < parts.len() => {
                                        status_data.insert(
                                            "messages".to_string(),
                                            parts[i + 1].to_string(),
                                        );
                                    }
                                    "RECENT" if i + 1 < parts.len() => {
                                        status_data
                                            .insert("recent".to_string(), parts[i + 1].to_string());
                                    }
                                    "UNSEEN" if i + 1 < parts.len() => {
                                        status_data
                                            .insert("unseen".to_string(), parts[i + 1].to_string());
                                    }
                                    "UIDNEXT" if i + 1 < parts.len() => {
                                        status_data.insert(
                                            "uidnext".to_string(),
                                            parts[i + 1].to_string(),
                                        );
                                    }
                                    "UIDVALIDITY" if i + 1 < parts.len() => {
                                        status_data.insert(
                                            "uidvalidity".to_string(),
                                            parts[i + 1].to_string(),
                                        );
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(serde_json::json!(status_data).to_string())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP status error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapStatus").unwrap().into(),
        status_fn.into(),
    );

    let select_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let mailbox = args.get(1).to_rust_string_lossy(_scope);
            let readonly = args.get(2).boolean_value(_scope);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let _rt = tokio::runtime::Runtime::new().unwrap();
                let result: Result<String, String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = if readonly {
                        format!("{} EXAMINE \"{}\"", tag, mailbox)
                    } else {
                        format!("{} SELECT \"{}\"", tag, mailbox)
                    };
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let mut select_data = HashMap::new();
                    loop {
                        let buf = imap_conn
                            .read_line()
                            .map_err(|e| format!("Read failed: {}", e))?;
                        if buf.starts_with(tag.as_bytes()) {
                            break;
                        }
                        let line = String::from_utf8_lossy(&buf).to_string();
                        if line.starts_with("* ") {
                            let parts: Vec<&str> = line.split_whitespace().collect();
                            if parts.len() >= 3 {
                                if parts[2] == "EXISTS" {
                                    select_data.insert("exists".to_string(), parts[1].to_string());
                                } else if parts[2] == "RECENT" {
                                    select_data.insert("recent".to_string(), parts[1].to_string());
                                }
                            }
                            if line.contains("UIDVALIDITY") {
                                for (i, part) in parts.iter().enumerate() {
                                    if *part == "UIDVALIDITY" && i + 1 < parts.len() {
                                        select_data.insert(
                                            "uidvalidity".to_string(),
                                            parts[i + 1].to_string(),
                                        );
                                    }
                                }
                            }
                            if line.contains("UIDNEXT") {
                                for (i, part) in parts.iter().enumerate() {
                                    if *part == "UIDNEXT" && i + 1 < parts.len() {
                                        select_data.insert(
                                            "uidnext".to_string(),
                                            parts[i + 1].to_string(),
                                        );
                                    }
                                }
                            }
                        }
                    }

                    imap_conn.selected_mailbox = Some(mailbox);
                    imap_conn.read_only = readonly;
                    imap_conn.state = ImapState::Selected;
                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(serde_json::json!(select_data).to_string())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP select error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapSelect").unwrap().into(),
        select_fn.into(),
    );

    let create_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let mailbox = args.get(1).to_rust_string_lossy(_scope);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let result: Result<(), String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} CREATE \"{}\"", tag, mailbox);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let _buf = imap_conn
                        .read_line()
                        .map_err(|e| format!("Read failed: {}", e))?;
                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP create error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapCreateMailbox")
            .unwrap()
            .into(),
        create_fn.into(),
    );

    let delete_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let mailbox = args.get(1).to_rust_string_lossy(_scope);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let result: Result<(), String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} DELETE \"{}\"", tag, mailbox);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let _buf = imap_conn
                        .read_line()
                        .map_err(|e| format!("Read failed: {}", e))?;
                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP delete error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapDeleteMailbox")
            .unwrap()
            .into(),
        delete_fn.into(),
    );

    let rename_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let old_mailbox = args.get(1).to_rust_string_lossy(_scope);
            let new_mailbox = args.get(2).to_rust_string_lossy(_scope);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let result: Result<(), String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} RENAME \"{}\" \"{}\"", tag, old_mailbox, new_mailbox);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let _buf = imap_conn
                        .read_line()
                        .map_err(|e| format!("Read failed: {}", e))?;
                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP rename error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapRenameMailbox")
            .unwrap()
            .into(),
        rename_fn.into(),
    );

    let subscribe_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let mailbox = args.get(1).to_rust_string_lossy(_scope);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let result: Result<(), String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} SUBSCRIBE \"{}\"", tag, mailbox);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let _buf = imap_conn
                        .read_line()
                        .map_err(|e| format!("Read failed: {}", e))?;
                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP subscribe error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapSubscribe").unwrap().into(),
        subscribe_fn.into(),
    );

    let fetch_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let sequence = args.get(1).to_rust_string_lossy(_scope);
            let bodies = args.get(2).to_rust_string_lossy(_scope);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let _rt = tokio::runtime::Runtime::new().unwrap();
                let result: Result<u32, String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let stream_id = inner.alloc_stream_id();
                    let mut fetch_stream = FetchStream::new();

                    let tag = imap_conn.next_tag();
                    let fetch_items = if bodies.is_empty() || bodies == "''" {
                        "BODY[]".to_string()
                    } else if bodies == "HEADER" {
                        "BODY[HEADER]".to_string()
                    } else if bodies == "TEXT" {
                        "BODY[TEXT]".to_string()
                    } else if bodies == "FULL" {
                        "BODY[]".to_string()
                    } else {
                        format!("BODY[{}]", bodies)
                    };
                    let cmd = format!("{} FETCH {} ({})", tag, sequence, fetch_items);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let mut current_msg = FetchMessage {
                        seq: 0,
                        attributes: HashMap::new(),
                        bodies: HashMap::new(),
                    };
                    let mut in_literal = false;
                    let mut literal_size = 0;
                    let mut literal_body = Vec::new();

                    loop {
                        let buf = imap_conn
                            .read_line()
                            .map_err(|e| format!("Read failed: {}", e))?;

                        if buf.starts_with(tag.as_bytes()) {
                            if current_msg.seq > 0 {
                                fetch_stream.add_message(current_msg.clone());
                            }
                            break;
                        }

                        if buf.starts_with(b"* ") && !in_literal {
                            let line = String::from_utf8_lossy(&buf).to_string();
                            if line.contains("FETCH") {
                                let parts: Vec<&str> = line.split_whitespace().collect();
                                for (i, part) in parts.iter().enumerate() {
                                    if *part == "*"
                                        && i + 1 < parts.len()
                                        && let Ok(seq) = parts[i + 1].parse::<u32>()
                                    {
                                        if current_msg.seq > 0 {
                                            fetch_stream.add_message(current_msg.clone());
                                        }
                                        current_msg = FetchMessage {
                                            seq,
                                            attributes: HashMap::new(),
                                            bodies: HashMap::new(),
                                        };
                                    }
                                }
                                if line.contains("FLAGS")
                                    && let Some(start) = line.find("FLAGS")
                                    && let Some(end) = line[start..].find(')')
                                {
                                    current_msg.attributes.insert(
                                        "flags".to_string(),
                                        line[start + 6..start + end].to_string(),
                                    );
                                }
                            }
                        } else if buf.starts_with(b"{") {
                            let line = String::from_utf8_lossy(&buf).to_string();
                            if let Some(start) = line.find('{')
                                && let Some(end) = line.find('}')
                                && let Ok(size) = line[start + 1..end].parse::<usize>()
                            {
                                literal_size = size;
                                in_literal = true;
                                literal_body.clear();
                            }
                        } else if in_literal {
                            literal_body.extend_from_slice(&buf);
                            if literal_body.len() >= literal_size {
                                let actual = &literal_body[..literal_size];
                                current_msg
                                    .bodies
                                    .insert("BODY[]".to_string(), actual.to_vec());
                                in_literal = false;
                                literal_body.clear();
                            }
                        }
                    }

                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    inner
                        .fetch_streams
                        .lock()
                        .unwrap()
                        .insert(stream_id, fetch_stream);
                    Ok(stream_id)
                })();

                if let Err(e) = result {
                    eprintln!("IMAP fetch error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapFetch").unwrap().into(),
        fetch_fn.into(),
    );

    let fetch_body_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let _imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let stream_id = args.get(1).uint32_value(_scope).unwrap_or(0);

            let inner = inner().clone();

            let mut fetch_stream = {
                let mut guard = inner.fetch_streams.lock().unwrap();
                guard.remove(&stream_id)
            };

            if let Some(ref mut fs) = fetch_stream
                && let Some(msg) = fs.next()
                && let Some(body) = msg.bodies.get("BODY[]")
            {
                let encoded = base64_encode(body);
                rv.set(v8::String::new(_scope, &encoded).unwrap().into());
                return;
            }

            rv.set(v8::String::new(_scope, "").unwrap().into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapFetchBody").unwrap().into(),
        fetch_body_fn.into(),
    );

    let append_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let mailbox = args.get(1).to_rust_string_lossy(_scope);
            let body = args.get(2).to_rust_string_lossy(_scope);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let result: Result<(), String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} APPEND \"{}\" {{{}}}\r\n", tag, mailbox, body.len());
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let buf = imap_conn
                        .read_line()
                        .map_err(|e| format!("Read failed: {}", e))?;

                    if !buf.starts_with(b"+") {
                        inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                        return Err("APPEND failed".to_string());
                    }

                    imap_conn
                        .conn
                        .write_all(body.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let _resp_buf = imap_conn
                        .read_line()
                        .map_err(|e| format!("Read failed: {}", e))?;
                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP append error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapAppend").unwrap().into(),
        append_fn.into(),
    );

    let copy_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let sequence = args.get(1).to_rust_string_lossy(_scope);
            let mailbox = args.get(2).to_rust_string_lossy(_scope);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let result: Result<(), String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} COPY {} \"{}\"", tag, sequence, mailbox);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let _buf = imap_conn
                        .read_line()
                        .map_err(|e| format!("Read failed: {}", e))?;
                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP copy error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapCopy").unwrap().into(),
        copy_fn.into(),
    );

    let move_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let sequence = args.get(1).to_rust_string_lossy(_scope);
            let mailbox = args.get(2).to_rust_string_lossy(_scope);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let result: Result<(), String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} MOVE {} \"{}\"", tag, sequence, mailbox);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let _buf = imap_conn
                        .read_line()
                        .map_err(|e| format!("Read failed: {}", e))?;
                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP move error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapMove").unwrap().into(),
        move_fn.into(),
    );

    let add_flags_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let sequence = args.get(1).to_rust_string_lossy(_scope);
            let flags = args.get(2).to_rust_string_lossy(_scope);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let result: Result<(), String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} STORE {} +FLAGS ({})", tag, sequence, flags);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let _buf = imap_conn
                        .read_line()
                        .map_err(|e| format!("Read failed: {}", e))?;
                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP add flags error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapAddFlags").unwrap().into(),
        add_flags_fn.into(),
    );

    let remove_flags_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let sequence = args.get(1).to_rust_string_lossy(_scope);
            let flags = args.get(2).to_rust_string_lossy(_scope);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let result: Result<(), String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} STORE {} -FLAGS ({})", tag, sequence, flags);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let _buf = imap_conn
                        .read_line()
                        .map_err(|e| format!("Read failed: {}", e))?;
                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP remove flags error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapRemoveFlags").unwrap().into(),
        remove_flags_fn.into(),
    );

    let set_flags_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let sequence = args.get(1).to_rust_string_lossy(_scope);
            let flags = args.get(2).to_rust_string_lossy(_scope);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let result: Result<(), String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} STORE {} FLAGS ({})", tag, sequence, flags);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let _buf = imap_conn
                        .read_line()
                        .map_err(|e| format!("Read failed: {}", e))?;
                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP set flags error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapSetFlags").unwrap().into(),
        set_flags_fn.into(),
    );

    let expunge_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let result: Result<(), String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} EXPUNGE", tag);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    loop {
                        let buf = imap_conn
                            .read_line()
                            .map_err(|e| format!("Read failed: {}", e))?;
                        if buf.starts_with(tag.as_bytes()) {
                            break;
                        }
                    }

                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP expunge error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapExpunge").unwrap().into(),
        expunge_fn.into(),
    );

    let search_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            let criteria = args.get(1).to_rust_string_lossy(_scope);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let result: Result<String, String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} SEARCH {}", tag, criteria);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let mut seq_nums = Vec::new();
                    loop {
                        let buf = imap_conn
                            .read_line()
                            .map_err(|e| format!("Read failed: {}", e))?;
                        if buf.starts_with(tag.as_bytes()) {
                            break;
                        }
                        let line = String::from_utf8_lossy(&buf).to_string();
                        if line.contains("SEARCH") {
                            for part in line.split_whitespace() {
                                if let Ok(n) = part.parse::<u32>() {
                                    seq_nums.push(n);
                                }
                            }
                        }
                    }

                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(serde_json::json!(seq_nums).to_string())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP search error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapSearch").unwrap().into(),
        search_fn.into(),
    );

    let logout_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let result: Result<(), String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} LOGOUT", tag);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    loop {
                        let buf = imap_conn
                            .read_line()
                            .map_err(|e| format!("Read failed: {}", e))?;
                        if buf.starts_with(tag.as_bytes()) {
                            break;
                        }
                    }

                    imap_conn.conn.shutdown();
                    Ok(())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP logout error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapLogout").unwrap().into(),
        logout_fn.into(),
    );

    let disconnect_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);

            if let Some(mut conn) = inner().connections.lock().unwrap().remove(&imap_id) {
                conn.conn.shutdown();
            }

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapDisconnect").unwrap().into(),
        disconnect_fn.into(),
    );

    let close_fn = v8::Function::new(
        scope,
        move |_scope: &mut v8::PinScope,
              args: v8::FunctionCallbackArguments,
              mut rv: v8::ReturnValue| {
            let imap_id = args.get(0).uint32_value(_scope).unwrap_or(0);

            let inner = inner().clone();

            std::thread::spawn(move || {
                let result: Result<(), String> = (|| {
                    let mut imap_conn = {
                        let mut guard = inner.connections.lock().unwrap();
                        guard
                            .remove(&imap_id)
                            .ok_or_else(|| "unknown IMAP connection".to_string())?
                    };

                    let tag = imap_conn.next_tag();
                    let cmd = format!("{} CLOSE", tag);
                    imap_conn
                        .conn
                        .write_all(cmd.as_bytes())
                        .map_err(|e| format!("Write failed: {}", e))?;
                    imap_conn
                        .conn
                        .write_all(CRLF)
                        .map_err(|e| format!("Write failed: {}", e))?;

                    let _buf = imap_conn
                        .read_line()
                        .map_err(|e| format!("Read failed: {}", e))?;

                    imap_conn.selected_mailbox = None;
                    imap_conn.state = ImapState::Authenticated;
                    inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                    Ok(())
                })();

                if let Err(e) = result {
                    eprintln!("IMAP close error: {}", e);
                }
            });

            rv.set(v8::undefined(_scope).into());
        },
    )
    .unwrap();
    global.set(
        scope,
        v8::String::new(scope, "__imapClose").unwrap().into(),
        close_fn.into(),
    );

    let js_code = r#"
    (function() {
      var EventEmitter = function() {};
      EventEmitter.prototype.on = function(event, fn) { this._events = this._events || {}; (this._events[event] = this._events[event] || []).push(fn); return this; };
      EventEmitter.prototype.emit = function(event) { var args = Array.prototype.slice.call(arguments, 1); var list = this._events ? this._events[event] || [] : []; list.forEach(function(fn) { fn.apply(null, args); }); return list.length > 0; };
      EventEmitter.prototype.off = function(event, fn) { var list = (this._events || {})[event] || []; var idx = list.indexOf(fn); if (idx >= 0) list.splice(idx, 1); return this; };

      var imapSymbol = Symbol('imap');
      var streamSymbol = Symbol('imapStream');

      function ImapClient(options) {
        EventEmitter.call(this);
        this[imapSymbol] = {
          id: null,
          connected: false,
          host: options.host,
          port: options.port || (options.tls ? 993 : 143),
          tls: options.tls || false,
          username: options.username,
          password: options.password,
          tlsOptions: options.tlsOptions || {},
          readyFired: false,
        };
      }
      ImapClient.prototype = Object.create(EventEmitter.prototype);
      ImapClient.prototype.constructor = ImapClient;

      ImapClient.prototype.connect = async function() {
        var self = this[imapSymbol];
        if (self.connected) return;
        try {
          self.id = __imapCreate(JSON.stringify({ host: self.host, port: self.port }));
          __imapConnect(self.id, self.host, self.port, self.tls ? 1 : 0);
          await new Promise(function(res) { setTimeout(res, 50); });
          if (self.username && self.password) {
            await this.login(self.username, self.password);
          }
          self.connected = true;
          if (!self.readyFired) {
            self.readyFired = true;
            var self2 = this;
            setTimeout(function() { self2.emit('ready'); }, 0);
          }
        } catch (e) {
          this.emit('error', e);
          throw e;
        }
      };

      ImapClient.prototype.login = async function(username, password) {
        var self = this[imapSymbol];
        __imapLogin(self.id, username, password);
        await new Promise(function(res) { setTimeout(res, 50); });
      };

      ImapClient.prototype.openBox = async function(name, readOnly) {
        var self = this[imapSymbol];
        __imapSelect(self.id, name, readOnly ? 1 : 0);
        await new Promise(function(res) { setTimeout(res, 50); });
        var status = __imapStatus(self.id, name, 'MESSAGES RECENT UNSEEN UIDNEXT UIDVALIDITY');
        return JSON.parse(status);
      };

      ImapClient.prototype.status = async function(name) {
        var self = this[imapSymbol];
        var status = __imapStatus(self.id, name, 'MESSAGES RECENT UNSEEN UIDNEXT UIDVALIDITY');
        return JSON.parse(status);
      };

      ImapClient.prototype.createBox = async function(name) {
        var self = this[imapSymbol];
        __imapCreateMailbox(self.id, name);
        await new Promise(function(res) { setTimeout(res, 50); });
      };

      ImapClient.prototype.deleteBox = async function(name) {
        var self = this[imapSymbol];
        __imapDeleteMailbox(self.id, name);
        await new Promise(function(res) { setTimeout(res, 50); });
      };

      ImapClient.prototype.renameBox = async function(oldName, newName) {
        var self = this[imapSymbol];
        __imapRenameMailbox(self.id, oldName, newName);
        await new Promise(function(res) { setTimeout(res, 50); });
      };

      ImapClient.prototype.subscribeBox = async function(name) {
        var self = this[imapSymbol];
        __imapSubscribe(self.id, name);
        await new Promise(function(res) { setTimeout(res, 50); });
      };

      ImapClient.prototype.listBoxes = async function(reference, mailbox) {
        var self = this[imapSymbol];
        var result = __imapListMailboxes(self.id, reference || '', mailbox || '*');
        return JSON.parse(result);
      };

      ImapClient.prototype.fetch = function(range, options) {
        var self = this[imapSymbol];
        options = options || {};
        var bodies = options.bodies || '';
        var stream = new FetchStream(self.id, range, bodies);
        this[streamSymbol] = stream;
        return stream;
      };

      ImapClient.prototype.append = async function(body, options) {
        var self = this[imapSymbol];
        options = options || {};
        var mailbox = options.mailbox || 'INBOX';
        __imapAppend(self.id, mailbox, body);
        await new Promise(function(res) { setTimeout(res, 50); });
      };

      ImapClient.prototype.copy = async function(range, mailbox) {
        var self = this[imapSymbol];
        __imapCopy(self.id, range, mailbox);
        await new Promise(function(res) { setTimeout(res, 50); });
      };

      ImapClient.prototype.move = async function(range, mailbox) {
        var self = this[imapSymbol];
        __imapMove(self.id, range, mailbox);
        await new Promise(function(res) { setTimeout(res, 50); });
      };

      ImapClient.prototype.addFlags = async function(range, flags) {
        var self = this[imapSymbol];
        var flagStr = flags.map(function(f) { return f.startsWith('\\') ? f : '\\' + f; }).join(' ');
        __imapAddFlags(self.id, range, flagStr);
        await new Promise(function(res) { setTimeout(res, 50); });
      };

      ImapClient.prototype.removeFlags = async function(range, flags) {
        var self = this[imapSymbol];
        var flagStr = flags.map(function(f) { return f.startsWith('\\') ? f : '\\' + f; }).join(' ');
        __imapRemoveFlags(self.id, range, flagStr);
        await new Promise(function(res) { setTimeout(res, 50); });
      };

      ImapClient.prototype.setFlags = async function(range, flags) {
        var self = this[imapSymbol];
        var flagStr = flags.map(function(f) { return f.startsWith('\\') ? f : '\\' + f; }).join(' ');
        __imapSetFlags(self.id, range, flagStr);
        await new Promise(function(res) { setTimeout(res, 50); });
      };

      ImapClient.prototype.expunge = async function() {
        var self = this[imapSymbol];
        __imapExpunge(self.id);
        await new Promise(function(res) { setTimeout(res, 50); });
      };

      ImapClient.prototype.search = async function(criteria) {
        var self = this[imapSymbol];
        var critStr = '';
        for (var i = 0; i < criteria.length; i++) {
          var item = criteria[i];
          if (Array.isArray(item)) {
            critStr += ' ' + item[0] + ' ' + (typeof item[1] === 'string' ? '"' + item[1] + '"' : item[1]);
          } else {
            critStr += ' ' + item;
          }
        }
        var result = __imapSearch(self.id, critStr.trim());
        return JSON.parse(result);
      };

      ImapClient.prototype.end = async function() {
        var self = this[imapSymbol];
        if (self.id !== null) {
          __imapLogout(self.id);
          await new Promise(function(res) { setTimeout(res, 50); });
          self.connected = false;
          this.emit('close');
        }
      };

      ImapClient.prototype.disconnect = function() {
        var self = this[imapSymbol];
        if (self.id !== null) {
          __imapDisconnect(self.id);
          self.connected = false;
          self.id = null;
          this.emit('close');
        }
      };

      ImapClient.prototype.closeBox = async function() {
        var self = this[imapSymbol];
        __imapClose(self.id);
        await new Promise(function(res) { setTimeout(res, 50); });
      };

      function FetchStream(imapId, range, bodies) {
        EventEmitter.call(this);
        this.imapId = imapId;
        this.range = range;
        this.bodies = bodies;
        this._streamId = null;
        this._messages = [];
        this._current = 0;
        this._fetching = false;
      }
      FetchStream.prototype = Object.create(EventEmitter.prototype);
      FetchStream.prototype.constructor = FetchStream;

      FetchStream.prototype.start = async function() {
        if (this._fetching) return;
        this._fetching = true;
        try {
          this._streamId = __imapFetch(this.imapId, this.range, this.bodies);
        } catch(e) {
          this.emit('error', e);
        }
      };

      FetchStream.prototype.next = async function() {
        if (!this._streamId) await this.start();
        if (this._current >= this._messages.length) {
          return null;
        }
        return this._messages[this._current++];
      };

      FetchStream.prototype.on = function(event, fn) {
        if (event === 'message') {
          this._onMessage = fn;
        }
        return EventEmitter.prototype.on.call(this, event, fn);
      };

      var imapMod = { Client: ImapClient };
      globalThis.imap = imapMod;
      if (globalThis.__requireCache) {
        globalThis.__requireCache['imap'] = imapMod;
        globalThis.__requireCache['node:imap'] = imapMod;
      }
    })();
    "#;

    let source = v8::String::new(scope, js_code).unwrap();
    if let Some(script) = v8::Script::compile(scope, source, None) {
        let _ = script.run(scope);
    }
}
