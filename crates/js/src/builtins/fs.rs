use rquickjs::{Ctx, Function, Result, function::Rest};
use std::path::PathBuf;
use std::sync::Arc;
use vvva_permissions::{Capability, PermissionState};

/// Create a JS Error exception with a dynamic message.
fn js_err<'js>(ctx: &Ctx<'js>, msg: String) -> rquickjs::Error {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    match ctx.eval::<rquickjs::Value, _>(format!("new Error(\"{}\")", escaped).as_str()) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
    }
}

pub fn inject_fs(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();

    // ── __fsReadFileSync(path) -> String ──────────────────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsReadFileSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<String> {
                let path_str = args
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsReadFileSync() requires a path".into()))?;
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileRead(path.clone())) {
                    return Err(js_err(
                        &ctx,
                        format!(
                            "Permission denied: --allow-read={} is required",
                            path.display()
                        ),
                    ));
                }
                std::fs::read_to_string(&path)
                    .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))
            },
        )?,
    )?;

    // ── __fsWriteFileSync(path, content) ──────────────────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsWriteFileSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<()> {
                let mut it = args.0.into_iter();
                let path_str = it
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsWriteFileSync() requires a path".into()))?;
                let content = it.next().unwrap_or_default();
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileWrite(path.clone())) {
                    return Err(js_err(
                        &ctx,
                        format!(
                            "Permission denied: --allow-write={} is required",
                            path.display()
                        ),
                    ));
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                std::fs::write(&path, content)
                    .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))
            },
        )?,
    )?;

    // ── __fsExistsSync(path) -> bool ──────────────────────────────────────────
    globals.set(
        "__fsExistsSync",
        Function::new(ctx.clone(), |args: Rest<String>| -> bool {
            args.0
                .into_iter()
                .next()
                .map(|p| PathBuf::from(p).exists())
                .unwrap_or(false)
        })?,
    )?;

    // ── __fsReaddirSync(path) -> String (JSON array of filenames) ─────────────
    let perms = permissions.clone();
    globals.set(
        "__fsReaddirSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<String> {
                let path_str = args
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsReaddirSync() requires a path".into()))?;
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileRead(path.clone())) {
                    return Err(js_err(
                        &ctx,
                        format!(
                            "Permission denied: --allow-read={} is required",
                            path.display()
                        ),
                    ));
                }
                let entries = std::fs::read_dir(&path)
                    .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))?;
                let names: Vec<String> = entries
                    .flatten()
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect();
                Ok(serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_string()))
            },
        )?,
    )?;

    // ── __fsMkdirSync(path) ───────────────────────────────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsMkdirSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<()> {
                let path_str = args
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsMkdirSync() requires a path".into()))?;
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileWrite(path.clone())) {
                    return Err(js_err(
                        &ctx,
                        format!(
                            "Permission denied: --allow-write={} is required",
                            path.display()
                        ),
                    ));
                }
                std::fs::create_dir_all(&path)
                    .map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))
            },
        )?,
    )?;

    // ── __fsRmSync(path) ──────────────────────────────────────────────────────
    let perms = permissions.clone();
    globals.set(
        "__fsRmSync",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, args: Rest<String>| -> Result<()> {
                let path_str = args
                    .0
                    .into_iter()
                    .next()
                    .ok_or_else(|| js_err(&ctx, "__fsRmSync() requires a path".into()))?;
                let path = PathBuf::from(&path_str);
                if !perms.check(&Capability::FileWrite(path.clone())) {
                    return Err(js_err(
                        &ctx,
                        format!(
                            "Permission denied: --allow-write={} is required",
                            path.display()
                        ),
                    ));
                }
                let result = if path.is_dir() {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                result.map_err(|e| js_err(&ctx, format!("ENOENT: {}: '{}'", e, path_str)))
            },
        )?,
    )?;

    // ── JS wrapper: globalThis.fs ─────────────────────────────────────────────
    ctx.eval::<(), _>(
        r#"
        globalThis.fs = {
            readFile:  function(path)          { return __fsReadFileSync(path); },
            writeFile: function(path, content) { return __fsWriteFileSync(path, content); },
            exists:    function(path)          { return __fsExistsSync(path); },
            readdir:   function(path)          { return JSON.parse(__fsReaddirSync(path)); },
            mkdir:     function(path)          { return __fsMkdirSync(path); },
            rm:        function(path)          { return __fsRmSync(path); },

            promises: {
                readFile:  function(path)          { try { return Promise.resolve(__fsReadFileSync(path));             } catch(e) { return Promise.reject(e); } },
                writeFile: function(path, content) { try { return Promise.resolve(__fsWriteFileSync(path, content));   } catch(e) { return Promise.reject(e); } },
                readdir:   function(path)          { try { return Promise.resolve(JSON.parse(__fsReaddirSync(path)));  } catch(e) { return Promise.reject(e); } },
                mkdir:     function(path)          { try { return Promise.resolve(__fsMkdirSync(path));                } catch(e) { return Promise.reject(e); } },
                rm:        function(path)          { try { return Promise.resolve(__fsRmSync(path));                   } catch(e) { return Promise.reject(e); } },
            }
        };
        "#,
    )?;

    Ok(())
}
