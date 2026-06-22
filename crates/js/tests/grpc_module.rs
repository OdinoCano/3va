// Tests for the gRPC builtin.
// Run: cargo test -p vvva_js --test grpc_module

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine_with_net(host: &str) -> JsEngine {
    let state = PermissionState::new();
    state.grant(Capability::Network(host.to_string()));
    JsEngine::new(Arc::new(state)).await.unwrap()
}

async fn engine_no_net() -> JsEngine {
    JsEngine::new(Arc::new(PermissionState::new()))
        .await
        .unwrap()
}

async fn wait_for_result(engine: &JsEngine, global: &str) -> String {
    for _ in 0..40 {
        engine.idle().await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let v = engine
            .eval_to_string(&format!("globalThis.{global} || ''"))
            .await
            .unwrap_or_default();
        if !v.is_empty() {
            return v;
        }
    }
    String::new()
}

// ── API shape ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn grpc_globals_exist() {
    let e = engine_no_net().await;
    let r = e.eval_to_string("typeof __grpcLoadProto").await.unwrap();
    assert_eq!(r, "function", "grpcLoadProto should be a function");

    let r = e
        .eval_to_string("typeof __grpcCreateChannel")
        .await
        .unwrap();
    assert_eq!(r, "function", "grpcCreateChannel should be a function");

    let r = e
        .eval_to_string("typeof __grpcMakeUnaryCall")
        .await
        .unwrap();
    assert_eq!(r, "function", "grpcMakeUnaryCall should be a function");

    let r = e
        .eval_to_string("typeof __grpcCreateServerStream")
        .await
        .unwrap();
    assert_eq!(r, "function", "grpcCreateServerStream should be a function");

    let r = e
        .eval_to_string("typeof __grpcCreateClientStream")
        .await
        .unwrap();
    assert_eq!(r, "function", "grpcCreateClientStream should be a function");

    let r = e.eval_to_string("typeof __grpcStreamWrite").await.unwrap();
    assert_eq!(r, "function", "grpcStreamWrite should be a function");

    let r = e.eval_to_string("typeof __grpcStreamFinish").await.unwrap();
    assert_eq!(r, "function", "grpcStreamFinish should be a function");

    let r = e.eval_to_string("typeof __grpcStreamRead").await.unwrap();
    assert_eq!(r, "function", "grpcStreamRead should be a function");

    let r = e.eval_to_string("typeof __grpcStreamCancel").await.unwrap();
    assert_eq!(r, "function", "grpcStreamCancel should be a function");

    let r = e.eval_to_string("typeof __grpcCloseChannel").await.unwrap();
    assert_eq!(r, "function", "grpcCloseChannel should be a function");
}

// ── Permission enforcement ───────────────────────────────────────────────────

#[tokio::test]
async fn grpc_channel_denied_without_permission() {
    let e = engine_no_net().await;
    e.eval(
        r#"globalThis.__grpcResult = null;
           (async function() {
               try {
                   await __grpcCreateChannel('localhost', 50051, false);
                   globalThis.__grpcResult = 'no_throw';
               } catch(e) {
                   globalThis.__grpcResult = 'threw:' + (e.message || String(e));
               }
           })();"#,
    )
    .await
    .unwrap();
    let result = wait_for_result(&e, "__grpcResult").await;
    assert!(
        result.contains("threw:") && result.contains("Network access denied"),
        "gRPC channel should be denied without permission, got: {}",
        result
    );
}

// ── Proto parsing ────────────────────────────────────────────────────────────

#[tokio::test]
async fn grpc_load_proto_parses_service() {
    let e = engine_no_net().await;
    let proto = r#"
        syntax = "proto3";
        package mypackage;

        service MyService {
            rpc MyMethod(MyRequest) returns (MyResponse);
        }

        message MyRequest {
            string field = 1;
        }

        message MyResponse {
            string field = 1;
        }
    "#;

    let r = e
        .eval_to_string(&format!(
            r#"(function() {{
                const result = __grpcLoadProto(`{}`, 'mypackage');
                return result ? 'ok' : 'failed';
            }})()"#,
            proto.replace('\n', "\\n")
        ))
        .await
        .unwrap();
    assert_eq!(r, "ok", "proto parsing should succeed");
}

// ── Channel lifecycle ───────────────────────────────────────────────────────

#[tokio::test]
async fn grpc_create_channel_returns_promise() {
    let e = engine_with_net("localhost").await;
    e.eval(
        r#"globalThis.__grpcResult = null;
           (async function() {
               try {
                   const channelId = await __grpcCreateChannel('localhost', 50051, false);
                   globalThis.__grpcResult = 'channel_created:' + typeof channelId;
                   __grpcCloseChannel(channelId);
               } catch(e) {
                   globalThis.__grpcResult = 'error:' + e.message;
               }
           })();"#,
    )
    .await
    .unwrap();
    let result = wait_for_result(&e, "__grpcResult").await;
    // Should either create channel successfully (returns number) or fail with connection error
    assert!(
        result.starts_with("channel_created:") || result.starts_with("error:"),
        "Unexpected result: {}",
        result
    );
}

// ── Invalid operations ──────────────────────────────────────────────────────

#[tokio::test]
async fn grpc_invalid_channel_operations() {
    let e = engine_with_net("localhost").await;

    e.eval(
        r#"globalThis.__grpcResult = null;
           (async function() {
               try {
                   await __grpcMakeUnaryCall(9999, 'MyService', 'MyMethod', new Uint8Array([1, 2, 3]));
                   globalThis.__grpcResult = 'no_error';
               } catch(e) {
                   globalThis.__grpcResult = 'got_error:' + (e.code || e.message || String(e));
               }
           })();"#,
    )
    .await
    .unwrap();
    let result = wait_for_result(&e, "__grpcResult").await;
    assert!(
        result.starts_with("got_error:"),
        "invalid channel should error, got: {}",
        result
    );

    e.eval(
        r#"globalThis.__grpcResult = null;
           (async function() {
               try {
                   await __grpcStreamRead(9999);
                   globalThis.__grpcResult = 'no_error';
               } catch(e) {
                   globalThis.__grpcResult = 'got_error:' + (e.code || e.message || String(e));
               }
           })();"#,
    )
    .await
    .unwrap();
    let result = wait_for_result(&e, "__grpcResult").await;
    assert!(
        result.starts_with("got_error:"),
        "invalid stream read should error, got: {}",
        result
    );

    e.eval(
        r#"globalThis.__grpcResult = null;
           (async function() {
               try {
                   await __grpcStreamWrite(9999, new Uint8Array([1, 2, 3]));
                   globalThis.__grpcResult = 'no_error';
               } catch(e) {
                   globalThis.__grpcResult = 'got_error:' + (e.code || e.message || String(e));
               }
           })();"#,
    )
    .await
    .unwrap();
    let result = wait_for_result(&e, "__grpcResult").await;
    assert!(
        result.starts_with("got_error:"),
        "invalid stream write should error, got: {}",
        result
    );
}

// ── Package definition format ────────────────────────────────────────────────

#[tokio::test]
async fn grpc_package_definition_format() {
    let e = engine_no_net().await;
    let proto = r#"
        syntax = "proto3";
        package testpkg;

        service TestService {
            rpc UnaryCall(Request) returns (Response);
            rpc ServerStream(Request) returns (stream Response);
            rpc ClientStream(stream Request) returns (Response);
            rpc BidiStream(stream Request) returns (stream Response);
        }
    "#;

    let r = e
        .eval_to_string(&format!(
            r#"(function() {{
                const result = __grpcLoadProto(`{}`, 'testpkg');
                if (!result) return 'null_result';
                try {{
                    const parsed = JSON.parse(result);
                    return 'ok';
                }} catch(e) {{
                    return 'parse_error:' + e.message;
                }}
            }})()"#,
            proto.replace('\n', "\\n")
        ))
        .await
        .unwrap();
    assert_eq!(r, "ok", "package definition should be valid JSON");
}

// ── Stream operations ─────────────────────────────────────────────────────────

#[tokio::test]
async fn grpc_stream_without_channel_fails() {
    let e = engine_with_net("localhost").await;
    e.eval(
        r#"globalThis.__grpcResult = null;
           (async function() {
               try {
                   // Try to create stream without creating a channel first
                   const streamId = await __grpcCreateServerStream(9999, 'MyService', 'MyMethod', new Uint8Array([1]));
                   globalThis.__grpcResult = 'stream_created:' + streamId;
               } catch(e) {
                   globalThis.__grpcResult = 'got_error:' + (e.code || e.message || String(e));
               }
           })();"#,
    )
    .await
    .unwrap();
    let result = wait_for_result(&e, "__grpcResult").await;
    assert!(
        result.starts_with("got_error:"),
        "stream without channel should error, got: {}",
        result
    );
}

#[tokio::test]
async fn grpc_close_nonexistent_channel_succeeds() {
    let e = engine_with_net("localhost").await;
    // Closing a non-existent channel should be a no-op (not throw)
    let r = e
        .eval_to_string(
            r#"(function() {
                try {
                    __grpcCloseChannel(9999);
                    return 'ok';
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()"#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok", "closing nonexistent channel should not throw");
}

#[tokio::test]
async fn grpc_cancel_nonexistent_stream_succeeds() {
    let e = engine_with_net("localhost").await;
    // Canceling a non-existent stream should be a no-op (not throw)
    let r = e
        .eval_to_string(
            r#"(function() {
                try {
                    __grpcStreamCancel(9999);
                    return 'ok';
                } catch(e) {
                    return 'error:' + e.message;
                }
            })()"#,
        )
        .await
        .unwrap();
    assert_eq!(r, "ok", "canceling nonexistent stream should not throw");
}
