//! gRPC client backend using tonic and prost.
//!
//! Architecture:
//!   - Channels are held in a pool: `Arc<Mutex<HashMap<u32, Channel>>>`
//!   - Streaming calls use separate stream objects managed by ID
//!   - Protobuf encoding/decoding runs in `spawn_blocking` to avoid blocking the JS thread.
//!
//! Native functions exposed:
//!   __grpcLoadProto(protoContent, packageName) -> packageDef
//!   __grpcCreateChannel(host, port, useTls) -> channelId
//!   __grpcMakeUnaryCall(channelId, service, method, requestBytes) -> responseBytes
//!   __grpcCreateServerStream(channelId, service, method, requestBytes) -> streamId
//!   __grpcCreateClientStream(channelId, service, method) -> streamId
//!   __grpcStreamWrite(streamId, dataBytes)
//!   __grpcStreamFinish(streamId)
//!   __grpcStreamRead(streamId) -> dataBytes
//!   __grpcStreamCancel(streamId)
//!   __grpcCloseChannel(channelId)

use crate::builtins::v8_compat::{uint8array_from_bytes, uint8array_to_vec};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tonic::transport::{Channel, Endpoint};
use v8::{ContextScope, FunctionCallbackArguments, HandleScope, PinScope, ReturnValue};
use vvva_permissions::{Capability, PermissionState};

struct GrpcStream {
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Vec<u8>>>>,
    cancel_tx: tokio::sync::oneshot::Sender<()>,
}

struct GrpcStreamState {
    streams: Arc<Mutex<HashMap<u32, GrpcStream>>>,
    next_id: Arc<Mutex<u32>>,
}

impl GrpcStreamState {
    fn alloc_id(&self) -> u32 {
        let mut n = self.next_id.lock().unwrap();
        let id = *n;
        *n = n.wrapping_add(1);
        id
    }

    fn insert(&self, stream: GrpcStream) -> u32 {
        let id = self.alloc_id();
        self.streams.lock().unwrap().insert(id, stream);
        id
    }

    fn get(
        &self,
        id: u32,
    ) -> Option<Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Vec<u8>>>>> {
        let guard = self.streams.lock().unwrap();
        guard.get(&id).map(|s| s.rx.clone())
    }

    fn remove(&self, id: u32) -> Option<GrpcStream> {
        self.streams.lock().unwrap().remove(&id)
    }
}

fn js_err<'s>(
    scope: &mut PinScope<'s, '_>,
    code: &str,
    msg: impl AsRef<str>,
) -> v8::Local<'s, v8::Value> {
    let msg = msg.as_ref();
    let src = format!(
        "(function(){{var e=new Error(\"{}\");e.code=\"{}\";return e;}})()",
        msg.replace('\\', "\\\\").replace('"', "\\\""),
        code
    );
    let source = v8::String::new(scope, &src).unwrap();
    v8::Script::compile(scope, source, None)
        .and_then(|s| s.run(scope))
        .unwrap_or_else(|| v8::undefined(scope).into())
}

/// Builds the channel without connecting yet (tonic's `connect_lazy`) —
/// the real TCP/TLS handshake happens lazily on the first RPC instead. This
/// keeps channel creation synchronous, so the native `__grpcCreateChannel`
/// binding never needs `block_in_place` + `Handle::block_on`, which panics
/// outright on a current_thread Tokio runtime (e.g. plain `#[tokio::test]`);
/// see fs_watch's `__fsWatchNext` fix for the same bug pattern. It also
/// matches how most gRPC clients actually behave: channel construction
/// doesn't fail on a down server, only the first call against it does.
fn create_channel_lazy(
    host: String,
    port: u16,
    use_tls: bool,
) -> std::result::Result<Channel, String> {
    let scheme = if use_tls { "https" } else { "http" };
    let addr = format!("{}://{}:{}", scheme, host, port);
    let endpoint = Endpoint::try_from(addr).map_err(|e| format!("Invalid address: {e}"))?;
    Ok(endpoint.connect_lazy())
}

fn parse_package_definition(proto_content: &str) -> std::result::Result<serde_json::Value, String> {
    let mut services = serde_json::Map::new();

    for line in proto_content.lines() {
        let line = line.trim();
        if line.starts_with("service ")
            && let Some(end) = line.find('{')
        {
            let service_def = &line["service ".len()..end];
            let service_name = service_def.split_whitespace().next().unwrap_or("");
            if !service_name.is_empty() {
                let mut service_obj = serde_json::Map::new();
                service_obj.insert(
                    "__type".to_string(),
                    serde_json::Value::String("service".to_string()),
                );
                services.insert(
                    service_name.to_string(),
                    serde_json::Value::Object(service_obj),
                );
            }
        }
    }

    let mut result = serde_json::Map::new();
    result.insert(
        "__package".to_string(),
        serde_json::Value::Object(services.clone()),
    );

    let mut package_obj = serde_json::Map::new();
    for (name, value) in services {
        package_obj.insert(name, value);
    }
    result.insert(
        "__services".to_string(),
        serde_json::Value::Object(package_obj),
    );

    Ok(serde_json::Value::Object(result))
}

fn set_fn(
    scope: &mut ContextScope<HandleScope>,
    global: v8::Local<v8::Object>,
    name: &str,
    f: impl v8::MapFnTo<v8::FunctionCallback>,
) {
    let func = v8::Function::new(scope, f).unwrap();
    let key = v8::String::new(scope, name).unwrap().into();
    global.set(scope, key, func.into());
}

// Thread-local, not a process-wide static — see the identical fix (and
// rationale) in fs.rs's FS_PERMISSIONS: a `OnceLock` here only keeps the
// *first* engine's permissions ever created in the process, so every later
// `JsEngine` (every other test, or a second engine in a long-lived process)
// silently inherits the first one's grants instead of its own.
thread_local! {
    static GRPC_PERMISSIONS: std::cell::RefCell<Option<Arc<PermissionState>>> =
        const { std::cell::RefCell::new(None) };
}
fn permissions() -> Arc<PermissionState> {
    GRPC_PERMISSIONS.with(|p| {
        p.borrow()
            .clone()
            .expect("inject_grpc not called on this thread")
    })
}
static GRPC_CHANNEL_POOL: std::sync::OnceLock<Arc<Mutex<HashMap<u32, Channel>>>> =
    std::sync::OnceLock::new();
fn channel_pool() -> &'static Arc<Mutex<HashMap<u32, Channel>>> {
    GRPC_CHANNEL_POOL.get().unwrap()
}
static GRPC_NEXT_CHANNEL_ID: std::sync::OnceLock<Arc<Mutex<u32>>> = std::sync::OnceLock::new();
fn next_channel_id() -> &'static Arc<Mutex<u32>> {
    GRPC_NEXT_CHANNEL_ID.get().unwrap()
}
static GRPC_STREAM_STATE: std::sync::OnceLock<Arc<GrpcStreamState>> = std::sync::OnceLock::new();
fn stream_state() -> &'static Arc<GrpcStreamState> {
    GRPC_STREAM_STATE.get().unwrap()
}

pub fn inject_grpc(
    scope: &mut ContextScope<HandleScope>,
    permissions_param: Arc<PermissionState>,
) -> anyhow::Result<()> {
    GRPC_PERMISSIONS.with(|p| *p.borrow_mut() = Some(permissions_param));
    GRPC_CHANNEL_POOL
        .set(Arc::new(Mutex::new(HashMap::new())))
        .ok();
    GRPC_NEXT_CHANNEL_ID.set(Arc::new(Mutex::new(0))).ok();
    GRPC_STREAM_STATE
        .set(Arc::new(GrpcStreamState {
            streams: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(Mutex::new(0)),
        }))
        .ok();

    let context = scope.get_current_context();
    let global = context.global(scope);

    set_fn(
        scope,
        global,
        "__grpcLoadProto",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let proto_content = args.get(0).to_rust_string_lossy(scope);
            match parse_package_definition(&proto_content) {
                Ok(v) => rv.set(v8::String::new(scope, &v.to_string()).unwrap().into()),
                Err(e) => rv.set(js_err(scope, "EPARSE", e)),
            }
        },
    );

    set_fn(
        scope,
        global,
        "__grpcCreateChannel",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let host = args.get(0).to_rust_string_lossy(scope);
            let port = args.get(1).uint32_value(scope).unwrap_or(0) as u16;
            let use_tls = args.get(2).boolean_value(scope);

            if !permissions().check(&Capability::Network(host.clone())) {
                let err = js_err(
                    scope,
                    "EACCES",
                    format!("Network access denied. Run with --allow-net={}", host),
                );
                scope.throw_exception(err);
                return;
            }

            let result = create_channel_lazy(host, port, use_tls);

            match result {
                Ok(channel) => {
                    let id = {
                        let mut n = next_channel_id().lock().unwrap();
                        let id = *n;
                        *n = n.wrapping_add(1);
                        id
                    };
                    channel_pool().lock().unwrap().insert(id, channel);
                    rv.set(v8::Integer::new_from_unsigned(scope, id).into());
                }
                Err(e) => {
                    let err = js_err(scope, "ECONNREFUSED", e);
                    scope.throw_exception(err);
                }
            }
        },
    );

    set_fn(
        scope,
        global,
        "__grpcMakeUnaryCall",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let channel_id = args.get(0).uint32_value(scope).unwrap_or(0);
            let request_bytes = v8::Local::<v8::Uint8Array>::try_from(args.get(3))
                .map(|arr| uint8array_to_vec(scope, arr))
                .unwrap_or_default();

            let has_channel = channel_pool().lock().unwrap().contains_key(&channel_id);
            if !has_channel {
                let err = js_err(scope, "ENOENT", "unknown channel");
                scope.throw_exception(err);
                return;
            }

            rv.set(uint8array_from_bytes(scope, &request_bytes).into());
        },
    );

    set_fn(
        scope,
        global,
        "__grpcCreateServerStream",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let channel_id = args.get(0).uint32_value(scope).unwrap_or(0);

            let has_channel = channel_pool().lock().unwrap().contains_key(&channel_id);
            if !has_channel {
                let err = js_err(scope, "ENOENT", "unknown channel");
                scope.throw_exception(err);
                return;
            }

            let (tx, rx) = tokio::sync::mpsc::channel(100);
            let (cancel_tx, _cancel_rx) = tokio::sync::oneshot::channel();
            let grpc_stream = GrpcStream {
                tx,
                rx: Arc::new(tokio::sync::Mutex::new(rx)),
                cancel_tx,
            };
            let stream_id = stream_state().insert(grpc_stream);
            rv.set(v8::Integer::new_from_unsigned(scope, stream_id).into());
        },
    );

    set_fn(
        scope,
        global,
        "__grpcCreateClientStream",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let channel_id = args.get(0).uint32_value(scope).unwrap_or(0);

            let has_channel = channel_pool().lock().unwrap().contains_key(&channel_id);
            if !has_channel {
                let err = js_err(scope, "ENOENT", "unknown channel");
                scope.throw_exception(err);
                return;
            }

            let (tx, rx) = tokio::sync::mpsc::channel(100);
            let (cancel_tx, _cancel_rx) = tokio::sync::oneshot::channel();
            let grpc_stream = GrpcStream {
                tx,
                rx: Arc::new(tokio::sync::Mutex::new(rx)),
                cancel_tx,
            };
            let stream_id = stream_state().insert(grpc_stream);
            rv.set(v8::Integer::new_from_unsigned(scope, stream_id).into());
        },
    );

    set_fn(
        scope,
        global,
        "__grpcStreamWrite",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let stream_id = args.get(0).uint32_value(scope).unwrap_or(0);
            let data_bytes = v8::Local::<v8::Uint8Array>::try_from(args.get(1))
                .map(|arr| uint8array_to_vec(scope, arr))
                .unwrap_or_default();

            let tx = {
                let guard = stream_state().streams.lock().unwrap();
                guard.get(&stream_id).map(|s| s.tx.clone())
            };
            let tx = match tx {
                Some(tx) => tx,
                None => {
                    let err = js_err(scope, "ENOENT", "unknown stream");
                    scope.throw_exception(err);
                    return;
                }
            };

            // try_send, not block_in_place + Handle::block_on — the latter
            // panics outright on a current_thread Tokio runtime (e.g. plain
            // `#[tokio::test]`); see fs_watch's __fsWatchNext fix for the
            // same bug. The channel has a 100-message buffer (see the
            // `mpsc::channel(100)` construction above), so a full channel
            // here means the consumer is badly backed up, not a normal case.
            match tx.try_send(data_bytes) {
                Ok(()) => rv.set(v8::undefined(scope).into()),
                Err(e) => {
                    let err = js_err(scope, "EIO", e.to_string());
                    scope.throw_exception(err);
                }
            }
        },
    );

    set_fn(
        scope,
        global,
        "__grpcStreamFinish",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let stream_id = args.get(0).uint32_value(scope).unwrap_or(0);
            if stream_state().remove(stream_id).is_none() {
                let err = js_err(scope, "ENOENT", "unknown stream");
                scope.throw_exception(err);
                return;
            }
            rv.set(v8::undefined(scope).into());
        },
    );

    set_fn(
        scope,
        global,
        "__grpcStreamRead",
        move |scope: &mut PinScope, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let stream_id = args.get(0).uint32_value(scope).unwrap_or(0);
            let rx = stream_state().get(stream_id);
            let rx = match rx {
                Some(rx) => rx,
                None => {
                    let err = js_err(scope, "ENOENT", "unknown stream");
                    scope.throw_exception(err);
                    return;
                }
            };

            // try_lock + try_recv, not block_in_place + Handle::block_on —
            // the latter panics outright on a current_thread Tokio runtime
            // (e.g. plain `#[tokio::test]`); see fs_watch's __fsWatchNext fix
            // for the same bug pattern. Non-blocking: returns empty bytes
            // when nothing is ready yet, same as when the stream is closed —
            // callers that need to tell those apart should poll on an
            // interval and treat repeated empties as "still open, not done".
            let data = rx
                .try_lock()
                .ok()
                .and_then(|mut guard| guard.try_recv().ok());

            match data {
                Some(data) => rv.set(uint8array_from_bytes(scope, &data).into()),
                None => rv.set(uint8array_from_bytes(scope, &[]).into()),
            }
        },
    );

    set_fn(
        scope,
        global,
        "__grpcStreamCancel",
        move |_scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let stream_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            if let Some(stream) = stream_state().remove(stream_id) {
                let _ = stream.cancel_tx.send(());
            }
        },
    );

    set_fn(
        scope,
        global,
        "__grpcCloseChannel",
        move |_scope: &mut PinScope, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let channel_id = args.get(0).uint32_value(_scope).unwrap_or(0);
            channel_pool().lock().unwrap().remove(&channel_id);
        },
    );

    Ok(())
}
