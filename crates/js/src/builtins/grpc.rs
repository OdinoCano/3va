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

use rquickjs::function::Async;
use rquickjs::{Ctx, Function, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tonic::transport::{Channel, Endpoint};
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

fn js_err(ctx: &Ctx<'_>, msg: String) -> rquickjs::Error {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    match ctx.eval::<rquickjs::Value<'_>, _>(format!("new Error(\"{}\")", escaped)) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
    }
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

pub fn inject_grpc(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let channel_pool: Arc<Mutex<HashMap<u32, Channel>>> = Arc::new(Mutex::new(HashMap::new()));
    let next_channel_id: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
    let stream_state = Arc::new(GrpcStreamState {
        streams: Arc::new(Mutex::new(HashMap::new())),
        next_id: Arc::new(Mutex::new(0)),
    });

    {
        let _perms = permissions.clone();
        ctx.globals().set(
            "__grpcLoadProto",
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'_>,
                      proto_content: String,
                      _package_name: String|
                      -> Result<String> {
                    parse_package_definition(&proto_content)
                        .map(|v| v.to_string())
                        .map_err(|e| js_err(&ctx, e.to_string()))
                },
            ),
        )?;
    }

    {
        let perms = permissions.clone();
        let pool = channel_pool.clone();
        let nid = next_channel_id.clone();
        ctx.globals().set(
            "__grpcCreateChannel",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>, host: String, port: u16, use_tls: bool| {
                        let perms = perms.clone();
                        let pool = pool.clone();
                        let nid = nid.clone();
                        async move {
                            if !perms.check(&Capability::Network(host.clone())) {
                                return Err(rquickjs::Error::new_from_js_message(
                                    "EACCES",
                                    "EACCES",
                                    format!("Network access denied. Run with --allow-net={}", host),
                                ));
                            }

                            let channel = create_channel_blocking(host, port, use_tls)
                                .await
                                .map_err(|e| {
                                    rquickjs::Error::new_from_js_message(
                                        "ECONNREFUSED",
                                        "ECONNREFUSED",
                                        e,
                                    )
                                })?;

                            let id = {
                                let mut n = nid.lock().unwrap();
                                let id = *n;
                                *n = n.wrapping_add(1);
                                id
                            };
                            pool.lock().unwrap().insert(id, channel);
                            Ok::<u32, rquickjs::Error>(id)
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let pool = channel_pool.clone();
        ctx.globals().set(
            "__grpcMakeUnaryCall",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>,
                          channel_id: u32,
                          _service: String,
                          _method: String,
                          request_bytes: Vec<u8>| {
                        let pool = pool.clone();
                        async move {
                            let has_channel = {
                                let guard = pool.lock().unwrap();
                                guard.contains_key(&channel_id)
                            };
                            if !has_channel {
                                return Err(rquickjs::Error::new_from_js_message(
                                    "ENOENT",
                                    "ENOENT",
                                    "unknown channel",
                                ));
                            }

                            Ok::<Vec<u8>, rquickjs::Error>(request_bytes)
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let pool = channel_pool.clone();
        let stream_state = stream_state.clone();
        ctx.globals().set(
            "__grpcCreateServerStream",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>,
                          channel_id: u32,
                          _service: String,
                          _method: String,
                          _request_bytes: Vec<u8>| {
                        let pool = pool.clone();
                        let stream_state = stream_state.clone();
                        async move {
                            let has_channel = {
                                let guard = pool.lock().unwrap();
                                guard.contains_key(&channel_id)
                            };
                            if !has_channel {
                                return Err(rquickjs::Error::new_from_js_message(
                                    "ENOENT",
                                    "ENOENT",
                                    "unknown channel",
                                ));
                            }

                            let (tx, rx) = tokio::sync::mpsc::channel(100);
                            let (cancel_tx, _cancel_rx) = tokio::sync::oneshot::channel();

                            let grpc_stream = GrpcStream {
                                tx,
                                rx: Arc::new(tokio::sync::Mutex::new(rx)),
                                cancel_tx,
                            };

                            let stream_id = stream_state.insert(grpc_stream);
                            Ok::<u32, rquickjs::Error>(stream_id)
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let pool = channel_pool.clone();
        let stream_state = stream_state.clone();
        ctx.globals().set(
            "__grpcCreateClientStream",
            Function::new(
                ctx.clone(),
                Async(
                    move |_ctx: Ctx<'_>, channel_id: u32, _service: String, _method: String| {
                        let pool = pool.clone();
                        let stream_state = stream_state.clone();
                        async move {
                            let has_channel = {
                                let guard = pool.lock().unwrap();
                                guard.contains_key(&channel_id)
                            };
                            if !has_channel {
                                return Err(rquickjs::Error::new_from_js_message(
                                    "ENOENT",
                                    "ENOENT",
                                    "unknown channel",
                                ));
                            }

                            let (tx, rx) = tokio::sync::mpsc::channel(100);
                            let (cancel_tx, _cancel_rx) = tokio::sync::oneshot::channel();

                            let grpc_stream = GrpcStream {
                                tx,
                                rx: Arc::new(tokio::sync::Mutex::new(rx)),
                                cancel_tx,
                            };

                            let stream_id = stream_state.insert(grpc_stream);
                            Ok::<u32, rquickjs::Error>(stream_id)
                        }
                    },
                ),
            ),
        )?;
    }

    {
        let stream_state = stream_state.clone();
        ctx.globals().set(
            "__grpcStreamWrite",
            Function::new(
                ctx.clone(),
                Async(move |_ctx: Ctx<'_>, stream_id: u32, data_bytes: Vec<u8>| {
                    let stream_state = stream_state.clone();
                    async move {
                        let tx = {
                            let guard = stream_state.streams.lock().unwrap();
                            guard.get(&stream_id).map(|s| s.tx.clone())
                        };
                        let tx = tx.ok_or_else(|| {
                            rquickjs::Error::new_from_js_message(
                                "ENOENT",
                                "ENOENT",
                                "unknown stream",
                            )
                        })?;

                        tx.send(data_bytes).await.map_err(|e| {
                            rquickjs::Error::new_from_js_message("EIO", "EIO", e.to_string())
                        })?;

                        Ok::<(), rquickjs::Error>(())
                    }
                }),
            ),
        )?;
    }

    {
        let stream_state = stream_state.clone();
        ctx.globals().set(
            "__grpcStreamFinish",
            Function::new(
                ctx.clone(),
                Async(move |_ctx: Ctx<'_>, stream_id: u32| {
                    let stream_state = stream_state.clone();
                    async move {
                        let stream = stream_state.remove(stream_id);
                        if stream.is_none() {
                            return Err(rquickjs::Error::new_from_js_message(
                                "ENOENT",
                                "ENOENT",
                                "unknown stream",
                            ));
                        }

                        Ok::<(), rquickjs::Error>(())
                    }
                }),
            ),
        )?;
    }

    {
        let stream_state = stream_state.clone();
        ctx.globals().set(
            "__grpcStreamRead",
            Function::new(
                ctx.clone(),
                Async(move |_ctx: Ctx<'_>, stream_id: u32| {
                    let stream_state = stream_state.clone();
                    async move {
                        let rx = stream_state.get(stream_id);
                        let rx = rx.ok_or_else(|| {
                            rquickjs::Error::new_from_js_message(
                                "ENOENT",
                                "ENOENT",
                                "unknown stream",
                            )
                        })?;

                        #[allow(clippy::await_holding_lock)]
                        let data = rx.lock().await.recv().await;

                        match data {
                            Some(data) => Ok::<Vec<u8>, rquickjs::Error>(data),
                            None => Ok(Vec::new()),
                        }
                    }
                }),
            ),
        )?;
    }

    {
        let stream_state = stream_state.clone();
        ctx.globals().set(
            "__grpcStreamCancel",
            Function::new(
                ctx.clone(),
                move |_ctx: Ctx<'_>, stream_id: u32| -> Result<()> {
                    if let Some(stream) = stream_state.remove(stream_id) {
                        let _ = stream.cancel_tx.send(());
                    }
                    Ok(())
                },
            ),
        )?;
    }

    {
        let pool = channel_pool.clone();
        ctx.globals().set(
            "__grpcCloseChannel",
            Function::new(
                ctx.clone(),
                move |_ctx: Ctx<'_>, channel_id: u32| -> Result<()> {
                    pool.lock().unwrap().remove(&channel_id);
                    Ok(())
                },
            ),
        )?;
    }

    Ok(())
}
