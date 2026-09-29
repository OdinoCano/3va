// SPDX-License-Identifier: MIT
// Copyright (c) 3va contributors

use base64::Engine as _;
use std::io::Read;
use std::sync::Arc;
use v8::{Function, FunctionCallbackArguments, HandleScope, PinScope, ReturnValue, Script};
use vvva_permissions::{Capability, PermissionState};

/// Hard ceiling on how many response-body bytes a single `fetch()` may buffer.
///
/// The body is read fully into memory before it reaches JS, so without a cap
/// one malicious/compromised server could exhaust the runtime's memory with a
/// single oversized (or unbounded, chunked) response. Mirrors undici's
/// `maxResponseSize` dispatcher option; scripts can lower (not raise) it per
/// call via `fetch(url, { maxResponseSize })`.
const MAX_RESPONSE_BODY_BYTES: u64 = 512 * 1024 * 1024;

pub(crate) fn host_from_url(url: &str) -> Option<String> {
    destination_from_url(url).map(|(host, _)| host)
}

/// The destination a URL names, as `host` and an effective port.
///
/// The port matters to permission checks: the request is authorised as
/// `host:port` so that a grant of `api.example.com:443` covers exactly the
/// HTTPS service and not an admin port on the same host. The scheme's default
/// port is filled in when the URL omits it, so `https://api.example.com/` and
/// `https://api.example.com:443/` ask the same question.
pub(crate) fn destination_from_url(url: &str) -> Option<(String, u16)> {
    // WHATWG parser, the same one ureq connects with: a hand-rolled split
    // disagreed on `\` (`http://127.0.0.1\@granted.host/` was checked as
    // `granted.host` but connected to 127.0.0.1).
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https" | "ws" | "wss") {
        return None;
    }
    let host = match parsed.host()? {
        url::Host::Ipv6(addr) => addr.to_string(),
        h => h.to_string(),
    };
    Some((host, parsed.port_or_known_default()?))
}

/// Canonical WHATWG serialization of `url`. Callers check permissions and
/// connect on this string, so the checked and the dialed destination can't
/// diverge.
pub(crate) fn canonical_url(url: &str) -> Option<String> {
    url::Url::parse(url).ok().map(String::from)
}

/// A ureq resolver that connects only to addresses no `--deny-net` rule names
/// by IP, resolved once (VULN-18).
pub(crate) fn vetted_resolver(
    perms: Arc<PermissionState>,
) -> impl Fn(&str) -> std::io::Result<Vec<std::net::SocketAddr>> + Send + Sync + 'static {
    move |netloc: &str| {
        let (host, port) = netloc
            .rsplit_once(':')
            .and_then(|(h, p)| Some((h, p.parse::<u16>().ok()?)))
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("bad address {netloc}"),
                )
            })?;
        perms.vetted_addrs(host, port)
    }
}

/// Streams `reader` into memory enforcing `cap` after every chunk read, so an
/// oversized or unbounded response is aborted mid-transfer instead of being
/// buffered to completion.
fn read_body_bounded(mut reader: impl Read, cap: u64) -> anyhow::Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    let mut buf = [0u8; 65_536];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..n]);
        if out.len() as u64 > cap {
            anyhow::bail!(
                "fetch failed: response body exceeded the maximum response size of {} bytes",
                cap
            );
        }
    }
}

fn response_to_json(r: ureq::Response, cap: u64) -> anyhow::Result<serde_json::Value> {
    let status = r.status();
    let status_text = r.status_text().to_string();
    let ok = (200u16..300).contains(&status);
    let mut resp_hdrs = serde_json::Map::new();
    for name in r.headers_names() {
        if let Some(val) = r.header(&name) {
            resp_hdrs.insert(name, serde_json::Value::String(val.to_string()));
        }
    }
    // Fast path: trust Content-Length only to reject early — the streaming
    // reader below still counts actual bytes, so a lying header cannot bypass
    // the cap.
    if let Some(cl) = r.header("content-length")
        && let Ok(n) = cl.trim().parse::<u64>()
        && n > cap
    {
        anyhow::bail!(
            "fetch failed: response Content-Length ({}) exceeds the maximum response size of {} bytes",
            n,
            cap
        );
    }
    let body_bytes = read_body_bounded(r.into_reader(), cap)?;
    let (body_val, binary) = match String::from_utf8(body_bytes.clone()) {
        Ok(s) => (serde_json::Value::String(s), false),
        Err(_) => (
            serde_json::Value::String(
                base64::engine::general_purpose::STANDARD.encode(&body_bytes),
            ),
            true,
        ),
    };
    Ok(serde_json::json!({
        "ok": ok, "status": status, "statusText": status_text,
        "headers": resp_hdrs, "body": body_val, "binary": binary,
    }))
}

fn do_request(
    url: String,
    method: String,
    hdrs_json: String,
    body: Option<String>,
    max_response_size: Option<u64>,
) -> anyhow::Result<String> {
    let extra_val: serde_json::Value =
        serde_json::from_str(&hdrs_json).unwrap_or(serde_json::Value::Object(Default::default()));
    let cap = max_response_size.unwrap_or(MAX_RESPONSE_BODY_BYTES);

    let agent = super::tls::agent_builder()
        .redirects(0)
        .resolver(vetted_resolver(permissions()))
        .build();
    let mut req = agent.request(&method, &url);

    if let Some(obj) = extra_val.as_object() {
        for (k, v) in obj {
            let s = match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            req = req.set(k, &s);
        }
    }
    req = req.set("User-Agent", "3va/0.1");

    let resp_result = if let Some(b) = body {
        req.send_string(&b)
    } else {
        req.call()
    };

    let json = match resp_result {
        Ok(r) => response_to_json(r, cap)?,
        Err(ureq::Error::Status(_, r)) => response_to_json(r, cap)?,
        Err(e) => return Err(anyhow::anyhow!("fetch failed: {}", e)),
    };

    Ok(json.to_string())
}

// Per thread, like the other builtins: a process-wide OnceLock kept the first
// engine's PermissionState forever, so later engines (workers, tests) had
// their fetch() checked against someone else's grants.
thread_local! {
    static INJECT_FETCH_PERMISSIONS: std::cell::RefCell<Option<Arc<PermissionState>>> =
        const { std::cell::RefCell::new(None) };
}
fn permissions() -> Arc<PermissionState> {
    INJECT_FETCH_PERMISSIONS.with(|p| {
        p.borrow()
            .clone()
            .expect("inject_fetch not called on this thread")
    })
}

pub fn inject_fetch(
    scope: &mut v8::ContextScope<HandleScope>,
    permissions_param: Arc<PermissionState>,
) -> anyhow::Result<()> {
    INJECT_FETCH_PERMISSIONS.with(|p| *p.borrow_mut() = Some(permissions_param));

    let native_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let Some(url) = canonical_url(&args.get(0).to_rust_string_lossy(scope)) else {
                let msg = v8::String::new(scope, "Invalid URL").unwrap();
                let err = v8::Exception::type_error(scope, msg);
                scope.throw_exception(err);
                return;
            };
            let method = args.get(1).to_rust_string_lossy(scope);
            let hdrs_json = args.get(2).to_rust_string_lossy(scope);
            let body_opt = if args.get(3).is_undefined() || args.get(3).is_null() {
                None
            } else {
                Some(args.get(3).to_rust_string_lossy(scope))
            };
            // Optional per-call cap (undici names this `maxResponseSize`).
            // Only lowering the default is honored: a script cannot raise the
            // runtime-wide ceiling through this option.
            let max_response_size = {
                let v = args.get(4);
                if v.is_number() {
                    let n = v.number_value(scope).unwrap_or(f64::NAN);
                    if n.is_finite() && n >= 0.0 {
                        Some(n as u64)
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            let host = match host_from_url(&url) {
                Some(h) => h,
                None => {
                    let msg = v8::String::new(scope, "Invalid URL").unwrap();
                    let err = v8::Exception::type_error(scope, msg);
                    scope.throw_exception(err);
                    return;
                }
            };

            // `authority` brackets IPv6 literals: an unbracketed `::1:443`
            // cannot be split back into an address and a port, so a
            // port-scoped grant would never match it.
            let destination =
                destination_from_url(&url).map(|(h, p)| vvva_permissions::authority(&h, p));
            if !permissions().check(&Capability::Network(
                destination.clone().unwrap_or_else(|| host.clone()),
            )) {
                let msg = format!(
                    "Network access denied. Run with --allow-net={}",
                    destination.as_deref().unwrap_or(&host)
                );
                let msg = v8::String::new(scope, &msg).unwrap();
                let err = v8::Exception::error(scope, msg);
                scope.throw_exception(err);
                return;
            }
            if url
                .get(..7)
                .is_some_and(|s| s.eq_ignore_ascii_case("http://"))
                && !vvva_permissions::plaintext_allowed(&host)
            {
                let msg = vvva_permissions::plaintext_denied_message("HTTP", &host);
                let msg = v8::String::new(scope, &msg).unwrap();
                let err = v8::Exception::error(scope, msg);
                scope.throw_exception(err);
                return;
            }

            let result = tokio::task::block_in_place(|| {
                do_request(url, method, hdrs_json, body_opt, max_response_size)
            });

            match result {
                Ok(json) => {
                    rv.set(v8::String::new(scope, &json).unwrap().into());
                }
                // Like undici: a network failure rejects with a TypeError.
                Err(e) => {
                    let msg = v8::String::new(scope, &e.to_string()).unwrap();
                    let err = v8::Exception::type_error(scope, msg);
                    scope.throw_exception(err);
                }
            }
        },
    );

    let context = scope.get_current_context();
    let global = context.global(scope);
    global.set(
        scope,
        v8::String::new(scope, "__fetchAsync").unwrap().into(),
        native_fn.unwrap().into(),
    );

    let js_code = r#"
    globalThis.fetch = function(input, options) {
        if (input && typeof input === 'object' && typeof input.url === 'string') {
            var req = input;
            options = options ? Object.assign({ method: req.method, signal: req.signal }, options) : { method: req.method, signal: req.signal };
            if (options.headers == null && req.headers) options.headers = req.headers;
            if (options.body == null && req._body != null) options.body = req._body;
            input = req.url;
        }
        options = options || {};

        var method  = (options.method  || 'GET').toUpperCase();
        var signal  = options.signal || null;
        var body    = (options.body != null) ? String(options.body) : undefined;

        var hdrs = options.headers;
        var headersObj = {};
        if (hdrs && typeof hdrs.forEach === 'function') {
            hdrs.forEach(function(v, k) { headersObj[k] = v; });
        } else if (hdrs && typeof hdrs === 'object') {
            headersObj = hdrs;
        }

        if (signal && signal.aborted) {
            return Promise.reject(signal.reason || new Error('AbortError'));
        }

        var fetchUrl = String(input);
        var maxResponseSize;
        if (options.maxResponseSize != null) {
            var mrs = Number(options.maxResponseSize);
            if (Number.isFinite(mrs) && mrs >= 0) maxResponseSize = mrs;
        }
        var pending = new Promise(function(resolve, reject) {
            try {
                var result = __fetchAsync(fetchUrl, method, JSON.stringify(headersObj), body, maxResponseSize);
                resolve(result);
            } catch(e) {
                reject(e);
            }
        });

        if (signal) {
            var abortPromise = new Promise(function(_, reject) {
                signal.addEventListener('abort', function() {
                    reject(signal.reason || new Error('AbortError'));
                });
            });
            pending = Promise.race([pending, abortPromise]);
        }

        return pending.then(function(raw) {
            var data = JSON.parse(raw);
            var respHeaders = new Headers(data.headers);
            return new Response(data.body, {
                status:      data.status,
                statusText:  data.statusText,
                headers:     respHeaders,
                url:         fetchUrl,
                redirected:  false,
                type:        'basic',
            });
        });
    };
    "#;

    let source = v8::String::new(scope, js_code).unwrap();
    let script = Script::compile(scope, source, None).unwrap();
    let _ = script.run(scope);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_matches_what_is_dialed() {
        // VULN-07: `\` is a path separator for special schemes, so the
        // connection goes to 127.0.0.1 — the check must see the same host.
        let raw = "http://127.0.0.1\\@granted.example.com:39311/";
        assert_eq!(
            destination_from_url(raw),
            Some(("127.0.0.1".to_string(), 80))
        );
        let canon = canonical_url(raw).unwrap();
        assert!(!canon.contains('\\'), "{canon}");
        assert_eq!(destination_from_url(&canon), destination_from_url(raw));

        assert_eq!(
            destination_from_url("https://user:pw@API.Example.com/x"),
            Some(("api.example.com".to_string(), 443))
        );
        assert_eq!(
            destination_from_url("ws://[::1]:8080/"),
            Some(("::1".to_string(), 8080))
        );
        assert_eq!(destination_from_url("file:///etc/passwd"), None);
        assert_eq!(destination_from_url("not a url"), None);
    }
}
