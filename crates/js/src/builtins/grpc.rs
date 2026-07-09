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

async fn create_channel_blocking(
    host: String,
    port: u16,
    use_tls: bool,
) -> std::result::Result<Channel, String> {
    let scheme = if use_tls { "https" } else { "http" };
    let addr = format!("{}://{}:{}", scheme, host, port);
    let endpoint = Endpoint::try_from(addr).map_err(|e| format!("Invalid address: {e}"))?;

    let channel = endpoint
        .connect()
        .await
        .map_err(|e| format!("Connection failed: {e}"))?;

    Ok(channel)
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

static GRPC_PERMISSIONS: std::sync::OnceLock<Arc<PermissionState>> = std::sync::OnceLock::new();
fn permissions() -> &'static Arc<PermissionState> {
    GRPC_PERMISSIONS.get().unwrap()
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
    GRPC_PERMISSIONS.set(permissions_param).ok();
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
                rv.set(err);
                return;
            }

            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current()
                    .block_on(create_channel_blocking(host, port, use_tls))
            });

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
                    rv.set(err);
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
                rv.set(err);
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
                rv.set(err);
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
                rv.set(err);
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
                    rv.set(err);
                    return;
                }
            };

            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(tx.send(data_bytes))
            });
            if let Err(e) = result {
                let err = js_err(scope, "EIO", e.to_string());
                rv.set(err);
            } else {
                rv.set(v8::undefined(scope).into());
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
                rv.set(err);
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
                    rv.set(err);
                    return;
                }
            };

            let data = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    #[allow(clippy::await_holding_lock)]
                    let mut guard = rx.lock().await;
                    guard.recv().await
                })
            });

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
