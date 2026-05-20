use rquickjs::{Ctx, Function, Result, function::Rest};
use std::cell::RefCell;
use std::rc::Rc;
use vvva_permissions::{Capability, PermissionState};

/// Create a JS Error exception with a dynamic message.
fn js_err<'js>(ctx: &Ctx<'js>, msg: String) -> rquickjs::Error {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    match ctx.eval::<rquickjs::Value, _>(format!("new Error(\"{}\")", escaped).as_str()) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
    }
}

/// Extract the hostname from a URL (strips scheme, port, and path).
fn host_from_url(url: &str) -> Option<String> {
    let after_scheme = url.find("://")?;
    let rest = &url[after_scheme + 3..];
    let host_part = rest.split('/').next().unwrap_or(rest);
    let host = host_part.split(':').next().unwrap_or(host_part);
    if host.is_empty() { None } else { Some(host.to_lowercase()) }
}

pub fn inject_fetch(ctx: &Ctx, permissions: Rc<RefCell<PermissionState>>) -> Result<()> {
    let globals = ctx.globals();

    // ── __fetchSync(url, method, headersJson, body?) -> responseJson ──────────
    // Returns: { ok, status, statusText, headers: {k:v,...}, body: "" }
    let perms = permissions.clone();
    globals.set(
        "__fetchSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<String> {
                let mut it = args.0.into_iter();
                let url     = it.next().ok_or_else(|| js_err(&ctx, "__fetchSync() requires a URL".into()))?;
                let method  = it.next().unwrap_or_else(|| "GET".into());
                let hdrs_js = it.next().unwrap_or_else(|| "{}".into());
                let body    = it.next();

                // Permission check
                let host = host_from_url(&url).ok_or_else(|| {
                    js_err(&ctx, format!("Invalid URL: {}", url))
                })?;
                if !perms.borrow().check(&Capability::Network(host.clone())) {
                    return Err(js_err(
                        &ctx,
                        format!("Network access denied. Run with --allow-net={}", host),
                    ));
                }

                // Parse extra request headers
                let extra: std::collections::HashMap<String, String> =
                    serde_json::from_str(&hdrs_js).unwrap_or_default();

                // Build and send request via ureq
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
                            "ok": ok,
                            "status": status,
                            "statusText": status_text,
                            "headers": resp_hdrs,
                            "body": body_text,
                        })
                    }
                    Err(ureq::Error::Status(status, r)) => {
                        let status_text = r.status_text().to_string();
                        let body_text = r.into_string().unwrap_or_default();
                        serde_json::json!({
                            "ok": false,
                            "status": status,
                            "statusText": status_text,
                            "headers": {},
                            "body": body_text,
                        })
                    }
                    Err(e) => {
                        return Err(js_err(&ctx, format!("fetch failed: {}", e)));
                    }
                };

                Ok(json.to_string())
            },
        )?,
    )?;

    // ── JS wrapper: globalThis.fetch(url, options?) -> Promise<Response> ──────
    ctx.eval::<(), _>(
        r#"
        globalThis.fetch = function(url, options) {
            options = options || {};
            var method  = (options.method  || 'GET').toUpperCase();
            var headers = options.headers  || {};
            var body    = (options.body != null) ? String(options.body) : undefined;

            try {
                var raw  = __fetchSync(url, method, JSON.stringify(headers), body);
                var data = JSON.parse(raw);

                var response = {
                    ok:         data.ok,
                    status:     data.status,
                    statusText: data.statusText,
                    headers:    data.headers,
                    _body:      data.body,
                    url:        url,

                    text: function()   { return Promise.resolve(this._body); },
                    json: function()   {
                        try { return Promise.resolve(JSON.parse(this._body)); }
                        catch(e) { return Promise.reject(new SyntaxError('Invalid JSON in response: ' + e.message)); }
                    },
                    arrayBuffer: function() { return Promise.resolve(this._body); },
                    clone: function() {
                        var copy = Object.assign({}, this);
                        return copy;
                    },
                };
                return Promise.resolve(response);
            } catch(e) {
                return Promise.reject(e);
            }
        };
        "#,
    )?;

    Ok(())
}
