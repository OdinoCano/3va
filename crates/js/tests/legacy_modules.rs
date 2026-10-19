// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

//! Los módulos heredados `_http_*` y `_stream_*` que Node 24 sigue
//! publicando (deprecados, pero públicos) y que 3va expone como alias de lo
//! que ya implementa.
//!
//! Ejecutar: cargo test -p vvva_js --test legacy_modules

use std::sync::Arc;
use tempfile::TempDir;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn eval_in_dir(dir: &TempDir, code: &str) -> String {
    let path = dir.path().join("check.js");
    std::fs::write(&path, code).unwrap();
    let state = PermissionState::new();
    state.grant(Capability::FileRead(dir.path().to_path_buf()));
    let mut engine = JsEngine::new(Arc::new(state)).await.unwrap();
    engine.eval_file(&path).await.unwrap();
    engine
        .eval_to_string("String(globalThis._result)")
        .await
        .unwrap()
}

#[tokio::test]
async fn the_stream_aliases_are_the_stream_classes() {
    let temp = TempDir::new().unwrap();
    let result = eval_in_dir(
        &temp,
        r#"
        const stream = require('stream');
        globalThis._result = [
            stream.Readable === require('_stream_readable'),
            stream.Writable === require('_stream_writable'),
            stream.Duplex === require('_stream_duplex'),
            stream.Transform === require('_stream_transform'),
            stream.PassThrough === require('_stream_passthrough')
        ].join(',');
        "#,
    )
    .await;
    assert_eq!(result, "true,true,true,true,true");
}

#[tokio::test]
async fn http_common_exposes_the_methods_and_the_header_checks() {
    let temp = TempDir::new().unwrap();
    let result = eval_in_dir(
        &temp,
        r#"
        const httpCommon = require('_http_common');
        const http = require('http');
        globalThis._result = [
            httpCommon.methods === http.METHODS,
            httpCommon._checkIsHttpToken('Content-Type'),
            httpCommon._checkIsHttpToken('Content Type') === false,
            httpCommon._checkInvalidHeaderChar('foo bar') === false,
            httpCommon._checkInvalidHeaderChar('foo\u0000bar'),
            httpCommon._checkInvalidHeaderChar('foo\u0008bar', true) === false,
            httpCommon._checkInvalidHeaderChar('foo\rbar', true)
        ].join(',');
        "#,
    )
    .await;
    assert_eq!(
        result, "true,true,true,true,true,true,true",
        "los alias de `_http_common` deben comportarse como los de Node"
    );
}

#[tokio::test]
async fn the_http_aliases_point_at_the_http_module() {
    let temp = TempDir::new().unwrap();
    let result = eval_in_dir(
        &temp,
        r#"
        const http = require('http');
        globalThis._result = [
            require('_http_agent').Agent === http.Agent,
            require('_http_client').ClientRequest === http.ClientRequest,
            require('_http_incoming').IncomingMessage === http.IncomingMessage,
            require('_http_outgoing').OutgoingMessage === http.OutgoingMessage,
            require('_http_server').Server === http.Server,
            require('_http_server').kConnectionsCheckingInterval === 30000,
            require('_http_outgoing').kHighWaterMark === 16384
        ].join(',');
        "#,
    )
    .await;
    assert_eq!(result, "true,true,true,true,true,true,true");
}
