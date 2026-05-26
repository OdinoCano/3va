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

    // JS wrapper: fetch(url, options?) -> Promise<Response>
    ctx.eval::<(), _>(
        r#"
        globalThis.fetch = function(url, options) {
            options = options || {};
            var method  = (options.method  || 'GET').toUpperCase();
            var headers = options.headers  || {};
            var body    = (options.body != null) ? String(options.body) : undefined;
            var signal  = options.signal || null;

            // Check AbortSignal before issuing the request
            if (signal && signal.aborted) {
                var reason = signal.reason || new Error('AbortError');
                return Promise.reject(reason);
            }

            var req = __fetchAsync(url, method, JSON.stringify(headers), body);

            // If a signal is provided, race against abort
            if (signal) {
                var abortPromise = new Promise(function(_, reject) {
                    signal.addEventListener('abort', function() {
                        reject(signal.reason || new Error('AbortError'));
                    });
                });
                req = Promise.race([req, abortPromise]);
            }

            return req.then(function(raw) {
                var data = JSON.parse(raw);
                return {
                    ok:          data.ok,
                    status:      data.status,
                    statusText:  data.statusText,
                    headers:     data.headers,
                    _body:       data.body,
                    url:         url,
                    redirected:  false,
                    type:        'basic',
                    text:        function() { return Promise.resolve(this._body); },
                    json:        function() {
                        try { return Promise.resolve(JSON.parse(this._body)); }
                        catch(e) { return Promise.reject(new SyntaxError('Invalid JSON: ' + e.message)); }
                    },
                    arrayBuffer: function() {
                        var s = this._body;
                        var buf = new ArrayBuffer(s.length);
                        var view = new Uint8Array(buf);
                        for (var i = 0; i < s.length; i++) view[i] = s.charCodeAt(i);
                        return Promise.resolve(buf);
                    },
                    bytes: function() {
                        return this.arrayBuffer().then(function(b) { return new Uint8Array(b); });
                    },
                    blob: function() { return Promise.resolve(new Blob([this._body], { type: this.headers['content-type'] || '' })); },
                    formData: function() {
                        try {
                            var ct = (this.headers['content-type'] || this.headers['Content-Type'] || '').toLowerCase();
                            var body = this._body;
                            var fd = new FormData();
                            if (ct.indexOf('application/x-www-form-urlencoded') >= 0) {
                                // URL-encoded: name=value&name2=value2
                                var pairs = body.split('&');
                                for (var i = 0; i < pairs.length; i++) {
                                    if (!pairs[i]) continue;
                                    var idx = pairs[i].indexOf('=');
                                    var k = decodeURIComponent(idx >= 0 ? pairs[i].slice(0, idx) : pairs[i]).replace(/\+/g, ' ');
                                    var v = decodeURIComponent(idx >= 0 ? pairs[i].slice(idx + 1) : '').replace(/\+/g, ' ');
                                    fd.append(k, v);
                                }
                                return Promise.resolve(fd);
                            }
                            if (ct.indexOf('multipart/form-data') >= 0) {
                                // Extract boundary
                                var bm = ct.match(/boundary=([^\s;]+)/);
                                if (!bm) return Promise.reject(new Error('formData: missing boundary in Content-Type'));
                                var boundary = '--' + bm[1];
                                var parts = body.split(boundary);
                                // parts[0] = preamble (ignore), parts[last] = '--' suffix (ignore)
                                for (var i = 1; i < parts.length - 1; i++) {
                                    var part = parts[i];
                                    // Strip leading \r\n
                                    if (part.startsWith('\r\n')) part = part.slice(2);
                                    if (part.endsWith('\r\n')) part = part.slice(0, -2);
                                    // Split headers from body at first blank line
                                    var headerEnd = part.indexOf('\r\n\r\n');
                                    if (headerEnd < 0) continue;
                                    var rawHeaders = part.slice(0, headerEnd);
                                    var partBody = part.slice(headerEnd + 4);
                                    // Parse Content-Disposition
                                    var cd = '';
                                    var lines = rawHeaders.split('\r\n');
                                    for (var j = 0; j < lines.length; j++) {
                                        if (lines[j].toLowerCase().startsWith('content-disposition:')) {
                                            cd = lines[j].slice(lines[j].indexOf(':') + 1).trim();
                                        }
                                    }
                                    var namem = cd.match(/name="([^"]*)"/);
                                    var filenamem = cd.match(/filename="([^"]*)"/);
                                    if (!namem) continue;
                                    var fieldName = namem[1];
                                    if (filenamem) {
                                        fd.append(fieldName, new File([partBody], filenamem[1]));
                                    } else {
                                        fd.append(fieldName, partBody);
                                    }
                                }
                                return Promise.resolve(fd);
                            }
                            return Promise.reject(new TypeError('formData: unsupported Content-Type: ' + ct));
                        } catch(e) { return Promise.reject(e); }
                    },
                    clone:  function() { return Object.assign({}, this); },
                };
            });
        };
        "#,
    )?;

    Ok(())
}
