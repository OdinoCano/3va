use rquickjs::function::Async;
use rquickjs::{Ctx, Function, Result};
use std::collections::HashMap;
use std::sync::Arc;
use vvva_permissions::{Capability, PermissionState};

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

/// Blocking HTTP request executed inside spawn_blocking.
fn do_request(
    url: String,
    method: String,
    hdrs_json: String,
    body: Option<String>,
) -> anyhow::Result<String> {
    let extra: HashMap<String, String> = serde_json::from_str(&hdrs_json).unwrap_or_default();

    let mut req = ureq::request(&method, &url);
    for (k, v) in &extra {
        req = req.set(k, v);
    }
    req = req.set("User-Agent", "3va/0.1");

    let resp_result = if let Some(b) = body {
        req.send_string(&b)
    } else {
        req.call()
    };

    let json = match resp_result {
        Ok(r) => {
            let status = r.status();
            let status_text = r.status_text().to_string();
            let ok = (200u16..300).contains(&status);
            let mut resp_hdrs = serde_json::Map::new();
            for name in r.headers_names() {
                if let Some(val) = r.header(&name) {
                    resp_hdrs.insert(name, serde_json::Value::String(val.to_string()));
                }
            }
            let body_text = r.into_string().unwrap_or_default();
            serde_json::json!({
                "ok": ok, "status": status, "statusText": status_text,
                "headers": resp_hdrs, "body": body_text,
            })
        }
        Err(ureq::Error::Status(status, r)) => {
            let status_text = r.status_text().to_string();
            let body_text = r.into_string().unwrap_or_default();
            serde_json::json!({
                "ok": false, "status": status, "statusText": status_text,
                "headers": {}, "body": body_text,
            })
        }
        Err(e) => return Err(anyhow::anyhow!("fetch failed: {}", e)),
    };

    Ok(json.to_string())
}

pub fn inject_fetch(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();

    // __fetchAsync(url, method, headersJson, body?) -> Promise<responseJson>
    // The async closure runs on the JS runtime executor; the blocking HTTP call
    // is offloaded to the tokio blocking thread pool via spawn_blocking.
    let perms = permissions;
    globals.set(
        "__fetchAsync",
        Function::new(
            ctx.clone(),
            Async(
                move |url: String, method: String, hdrs_json: String, body: Option<String>| {
                    let perms = perms.clone();
                    async move {
                        let host = host_from_url(&url)
                            .ok_or_else(|| rquickjs::Error::new_from_js("url", "Invalid URL"))?;
                        if !perms.check(&Capability::Network(host.clone())) {
                            return Err(rquickjs::Error::new_from_js_message(
                                "permission",
                                "permission",
                                format!("Network access denied. Run with --allow-net={}", host),
                            ));
                        }
                        tokio::task::spawn_blocking(move || {
                            do_request(url, method, hdrs_json, body)
                        })
                        .await
                        .map_err(|e| {
                            rquickjs::Error::new_from_js_message("spawn", "spawn", e.to_string())
                        })?
                        .map_err(|e| {
                            rquickjs::Error::new_from_js_message("http", "http", e.to_string())
                        })
                    }
                },
            ),
        )?,
    )?;

    // JS wrapper: fetch(input, options?) -> Promise<Response>
    // Accepts a URL string or a Request object as first argument.
    // Returns a proper Response instance (WinterCG-compatible).
    ctx.eval::<(), _>(
        r#"
        globalThis.fetch = function(input, options) {
            // Accept Request object as first argument
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

            // Normalise headers — accept Headers instance, plain object, or undefined
            var hdrs = options.headers;
            var headersObj = {};
            if (hdrs && typeof hdrs.forEach === 'function') {
                hdrs.forEach(function(v, k) { headersObj[k] = v; });
            } else if (hdrs && typeof hdrs === 'object') {
                headersObj = hdrs;
            }

            // Check AbortSignal before issuing the request
            if (signal && signal.aborted) {
                return Promise.reject(signal.reason || new Error('AbortError'));
            }

            var fetchUrl = String(input);
            var pending = __fetchAsync(fetchUrl, method, JSON.stringify(headersObj), body);

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
        "#,
    )?;

    Ok(())
}
