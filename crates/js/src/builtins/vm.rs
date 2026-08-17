use std::collections::HashMap;
use v8::{
    ContextScope, FunctionCallbackArguments, GetPropertyNamesArgs, HandleScope, PinScope,
    ReturnValue, String as V8String,
};

struct VmCtx {
    contexts: HashMap<u32, v8::Global<v8::Context>>,
}

thread_local! {
    static VM_CTX: std::cell::RefCell<Option<Box<VmCtx>>> = const { std::cell::RefCell::new(None) };
}

fn with_vm_ctx<R>(f: impl FnOnce(&mut VmCtx) -> R) -> R {
    VM_CTX.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.is_none() {
            *cell = Some(Box::new(VmCtx {
                contexts: HashMap::new(),
            }));
        }
        f(cell.as_mut().unwrap())
    })
}

fn vm_ctx_insert(id: u32, global: v8::Global<v8::Context>) {
    with_vm_ctx(|ctx| ctx.contexts.insert(id, global));
}

fn vm_ctx_get(context_id: u32) -> Option<v8::Global<v8::Context>> {
    with_vm_ctx(|ctx| ctx.contexts.get(&context_id).cloned())
}

fn build_error_json(msg: &str) -> String {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    format!(r#"{{"error":"{escaped}"}}"#)
}

pub fn inject_vm(scope: &mut ContextScope<HandleScope>) -> anyhow::Result<()> {
    let context = scope.get_current_context();
    let global = context.global(scope);

    {
        let external = v8::External::new(scope, std::ptr::null_mut());
        let create_context_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>,
             args: FunctionCallbackArguments,
             mut _rv: ReturnValue| {
                let id = args.get(0).uint32_value(scope).unwrap_or(0);
                let sandbox_json_arg = args.get(1);
                let sandbox_json = sandbox_json_arg.to_rust_string_lossy(scope);

                let ctx_obj = v8::Context::new(scope, Default::default());
                let ctx_scope = v8::ContextScope::new(scope, ctx_obj);
                let global_obj = ctx_obj.global(&ctx_scope);

                if let Some(obj) = serde_json::from_str::<serde_json::Value>(&sandbox_json)
                    .ok()
                    .and_then(|v| v.as_object().cloned())
                {
                    for (key, value) in &obj {
                        let v8_key = V8String::new(&ctx_scope, key).unwrap();
                        let v8_val = match value {
                            serde_json::Value::Null => v8::null(&ctx_scope).into(),
                            serde_json::Value::Bool(b) => v8::Boolean::new(&ctx_scope, *b).into(),
                            serde_json::Value::Number(n) => {
                                if let Some(i) = n.as_i64() {
                                    v8::Integer::new_from_unsigned(&ctx_scope, i as u32).into()
                                } else if let Some(f) = n.as_f64() {
                                    v8::Number::new(&ctx_scope, f).into()
                                } else {
                                    v8::undefined(&ctx_scope).into()
                                }
                            }
                            serde_json::Value::String(s) => {
                                V8String::new(&ctx_scope, s).unwrap().into()
                            }
                            serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                                if let Ok(json_str) = serde_json::to_string(value) {
                                    let js_str = V8String::new(&ctx_scope, &json_str).unwrap();
                                    v8::json::parse(&ctx_scope, js_str)
                                        .unwrap_or_else(|| v8::undefined(&ctx_scope).into())
                                } else {
                                    v8::undefined(&ctx_scope).into()
                                }
                            }
                        };
                        let _ = global_obj.set(&ctx_scope, v8_key.into(), v8_val);
                    }
                }

                let global = v8::Global::new(&ctx_scope, ctx_obj);
                vm_ctx_insert(id, global);
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__vmCreateContext").unwrap().into(),
            create_context_fn.into(),
        );
    }

    {
        let external = v8::External::new(scope, std::ptr::null_mut());
        let run_in_context_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let context_id = args.get(0).uint32_value(scope).unwrap_or(0);
                let code = args.get(1).to_rust_string_lossy(scope);

                let ctx_global = match vm_ctx_get(context_id) {
                    Some(g) => g,
                    None => {
                        let err = build_error_json("unknown context id");
                        rv.set(V8String::new(scope, &err).unwrap().into());
                        return;
                    }
                };

                let ctx_local = v8::Local::new(scope, &ctx_global);

                let ctx_scope = v8::ContextScope::new(scope, ctx_local);
                let code_src = V8String::new(&ctx_scope, &code).unwrap();
                let script = match v8::Script::compile(&ctx_scope, code_src, None) {
                    Some(s) => s,
                    None => {
                        let err = build_error_json("compile error");
                        rv.set(V8String::new(&ctx_scope, &err).unwrap().into());
                        return;
                    }
                };
                let result = script.run(&ctx_scope);
                let result_json = match result {
                    Some(value) => {
                        let json_val = v8::json::stringify(&ctx_scope, value);
                        match json_val {
                            Some(json_str) => format!(
                                r#"{{"value":{}}}"#,
                                json_str.to_rust_string_lossy(&ctx_scope)
                            ),
                            None => {
                                let result_str = value.to_rust_string_lossy(&ctx_scope);
                                let escaped = result_str.replace('\\', "\\\\").replace('"', "\\\"");
                                format!(r#"{{"value":"{escaped}"}}"#)
                            }
                        }
                    }
                    None => build_error_json("runtime error"),
                };
                rv.set(V8String::new(&ctx_scope, &result_json).unwrap().into());
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__vmRunInContextById").unwrap().into(),
            run_in_context_fn.into(),
        );
    }

    {
        let external = v8::External::new(scope, std::ptr::null_mut());
        let get_globals_fn = v8::Function::builder(
            |scope: &mut PinScope<'_, '_>, args: FunctionCallbackArguments, mut rv: ReturnValue| {
                let context_id = args.get(0).uint32_value(scope).unwrap_or(0);

                let ctx_global = match vm_ctx_get(context_id) {
                    Some(g) => g,
                    None => {
                        rv.set(v8::null(scope).into());
                        return;
                    }
                };

                let ctx_local = v8::Local::new(scope, &ctx_global);

                let ctx_scope = v8::ContextScope::new(scope, ctx_local);
                let global_obj = ctx_local.global(&ctx_scope);

                let prop_names = global_obj
                    .get_property_names(&ctx_scope, GetPropertyNamesArgs::default())
                    .unwrap_or_else(|| v8::Array::new(&ctx_scope, 0));

                let len = prop_names.length();
                let mut result_map = serde_json::Map::new();

                for i in 0..len {
                    let index = v8::Integer::new_from_unsigned(&ctx_scope, i);
                    let key = prop_names.get(&ctx_scope, index.into());
                    if let Some(key_val) = key {
                        let key_name = key_val.to_rust_string_lossy(&ctx_scope);
                        let Some(val) = global_obj.get(&ctx_scope, key_val) else {
                            continue;
                        };
                        let Some(json_str) = v8::json::stringify(&ctx_scope, val) else {
                            continue;
                        };
                        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(
                            &json_str.to_rust_string_lossy(&ctx_scope),
                        ) {
                            result_map.insert(key_name, parsed);
                        }
                    }
                }

                let result_json =
                    serde_json::to_string(&result_map).unwrap_or_else(|_| "{}".to_string());
                rv.set(V8String::new(&ctx_scope, &result_json).unwrap().into());
            },
        )
        .data(external.into())
        .build(scope)
        .unwrap();
        global.set(
            scope,
            V8String::new(scope, "__vmGetContextGlobals")
                .unwrap()
                .into(),
            get_globals_fn.into(),
        );
    }

    Ok(())
}
