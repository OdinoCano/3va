use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use v8::{
    ContextScope, FunctionCallbackArguments, HandleScope, PinScope, ReturnValue, String as V8String,
};

static SOURCE_MAPS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn source_maps() -> &'static Mutex<HashMap<String, String>> {
    SOURCE_MAPS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn inject_source_maps(scope: &mut ContextScope<HandleScope>) -> anyhow::Result<()> {
    let context = scope.get_current_context();
    let global = context.global(scope);

    let store_sm_fn = v8::Function::builder(
        |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut _rv: ReturnValue| {
            let path = args.get(0).to_rust_string_lossy(scope);
            let map_json = args.get(1).to_rust_string_lossy(scope);
            source_maps().lock().unwrap().insert(path, map_json);
        },
    )
    .build(scope)
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__storeSourceMap").unwrap().into(),
        store_sm_fn.into(),
    );

    let get_sm_fn = v8::Function::builder(
        |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let path = args.get(0).to_rust_string_lossy(scope);
            let map = source_maps().lock().unwrap().get(&path).cloned();
            match map {
                Some(m) => rv.set(V8String::new(scope, &m).unwrap().into()),
                None => rv.set(v8::null(scope).into()),
            }
        },
    )
    .build(scope)
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__getSourceMap").unwrap().into(),
        get_sm_fn.into(),
    );

    let apply_sm_fn = v8::Function::builder(
        |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
            let path = args.get(0).to_rust_string_lossy(scope);
            let line = args.get(1).uint32_value(scope).unwrap_or(1);
            let col = args.get(2).uint32_value(scope).unwrap_or(0);

            let maps = source_maps().lock().unwrap();
            let map_json = match maps.get(&path) {
                Some(m) => m,
                None => {
                    rv.set(v8::null(scope).into());
                    return;
                }
            };

            let Ok(parsed) = serde_json::from_str::<serde_json::Value>(map_json) else {
                rv.set(v8::null(scope).into());
                return;
            };
            let first_source = parsed
                .as_object()
                .and_then(|obj| obj.get("sources"))
                .and_then(|s| s.as_array())
                .and_then(|sources| sources.first())
                .and_then(|v| v.as_str());

            match first_source {
                Some(first_source) => {
                    let result = serde_json::json!({
                        "source": first_source,
                        "line": line.saturating_sub(1),
                        "col": col
                    });
                    rv.set(V8String::new(scope, &result.to_string()).unwrap().into());
                }
                None => rv.set(v8::null(scope).into()),
            }
        },
    )
    .build(scope)
    .unwrap();
    global.set(
        scope,
        V8String::new(scope, "__applySourceMap").unwrap().into(),
        apply_sm_fn.into(),
    );

    Ok(())
}
