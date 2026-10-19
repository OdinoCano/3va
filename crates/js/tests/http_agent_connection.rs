// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

// The HTTP request must go through the agent's `createConnection()`.
//
// The proxy agents (https-proxy-agent, http-proxy-agent, tunnel,
// global-agent, the `httpsAgent` of axios) replace it: the connection they
// build is the one the request has to use. If 3va ignores it, the request
// goes straight to the origin and the proxy is bypassed — in a corporate
// network the request then fails, and the exit proxy is skipped.
//
// The test uses a factory that answers the request itself, so the request
// can only succeed if the factory was used.
//
// Run: cargo test -p vvva_js --test http_agent_connection

use std::sync::Arc;
use vvva_js::JsEngine;
use vvva_permissions::{Capability, PermissionState};

async fn engine() -> JsEngine {
    let perms = PermissionState::new();
    perms.grant(Capability::Network("127.0.0.1".to_string()));
    JsEngine::new(Arc::new(perms)).await.unwrap()
}

/// Drive the JS event loop until `globalThis.__done === true` or the deadline.
async fn wait_done(e: &mut JsEngine, secs: u64) -> String {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        if e.eval_to_string("String(globalThis.__done === true)")
            .await
            .unwrap_or_default()
            == "true"
        {
            return e
                .eval_to_string("String(globalThis.__result)")
                .await
                .unwrap_or_default();
        }
        if tokio::time::Instant::now() >= deadline {
            let err = e
                .eval_to_string("String(globalThis.__err)")
                .await
                .unwrap_or_default();
            panic!("JS never finished; __err={err:?}");
        }
        let _ = e.run_event_loop().await;
        tokio::task::yield_now().await;
    }
}

/// Run `setup` and return the value it left in `globalThis.__result`.
async fn eval_async_result(e: &mut JsEngine, setup: &str) -> String {
    e.eval_to_string(setup).await.unwrap();
    wait_done(e, 10).await
}

/// The factory of the fake agent: a Duplex that answers the request itself.
const FAKE_AGENT: &str = r#"
    const http = require('http');
    const { Duplex } = require('stream');
    class FakeAgent extends http.Agent {
        createConnection() {
            const s = new Duplex();
            let once = false;
            s._read = function() {
                if (once) return this.push(null);
                once = true;
                this.push('HTTP/1.1 200 Ok\r\nTransfer-Encoding: chunked\r\n\r\n');
                this.push('b\r\nhello world\r\n');
                this.readable = false;
                this.push('0\r\n\r\n');
            };
            // Blackhole: the request is written and nothing answers.
            s._write = function(data, enc, cb) { cb(); };
            s.destroy = s.destroySoon = function() { this.writable = false; };
            return s;
        }
    }
"#;

#[tokio::test]
async fn the_request_goes_through_the_agents_create_connection() {
    let mut e = engine().await;
    let code = FAKE_AGENT.to_string()
        + r#"
            let received = '';
            const req = http.request({ agent: new FakeAgent() }, (res) => {
                res.on('data', (c) => { received += c; });
                res.on('end', () => { globalThis.__result = received; globalThis.__done = true; });
            });
            req.end();
        "#;
    let out = eval_async_result(&mut e, &code).await;
    assert_eq!(out, "hello world");
}

#[tokio::test]
async fn options_create_connection_replaces_the_agents_one() {
    let mut e = engine().await;
    let code = FAKE_AGENT.to_string()
        + r#"
            let received = '';
            const req = http.request({
                agent: new FakeAgent(),
                createConnection() {
                    const s = new Duplex();
                    let once = false;
                    s._read = function() {
                        if (once) return this.push(null);
                        once = true;
                        this.push('HTTP/1.1 200 Ok\r\nContent-Length: 2\r\n\r\nok');
                    };
                    s._write = function(data, enc, cb) { cb(); };
                    s.destroy = s.destroySoon = function() { this.writable = false; };
                    return s;
                },
            }, (res) => {
                res.on('data', (c) => { received += c; });
                res.on('end', () => { globalThis.__result = received; globalThis.__done = true; });
            });
            req.end();
        "#;
    let out = eval_async_result(&mut e, &code).await;
    assert_eq!(out, "ok");
}

#[tokio::test]
async fn the_factory_gets_the_options_of_the_request() {
    // The factory has to know where to connect and how: the TLS options of
    // the request included, so a proxy agent can build the tunnel.
    let mut e = engine().await;
    let out = eval_async_result(
        &mut e,
        r#"
        const http = require('http');
        const { Duplex } = require('stream');
        let seen = null;
        const agent = new http.Agent();
        agent.createConnection = function(options, cb) {
            seen = options;
            const s = new Duplex();
            s._read = function() { this.push(null); };
            s._write = function(data, enc, c) { c(); };
            s.destroy = s.destroySoon = function() { this.writable = false; };
            if (cb) cb(null, s);
            return s;
        };
        const req = http.request({
            agent,
            host: 'example.test',
            port: 8443,
            path: '/hello',
            method: 'POST',
            ca: '-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n',
            cert: 'CLIENT-CERT',
            key: 'CLIENT-KEY',
            rejectUnauthorized: false,
            servername: 'sni.test',
        });
        req.on('error', () => {});
        req.end();
        globalThis.__done = false;
        setTimeout(() => {
            globalThis.__result = JSON.stringify({
                host: seen && seen.host,
                port: seen && seen.port,
                path: seen && seen.path,
                method: seen && seen.method,
                ca: seen && seen.ca,
                cert: seen && seen.cert,
                key: seen && seen.key,
                rejectUnauthorized: seen && seen.rejectUnauthorized,
                servername: seen && seen.servername,
            });
            globalThis.__done = true;
        }, 20);
    "#,
    )
    .await;
    assert!(out.contains("\"host\":\"example.test\""), "{out}");
    assert!(out.contains("\"port\":8443"), "{out}");
    assert!(out.contains("\"path\":\"/hello\""), "{out}");
    assert!(out.contains("\"method\":\"POST\""), "{out}");
    assert!(out.contains("BEGIN CERTIFICATE"), "{out}");
    assert!(out.contains("\"cert\":\"CLIENT-CERT\""), "{out}");
    assert!(out.contains("\"key\":\"CLIENT-KEY\""), "{out}");
    assert!(out.contains("\"rejectUnauthorized\":false"), "{out}");
    assert!(out.contains("\"servername\":\"sni.test\""), "{out}");
}

#[tokio::test]
async fn an_agent_without_new_still_works() {
    // Node allows `http.Agent()` and `http.Server(fn)` without `new`.
    let mut e = engine().await;
    let out = eval_async_result(
        &mut e,
        r#"
        const http = require('http');
        const agent = http.Agent();
        const server = http.Server();
        globalThis.__done = true;
        globalThis.__result = (agent instanceof http.Agent) + ',' + (server instanceof http.Server);
    "#,
    )
    .await;
    assert_eq!(out, "true,true");
}

#[tokio::test]
async fn detach_socket_takes_back_its_close_listener() {
    // `assignSocket` adds a 'close' listener to the socket; `detachSocket`
    // has to take it back out, or the response keeps reacting to a socket it
    // no longer belongs to.
    let mut e = engine().await;
    let out = eval_async_result(
        &mut e,
        r#"
        const http = require('http');
        const { EventEmitter } = require('events');
        const res = new http.ServerResponse({ socket: {} });
        const socket = new EventEmitter();
        const before = socket.listenerCount('close');
        res.assignSocket(socket);
        const during = socket.listenerCount('close');
        res.detachSocket(socket);
        const after = socket.listenerCount('close');
        globalThis.__done = true;
        globalThis.__result = [before, during, after].join(',');
    "#,
    )
    .await;
    assert_eq!(out, "0,1,0");
}
