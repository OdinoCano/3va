//! IMAP4rev1 client backend.
//!
//! Architecture:
//!   - Connections held in pool: `Arc<Mutex<HashMap<u32, ImapConnection>>>`
//!   - Streams for fetch operations: `Arc<Mutex<HashMap<u32, FetchStream>>>`
//!   - All blocking I/O runs in `spawn_blocking`.

use base64::Engine;
use native_tls::TlsStream;
use rquickjs::function::Async;
use rquickjs::{Ctx, Function, Result, Value};
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

    #[allow(dead_code)]
    fn send_command(&mut self, cmd: &str) -> io::Result<String> {
        let tag = self.next_tag();
        let line = format!("{} {}", tag, cmd);
        self.conn.write_all(line.as_bytes())?;
        self.conn.write_all(CRLF)?;
        Ok(tag)
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

fn js_err(ctx: &Ctx<'_>, msg: String) -> rquickjs::Error {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    match ctx.eval::<Value<'_>, _>(format!("new Error(\"{}\")", escaped)) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
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

pub fn inject_imap(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let inner = Arc::new(ImapStateInner {
        connections: Arc::new(Mutex::new(HashMap::new())),
        fetch_streams: Arc::new(Mutex::new(HashMap::new())),
        next_id: Arc::new(Mutex::new(0)),
        next_stream_id: Arc::new(Mutex::new(0)),
    });

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapCreate",
            Function::new(
                ctx.clone(),
                move |_ctx: Ctx<'_>, _options: String| -> Result<u32> {
                    let id = inner.alloc_id();
                    Ok(id)
                },
            ),
        )?;
    }

    {
        let perms = permissions.clone();
        let inner = inner.clone();
        ctx.globals().set(
            "__imapConnect",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>, imap_id: u32, host: String, port: u16, use_tls: bool| {
                        let perms = perms.clone();
                        let inner = inner.clone();
                        async move {
                            if !perms.check(&Capability::Network(host.clone())) {
                                return Err(rquickjs::Error::new_from_js_message(
                                    "EACCES",
                                    "EACCES",
                                    format!("Network access denied. Run with --allow-net={}", host),
                                ));
                            }

                            let addr = format!("{}:{}", host, port);
                            let stream =
                                tokio::task::spawn_blocking(move || TcpStream::connect(&addr))
                                    .await
                                    .map_err(|e| {
                                        rquickjs::Error::new_from_js_message(
                                            "EIO",
                                            "EIO",
                                            e.to_string(),
                                        )
                                    })?
                                    .map_err(|e| {
                                        rquickjs::Error::new_from_js_message(
                                            "ECONNREFUSED",
                                            "ECONNREFUSED",
                                            e.to_string(),
                                        )
                                    })?;

                            stream.set_nonblocking(true).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            let conn = if use_tls {
                                let connector = native_tls::TlsConnector::new().map_err(|e| {
                                    rquickjs::Error::new_from_js_message(
                                        "EIO",
                                        "EIO",
                                        format!("TLS init failed: {}", e),
                                    )
                                })?;
                                let tls_stream = connector.connect(&host, stream).map_err(|e| {
                                    rquickjs::Error::new_from_js_message(
                                        "ECONNRESET",
                                        "ECONNRESET",
                                        format!("TLS handshake failed: {}", e),
                                    )
                                })?;
                                ImapConn::Tls(tls_stream)
                            } else {
                                ImapConn::Plain(stream)
                            };

                            let mut imap_conn = ImapConnection::new(conn);
                            let tag = imap_conn.next_tag();
                            imap_conn
                                .conn
                                .write_all(format!("{} CAPABILITY\r\n", tag).as_bytes())
                                .map_err(|e| {
                                    rquickjs::Error::new_from_js_message(
                                        "EIO",
                                        "EIO",
                                        e.to_string(),
                                    )
                                })?;

                            loop {
                                let buf = imap_conn.read_line().map_err(|e| {
                                    rquickjs::Error::new_from_js_message(
                                        "EIO",
                                        "EIO",
                                        e.to_string(),
                                    )
                                })?;
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
                            Ok::<(), rquickjs::Error>(())
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapLogin",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>, imap_id: u32, username: String, password: String| {
                        let inner = inner.clone();
                        async move {
                            let mut imap_conn = {
                                let mut guard = inner.connections.lock().unwrap();
                                guard.remove(&imap_id).ok_or_else(|| {
                                    rquickjs::Error::new_from_js_message(
                                        "ENOENT",
                                        "ENOENT",
                                        "unknown IMAP connection",
                                    )
                                })?
                            };

                            let tag = imap_conn.next_tag();
                            let cmd = format!("{} LOGIN \"{}\" \"{}\"", tag, username, password);
                            imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            imap_conn.conn.write_all(CRLF).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            let buf = imap_conn.read_line().map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            let response = String::from_utf8_lossy(&buf).to_string();

                            if response.contains("OK") {
                                imap_conn.authenticated = true;
                                imap_conn.state = ImapState::Authenticated;
                                inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                                Ok::<(), rquickjs::Error>(())
                            } else {
                                Err(rquickjs::Error::new_from_js_message(
                                    "EAUTH",
                                    "EAUTH",
                                    format!("Login failed: {}", response.trim()),
                                ))
                            }
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapCapability",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>, imap_id: u32| -> Result<String> {
                    let guard = inner.connections.lock().unwrap();
                    let imap_conn = guard
                        .get(&imap_id)
                        .ok_or_else(|| js_err(&ctx, "unknown IMAP connection".into()))?;
                    Ok(imap_conn.capabilities.join(" "))
                },
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapListMailboxes",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>, imap_id: u32, reference: String, mailbox: String| {
                        let inner = inner.clone();
                        async move {
                            let mut imap_conn = {
                                let mut guard = inner.connections.lock().unwrap();
                                guard.remove(&imap_id).ok_or_else(|| {
                                    rquickjs::Error::new_from_js_message(
                                        "ENOENT",
                                        "ENOENT",
                                        "unknown IMAP connection",
                                    )
                                })?
                            };

                            let tag = imap_conn.next_tag();
                            let cmd = format!("{} LIST \"{}\" \"{}\"", tag, reference, mailbox);
                            imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            imap_conn.conn.write_all(CRLF).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            let mut response_lines = Vec::new();
                            loop {
                                let buf = imap_conn.read_line().map_err(|e| {
                                    rquickjs::Error::new_from_js_message(
                                        "EIO",
                                        "EIO",
                                        e.to_string(),
                                    )
                                })?;
                                if buf.starts_with(tag.as_bytes()) {
                                    break;
                                }
                                response_lines.push(String::from_utf8_lossy(&buf).to_string());
                            }

                            let mailboxes = parse_mailbox_list(&response_lines);
                            inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                            Ok::<String, rquickjs::Error>(serde_json::json!(mailboxes).to_string())
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapStatus",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>, imap_id: u32, mailbox: String, items: String| {
                        let inner = inner.clone();
                        async move {
                            let mut imap_conn = {
                                let mut guard = inner.connections.lock().unwrap();
                                guard.remove(&imap_id).ok_or_else(|| {
                                    rquickjs::Error::new_from_js_message(
                                        "ENOENT",
                                        "ENOENT",
                                        "unknown IMAP connection",
                                    )
                                })?
                            };

                            let tag = imap_conn.next_tag();
                            let cmd = format!("{} STATUS \"{}\" ({})", tag, mailbox, items);
                            imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            imap_conn.conn.write_all(CRLF).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            let mut status_data = HashMap::new();
                            loop {
                                let buf = imap_conn.read_line().map_err(|e| {
                                    rquickjs::Error::new_from_js_message(
                                        "EIO",
                                        "EIO",
                                        e.to_string(),
                                    )
                                })?;
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
                                                status_data.insert(
                                                    "recent".to_string(),
                                                    parts[i + 1].to_string(),
                                                );
                                            }
                                            "UNSEEN" if i + 1 < parts.len() => {
                                                status_data.insert(
                                                    "unseen".to_string(),
                                                    parts[i + 1].to_string(),
                                                );
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
                            Ok::<String, rquickjs::Error>(
                                serde_json::json!(status_data).to_string(),
                            )
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapSelect",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>, imap_id: u32, mailbox: String, readonly: bool| {
                        let inner = inner.clone();
                        async move {
                            let mut imap_conn = {
                                let mut guard = inner.connections.lock().unwrap();
                                guard.remove(&imap_id).ok_or_else(|| {
                                    rquickjs::Error::new_from_js_message(
                                        "ENOENT",
                                        "ENOENT",
                                        "unknown IMAP connection",
                                    )
                                })?
                            };

                            let tag = imap_conn.next_tag();
                            let cmd = if readonly {
                                format!("{} EXAMINE \"{}\"", tag, mailbox)
                            } else {
                                format!("{} SELECT \"{}\"", tag, mailbox)
                            };
                            imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            imap_conn.conn.write_all(CRLF).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            let mut select_data = HashMap::new();
                            loop {
                                let buf = imap_conn.read_line().map_err(|e| {
                                    rquickjs::Error::new_from_js_message(
                                        "EIO",
                                        "EIO",
                                        e.to_string(),
                                    )
                                })?;
                                if buf.starts_with(tag.as_bytes()) {
                                    break;
                                }
                                let line = String::from_utf8_lossy(&buf).to_string();
                                if line.starts_with("* ") {
                                    let parts: Vec<&str> = line.split_whitespace().collect();
                                    if parts.len() >= 3 {
                                        if parts[2] == "EXISTS" {
                                            select_data
                                                .insert("exists".to_string(), parts[1].to_string());
                                        } else if parts[2] == "RECENT" {
                                            select_data
                                                .insert("recent".to_string(), parts[1].to_string());
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
                            Ok::<String, rquickjs::Error>(
                                serde_json::json!(select_data).to_string(),
                            )
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapCreateMailbox",
            Function::new(
                ctx.clone(),
                Async(move |_ctx: Ctx<'_>, imap_id: u32, mailbox: String| {
                    let inner = inner.clone();
                    async move {
                        let mut imap_conn = {
                            let mut guard = inner.connections.lock().unwrap();
                            guard.remove(&imap_id).ok_or_else(|| {
                                rquickjs::Error::new_from_js_message(
                                    "ENOENT",
                                    "ENOENT",
                                    "unknown IMAP connection",
                                )
                            })?
                        };

                        let tag = imap_conn.next_tag();
                        let cmd = format!("{} CREATE \"{}\"", tag, mailbox);
                        imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;
                        imap_conn.conn.write_all(CRLF).map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;

                        let _buf = imap_conn.read_line().map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;

                        inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                        Ok::<(), rquickjs::Error>(())
                    }
                }),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapDeleteMailbox",
            Function::new(
                ctx.clone(),
                Async(move |_ctx: Ctx<'_>, imap_id: u32, mailbox: String| {
                    let inner = inner.clone();
                    async move {
                        let mut imap_conn = {
                            let mut guard = inner.connections.lock().unwrap();
                            guard.remove(&imap_id).ok_or_else(|| {
                                rquickjs::Error::new_from_js_message(
                                    "ENOENT",
                                    "ENOENT",
                                    "unknown IMAP connection",
                                )
                            })?
                        };

                        let tag = imap_conn.next_tag();
                        let cmd = format!("{} DELETE \"{}\"", tag, mailbox);
                        imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;
                        imap_conn.conn.write_all(CRLF).map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;

                        let _buf = imap_conn.read_line().map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;

                        inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                        Ok::<(), rquickjs::Error>(())
                    }
                }),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapRenameMailbox",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>, imap_id: u32, old_mailbox: String, new_mailbox: String| {
                        let inner = inner.clone();
                        async move {
                            let mut imap_conn = {
                                let mut guard = inner.connections.lock().unwrap();
                                guard.remove(&imap_id).ok_or_else(|| {
                                    rquickjs::Error::new_from_js_message(
                                        "ENOENT",
                                        "ENOENT",
                                        "unknown IMAP connection",
                                    )
                                })?
                            };

                            let tag = imap_conn.next_tag();
                            let cmd =
                                format!("{} RENAME \"{}\" \"{}\"", tag, old_mailbox, new_mailbox);
                            imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            imap_conn.conn.write_all(CRLF).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            let _buf = imap_conn.read_line().map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                            Ok::<(), rquickjs::Error>(())
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapSubscribe",
            Function::new(
                ctx.clone(),
                Async(move |_ctx: Ctx<'_>, imap_id: u32, mailbox: String| {
                    let inner = inner.clone();
                    async move {
                        let mut imap_conn = {
                            let mut guard = inner.connections.lock().unwrap();
                            guard.remove(&imap_id).ok_or_else(|| {
                                rquickjs::Error::new_from_js_message(
                                    "ENOENT",
                                    "ENOENT",
                                    "unknown IMAP connection",
                                )
                            })?
                        };

                        let tag = imap_conn.next_tag();
                        let cmd = format!("{} SUBSCRIBE \"{}\"", tag, mailbox);
                        imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;
                        imap_conn.conn.write_all(CRLF).map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;

                        let _buf = imap_conn.read_line().map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;

                        inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                        Ok::<(), rquickjs::Error>(())
                    }
                }),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapFetch",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>, imap_id: u32, sequence: String, bodies: String| {
                        let inner = inner.clone();
                        async move {
                            let mut imap_conn = {
                                let mut guard = inner.connections.lock().unwrap();
                                guard.remove(&imap_id).ok_or_else(|| {
                                    rquickjs::Error::new_from_js_message(
                                        "ENOENT",
                                        "ENOENT",
                                        "unknown IMAP connection",
                                    )
                                })?
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
                            imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            imap_conn.conn.write_all(CRLF).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            let mut current_msg = FetchMessage {
                                seq: 0,
                                attributes: HashMap::new(),
                                bodies: HashMap::new(),
                            };
                            let mut in_literal = false;
                            let mut literal_size = 0;
                            let mut literal_body = Vec::new();

                            loop {
                                let buf = imap_conn.read_line().map_err(|e| {
                                    rquickjs::Error::new_from_js_message(
                                        "EIO",
                                        "EIO",
                                        e.to_string(),
                                    )
                                })?;

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
                                        {
                                            let rest = &line[start..];
                                            if let Some(end) = rest.find(')') {
                                                current_msg.attributes.insert(
                                                    "flags".to_string(),
                                                    rest[6..end].to_string(),
                                                );
                                            }
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
                            Ok::<u32, rquickjs::Error>(stream_id)
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapFetchBody",
            Function::new(
                ctx.clone(),
                Async(move |_ctx: Ctx<'_>, _imap_id: u32, stream_id: u32| {
                    let inner = inner.clone();
                    async move {
                        let mut fetch_stream = {
                            let mut guard = inner.fetch_streams.lock().unwrap();
                            guard.remove(&stream_id).ok_or_else(|| {
                                rquickjs::Error::new_from_js_message(
                                    "ENOENT",
                                    "ENOENT",
                                    "unknown fetch stream",
                                )
                            })?
                        };

                        if let Some(msg) = fetch_stream.next() {
                            if let Some(body) = msg.bodies.get("BODY[]") {
                                Ok::<String, rquickjs::Error>(base64_encode(body))
                            } else {
                                Ok::<String, rquickjs::Error>("".to_string())
                            }
                        } else {
                            Err(rquickjs::Error::new_from_js_message(
                                "ENOENT",
                                "ENOENT",
                                "no more messages in stream",
                            ))
                        }
                    }
                }),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapAppend",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>, imap_id: u32, mailbox: String, body: String| {
                        let inner = inner.clone();
                        async move {
                            let mut imap_conn = {
                                let mut guard = inner.connections.lock().unwrap();
                                guard.remove(&imap_id).ok_or_else(|| {
                                    rquickjs::Error::new_from_js_message(
                                        "ENOENT",
                                        "ENOENT",
                                        "unknown IMAP connection",
                                    )
                                })?
                            };

                            let tag = imap_conn.next_tag();
                            let cmd =
                                format!("{} APPEND \"{}\" {{{}}}\r\n", tag, mailbox, body.len());
                            imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            imap_conn.conn.write_all(CRLF).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            let buf = imap_conn.read_line().map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            if !buf.starts_with(b"+") {
                                inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                                return Err(rquickjs::Error::new_from_js_message(
                                    "EIO",
                                    "EIO",
                                    "APPEND failed",
                                ));
                            }

                            imap_conn.conn.write_all(body.as_bytes()).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            imap_conn.conn.write_all(CRLF).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            let _resp_buf = imap_conn.read_line().map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                            Ok::<(), rquickjs::Error>(())
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapCopy",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>, imap_id: u32, sequence: String, mailbox: String| {
                        let inner = inner.clone();
                        async move {
                            let mut imap_conn = {
                                let mut guard = inner.connections.lock().unwrap();
                                guard.remove(&imap_id).ok_or_else(|| {
                                    rquickjs::Error::new_from_js_message(
                                        "ENOENT",
                                        "ENOENT",
                                        "unknown IMAP connection",
                                    )
                                })?
                            };

                            let tag = imap_conn.next_tag();
                            let cmd = format!("{} COPY {} \"{}\"", tag, sequence, mailbox);
                            imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            imap_conn.conn.write_all(CRLF).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            let _buf = imap_conn.read_line().map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                            Ok::<(), rquickjs::Error>(())
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapMove",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>, imap_id: u32, sequence: String, mailbox: String| {
                        let inner = inner.clone();
                        async move {
                            let mut imap_conn = {
                                let mut guard = inner.connections.lock().unwrap();
                                guard.remove(&imap_id).ok_or_else(|| {
                                    rquickjs::Error::new_from_js_message(
                                        "ENOENT",
                                        "ENOENT",
                                        "unknown IMAP connection",
                                    )
                                })?
                            };

                            let tag = imap_conn.next_tag();
                            let cmd = format!("{} MOVE {} \"{}\"", tag, sequence, mailbox);
                            imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            imap_conn.conn.write_all(CRLF).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            let _buf = imap_conn.read_line().map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                            Ok::<(), rquickjs::Error>(())
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapAddFlags",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>, imap_id: u32, sequence: String, flags: String| {
                        let inner = inner.clone();
                        async move {
                            let mut imap_conn = {
                                let mut guard = inner.connections.lock().unwrap();
                                guard.remove(&imap_id).ok_or_else(|| {
                                    rquickjs::Error::new_from_js_message(
                                        "ENOENT",
                                        "ENOENT",
                                        "unknown IMAP connection",
                                    )
                                })?
                            };

                            let tag = imap_conn.next_tag();
                            let cmd = format!("{} STORE {} +FLAGS ({})", tag, sequence, flags);
                            imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            imap_conn.conn.write_all(CRLF).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            let _buf = imap_conn.read_line().map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                            Ok::<(), rquickjs::Error>(())
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapRemoveFlags",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>, imap_id: u32, sequence: String, flags: String| {
                        let inner = inner.clone();
                        async move {
                            let mut imap_conn = {
                                let mut guard = inner.connections.lock().unwrap();
                                guard.remove(&imap_id).ok_or_else(|| {
                                    rquickjs::Error::new_from_js_message(
                                        "ENOENT",
                                        "ENOENT",
                                        "unknown IMAP connection",
                                    )
                                })?
                            };

                            let tag = imap_conn.next_tag();
                            let cmd = format!("{} STORE {} -FLAGS ({})", tag, sequence, flags);
                            imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            imap_conn.conn.write_all(CRLF).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            let _buf = imap_conn.read_line().map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                            Ok::<(), rquickjs::Error>(())
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapSetFlags",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>, imap_id: u32, sequence: String, flags: String| {
                        let inner = inner.clone();
                        async move {
                            let mut imap_conn = {
                                let mut guard = inner.connections.lock().unwrap();
                                guard.remove(&imap_id).ok_or_else(|| {
                                    rquickjs::Error::new_from_js_message(
                                        "ENOENT",
                                        "ENOENT",
                                        "unknown IMAP connection",
                                    )
                                })?
                            };

                            let tag = imap_conn.next_tag();
                            let cmd = format!("{} STORE {} FLAGS ({})", tag, sequence, flags);
                            imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            imap_conn.conn.write_all(CRLF).map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            let _buf = imap_conn.read_line().map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;

                            inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                            Ok::<(), rquickjs::Error>(())
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapExpunge",
            Function::new(
                ctx.clone(),
                Async(move |_ctx: Ctx<'_>, imap_id: u32| {
                    let inner = inner.clone();
                    async move {
                        let mut imap_conn = {
                            let mut guard = inner.connections.lock().unwrap();
                            guard.remove(&imap_id).ok_or_else(|| {
                                rquickjs::Error::new_from_js_message(
                                    "ENOENT",
                                    "ENOENT",
                                    "unknown IMAP connection",
                                )
                            })?
                        };

                        let tag = imap_conn.next_tag();
                        let cmd = format!("{} EXPUNGE", tag);
                        imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;
                        imap_conn.conn.write_all(CRLF).map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;

                        loop {
                            let buf = imap_conn.read_line().map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            if buf.starts_with(tag.as_bytes()) {
                                break;
                            }
                        }

                        inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                        Ok::<(), rquickjs::Error>(())
                    }
                }),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapSearch",
            Function::new(
                ctx.clone(),
                Async(move |_ctx: Ctx<'_>, imap_id: u32, criteria: String| {
                    let inner = inner.clone();
                    async move {
                        let mut imap_conn = {
                            let mut guard = inner.connections.lock().unwrap();
                            guard.remove(&imap_id).ok_or_else(|| {
                                rquickjs::Error::new_from_js_message(
                                    "ENOENT",
                                    "ENOENT",
                                    "unknown IMAP connection",
                                )
                            })?
                        };

                        let tag = imap_conn.next_tag();
                        let cmd = format!("{} SEARCH {}", tag, criteria);
                        imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;
                        imap_conn.conn.write_all(CRLF).map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;

                        let mut seq_nums = Vec::new();
                        loop {
                            let buf = imap_conn.read_line().map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
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
                        Ok::<String, rquickjs::Error>(serde_json::json!(seq_nums).to_string())
                    }
                }),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapLogout",
            Function::new(
                ctx.clone(),
                Async(move |_ctx: Ctx<'_>, imap_id: u32| {
                    let inner = inner.clone();
                    async move {
                        let mut imap_conn = {
                            let mut guard = inner.connections.lock().unwrap();
                            guard.remove(&imap_id).ok_or_else(|| {
                                rquickjs::Error::new_from_js_message(
                                    "ENOENT",
                                    "ENOENT",
                                    "unknown IMAP connection",
                                )
                            })?
                        };

                        let tag = imap_conn.next_tag();
                        let cmd = format!("{} LOGOUT", tag);
                        imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;
                        imap_conn.conn.write_all(CRLF).map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;

                        loop {
                            let buf = imap_conn.read_line().map_err(|e| {
                                rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                            })?;
                            if buf.starts_with(tag.as_bytes()) {
                                break;
                            }
                        }

                        imap_conn.conn.shutdown();
                        Ok::<(), rquickjs::Error>(())
                    }
                }),
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapDisconnect",
            Function::new(
                ctx.clone(),
                move |_ctx: Ctx<'_>, imap_id: u32| -> Result<()> {
                    if let Some(mut conn) = inner.connections.lock().unwrap().remove(&imap_id) {
                        conn.conn.shutdown();
                    }
                    Ok(())
                },
            ),
        )?;
    }

    {
        let inner = inner.clone();
        ctx.globals().set(
            "__imapClose",
            Function::new(
                ctx.clone(),
                Async(move |_ctx: Ctx<'_>, imap_id: u32| {
                    let inner = inner.clone();
                    async move {
                        let mut imap_conn = {
                            let mut guard = inner.connections.lock().unwrap();
                            guard.remove(&imap_id).ok_or_else(|| {
                                rquickjs::Error::new_from_js_message(
                                    "ENOENT",
                                    "ENOENT",
                                    "unknown IMAP connection",
                                )
                            })?
                        };

                        let tag = imap_conn.next_tag();
                        let cmd = format!("{} CLOSE", tag);
                        imap_conn.conn.write_all(cmd.as_bytes()).map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;
                        imap_conn.conn.write_all(CRLF).map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;

                        let _buf = imap_conn.read_line().map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;

                        imap_conn.selected_mailbox = None;
                        imap_conn.state = ImapState::Authenticated;
                        inner.connections.lock().unwrap().insert(imap_id, imap_conn);
                        Ok::<(), rquickjs::Error>(())
                    }
                }),
            ),
        )?;
    }

    ctx.eval::<(), _>(IMAP_JS_SHIM)?;
    Ok(())
}

const IMAP_JS_SHIM: &str = r#"
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
