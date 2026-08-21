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

fn host_from_url(url: &str) -> Option<String> {
    let after_scheme = url.find("://")?;
    let rest = &url[after_scheme + 3..];
    let host_part = rest.split('/').next().unwrap_or(rest);
    let host = host_part.split(':').next().unwrap_or(host_part);
    if host.is_empty() {
        None
    } else {
        Some(host.to_lowercase())
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

    let agent = ureq::AgentBuilder::new().redirects(0).build();
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

static INJECT_FETCH_PERMISSIONS: std::sync::OnceLock<Arc<PermissionState>> =
    std::sync::OnceLock::new();
fn permissions() -> &'static Arc<PermissionState> {
    INJECT_FETCH_PERMISSIONS.get().unwrap()
}

pub fn inject_fetch(
    scope: &mut v8::ContextScope<HandleScope>,
    permissions_param: Arc<PermissionState>,
) -> anyhow::Result<()> {
    INJECT_FETCH_PERMISSIONS.set(permissions_param).ok();

    let native_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: FunctionCallbackArguments,
              mut rv: ReturnValue| {
            let url = args.get(0).to_rust_string_lossy(scope);
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
                    let err = v8::String::new(scope, "Invalid URL").unwrap();
                    scope.throw_exception(err.into());
                    return;
                }
            };

            if !permissions().check(&Capability::Network(host.clone())) {
                let msg = format!("Network access denied. Run with --allow-net={}", host);
                let err = v8::String::new(scope, &msg).unwrap();
                scope.throw_exception(err.into());
                return;
            }

            let result = tokio::task::block_in_place(|| {
                do_request(url, method, hdrs_json, body_opt, max_response_size)
            });

            match result {
                Ok(json) => {
                    rv.set(v8::String::new(scope, &json).unwrap().into());
                }
                Err(e) => {
                    let err = v8::String::new(scope, &e.to_string()).unwrap();
                    scope.throw_exception(err.into());
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
