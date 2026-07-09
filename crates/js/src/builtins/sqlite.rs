use base64::Engine;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use v8::{ContextScope, Function, HandleScope, PinScope, Script, String as V8String};

type ConnMap = Arc<Mutex<HashMap<u32, Arc<Mutex<rusqlite::Connection>>>>>;
type StmtMap = Arc<Mutex<HashMap<u32, (u32, String)>>>;

static SQLITE_CONNS: std::sync::OnceLock<ConnMap> = std::sync::OnceLock::new();
fn conns() -> &'static ConnMap {
    SQLITE_CONNS.get().unwrap()
}
static SQLITE_STMTS: std::sync::OnceLock<StmtMap> = std::sync::OnceLock::new();
fn stmts() -> &'static StmtMap {
    SQLITE_STMTS.get().unwrap()
}
static SQLITE_CONN_COUNTER: std::sync::OnceLock<Arc<Mutex<u32>>> = std::sync::OnceLock::new();
fn conn_counter() -> &'static Arc<Mutex<u32>> {
    SQLITE_CONN_COUNTER.get().unwrap()
}
static SQLITE_STMT_COUNTER: std::sync::OnceLock<Arc<Mutex<u32>>> = std::sync::OnceLock::new();
fn stmt_counter() -> &'static Arc<Mutex<u32>> {
    SQLITE_STMT_COUNTER.get().unwrap()
}

pub fn inject_sqlite(scope: &mut ContextScope<HandleScope>) -> anyhow::Result<()> {
    SQLITE_CONNS.set(Arc::new(Mutex::new(HashMap::new()))).ok();
    SQLITE_STMTS.set(Arc::new(Mutex::new(HashMap::new()))).ok();
    SQLITE_CONN_COUNTER.set(Arc::new(Mutex::new(0u32))).ok();
    SQLITE_STMT_COUNTER.set(Arc::new(Mutex::new(0u32))).ok();
    let context = scope.get_current_context();
    let global = context.global(scope);

    let sqlite_open_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: v8::FunctionCallbackArguments<'_>,
              mut rv: v8::ReturnValue<'_>| {
            let path_arg = args.get(0);
            let path = path_arg.to_rust_string_lossy(scope);

            match rusqlite::Connection::open(&path) {
                Ok(conn) => {
                    let mut id = conn_counter().lock().unwrap();
                    *id += 1;
                    let cid = *id;
                    conns()
                        .lock()
                        .unwrap()
                        .insert(cid, Arc::new(Mutex::new(conn)));
                    rv.set(v8::Integer::new_from_unsigned(scope, cid).into());
                }
                Err(e) => {
                    let err_str =
                        V8String::new(scope, &format!("sqlite open error: {}", e)).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    );
    let key = V8String::new(scope, "__sqliteOpen").unwrap().into();
    global.set(scope, key, sqlite_open_fn.unwrap().into());

    let sqlite_close_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: v8::FunctionCallbackArguments<'_>,
              mut rv: v8::ReturnValue<'_>| {
            let id_arg = args.get(0);
            let id = id_arg.uint32_value(scope).unwrap_or(0);
            conns().lock().unwrap().remove(&id);
            rv.set(v8::undefined(scope).into());
        },
    );
    let key = V8String::new(scope, "__sqliteClose").unwrap().into();
    global.set(scope, key, sqlite_close_fn.unwrap().into());

    let sqlite_exec_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: v8::FunctionCallbackArguments<'_>,
              mut rv: v8::ReturnValue<'_>| {
            let id_arg = args.get(0);
            let id = id_arg.uint32_value(scope).unwrap_or(0);
            let sql_arg = args.get(1);
            let sql = sql_arg.to_rust_string_lossy(scope);

            let map = conns().lock().unwrap();
            match map.get(&id) {
                Some(conn) => match conn.lock().unwrap().execute_batch(&sql) {
                    Ok(_) => rv.set(v8::undefined(scope).into()),
                    Err(e) => {
                        let err_str =
                            V8String::new(scope, &format!("sqlite exec error: {}", e)).unwrap();
                        rv.set(err_str.into());
                    }
                },
                None => {
                    let err_str = V8String::new(scope, "unknown connection").unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    );
    let key = V8String::new(scope, "__sqliteExec").unwrap().into();
    global.set(scope, key, sqlite_exec_fn.unwrap().into());

    let sqlite_prepare_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: v8::FunctionCallbackArguments<'_>,
              mut rv: v8::ReturnValue<'_>| {
            let conn_id_arg = args.get(0);
            let conn_id = conn_id_arg.uint32_value(scope).unwrap_or(0);
            let sql_arg = args.get(1);
            let sql = sql_arg.to_rust_string_lossy(scope);

            {
                let map = conns().lock().unwrap();
                match map.get(&conn_id) {
                    Some(conn) => {
                        if let Err(e) = conn.lock().unwrap().prepare(&sql) {
                            let err_str =
                                V8String::new(scope, &format!("sqlite prepare error: {}", e))
                                    .unwrap();
                            rv.set(err_str.into());
                            return;
                        }
                    }
                    None => {
                        let err_str = V8String::new(scope, "unknown connection").unwrap();
                        rv.set(err_str.into());
                        return;
                    }
                }
            }

            let mut id = stmt_counter().lock().unwrap();
            *id += 1;
            let sid = *id;
            stmts().lock().unwrap().insert(sid, (conn_id, sql));
            rv.set(v8::Integer::new_from_unsigned(scope, sid).into());
        },
    );
    let key = V8String::new(scope, "__sqlitePrepare").unwrap().into();
    global.set(scope, key, sqlite_prepare_fn.unwrap().into());

    let sqlite_run_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: v8::FunctionCallbackArguments<'_>,
              mut rv: v8::ReturnValue<'_>| {
            let stmt_id_arg = args.get(0);
            let stmt_id = stmt_id_arg.uint32_value(scope).unwrap_or(0);
            let params_json_arg = args.get(1);
            let params_json = params_json_arg.to_rust_string_lossy(scope);

            let (conn_id, sql) = {
                let map = stmts().lock().unwrap();
                match map.get(&stmt_id) {
                    Some(e) => e.clone(),
                    None => {
                        let err_str = V8String::new(scope, "unknown statement").unwrap();
                        rv.set(err_str.into());
                        return;
                    }
                }
            };

            let params: Vec<serde_json::Value> =
                serde_json::from_str(&params_json).unwrap_or_default();
            let map = conns().lock().unwrap();
            let conn = match map.get(&conn_id) {
                Some(c) => c,
                None => {
                    let err_str = V8String::new(scope, "unknown connection").unwrap();
                    rv.set(err_str.into());
                    return;
                }
            };
            let conn = conn.lock().unwrap();

            let params_rusqlite: Vec<Box<dyn rusqlite::ToSql>> = params
                .iter()
                .map(|v| -> Box<dyn rusqlite::ToSql> {
                    match v {
                        serde_json::Value::Null => Box::new(rusqlite::types::Value::Null),
                        serde_json::Value::Bool(b) => Box::new(*b as i64),
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                Box::new(i)
                            } else {
                                Box::new(n.as_f64().unwrap_or(0.0))
                            }
                        }
                        serde_json::Value::String(s) => Box::new(s.clone()),
                        _ => Box::new(serde_json::to_string(v).unwrap_or_default()),
                    }
                })
                .collect();

            let params_refs: Vec<&dyn rusqlite::ToSql> =
                params_rusqlite.iter().map(|b| b.as_ref()).collect();

            match conn.execute(&sql, params_refs.as_slice()) {
                Ok(changes) => {
                    let rowid = conn.last_insert_rowid();
                    let json = format!("{{\"changes\":{},\"lastInsertRowid\":{}}}", changes, rowid);
                    let result_str = V8String::new(scope, &json).unwrap();
                    rv.set(result_str.into());
                }
                Err(e) => {
                    let err_str =
                        V8String::new(scope, &format!("sqlite run error: {}", e)).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    );
    let key = V8String::new(scope, "__sqliteRun").unwrap().into();
    global.set(scope, key, sqlite_run_fn.unwrap().into());

    let sqlite_get_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: v8::FunctionCallbackArguments<'_>,
              mut rv: v8::ReturnValue<'_>| {
            let stmt_id_arg = args.get(0);
            let stmt_id = stmt_id_arg.uint32_value(scope).unwrap_or(0);
            let params_json_arg = args.get(1);
            let params_json = params_json_arg.to_rust_string_lossy(scope);

            let (conn_id, sql) = {
                let map = stmts().lock().unwrap();
                match map.get(&stmt_id) {
                    Some(e) => e.clone(),
                    None => {
                        let err_str = V8String::new(scope, "unknown statement").unwrap();
                        rv.set(err_str.into());
                        return;
                    }
                }
            };

            let params: Vec<serde_json::Value> =
                serde_json::from_str(&params_json).unwrap_or_default();
            let map = conns().lock().unwrap();
            let conn = match map.get(&conn_id) {
                Some(c) => c,
                None => {
                    let err_str = V8String::new(scope, "unknown connection").unwrap();
                    rv.set(err_str.into());
                    return;
                }
            };
            let conn = conn.lock().unwrap();

            let params_rusqlite: Vec<Box<dyn rusqlite::ToSql>> = params
                .iter()
                .map(|v| -> Box<dyn rusqlite::ToSql> {
                    match v {
                        serde_json::Value::Null => Box::new(rusqlite::types::Value::Null),
                        serde_json::Value::Bool(b) => Box::new(*b as i64),
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                Box::new(i)
                            } else {
                                Box::new(n.as_f64().unwrap_or(0.0))
                            }
                        }
                        serde_json::Value::String(s) => Box::new(s.clone()),
                        _ => Box::new(serde_json::to_string(v).unwrap_or_default()),
                    }
                })
                .collect();

            let params_refs: Vec<&dyn rusqlite::ToSql> =
                params_rusqlite.iter().map(|b| b.as_ref()).collect();

            let mut stmt = match conn.prepare(&sql) {
                Ok(s) => s,
                Err(e) => {
                    let err_str =
                        V8String::new(scope, &format!("sqlite prepare error: {}", e)).unwrap();
                    rv.set(err_str.into());
                    return;
                }
            };

            let cols: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
            let mut rows = match stmt.query(params_refs.as_slice()) {
                Ok(r) => r,
                Err(e) => {
                    let err_str =
                        V8String::new(scope, &format!("sqlite query error: {}", e)).unwrap();
                    rv.set(err_str.into());
                    return;
                }
            };

            match rows.next() {
                Ok(Some(row)) => {
                    let mut obj = serde_json::Map::new();
                    for (i, col) in cols.iter().enumerate() {
                        let val: rusqlite::types::Value =
                            row.get(i).unwrap_or(rusqlite::types::Value::Null);
                        obj.insert(
                            col.clone(),
                            match val {
                                rusqlite::types::Value::Null => serde_json::Value::Null,
                                rusqlite::types::Value::Integer(n) => {
                                    serde_json::Value::Number(n.into())
                                }
                                rusqlite::types::Value::Real(f) => serde_json::json!(f),
                                rusqlite::types::Value::Text(s) => serde_json::Value::String(s),
                                rusqlite::types::Value::Blob(b) => serde_json::Value::String(
                                    base64::engine::general_purpose::STANDARD.encode(b),
                                ),
                            },
                        );
                    }
                    let json = serde_json::to_string(&obj).unwrap_or_else(|_| "null".to_string());
                    let result_str = V8String::new(scope, &json).unwrap();
                    rv.set(result_str.into());
                }
                Ok(None) => {
                    let null_str = V8String::new(scope, "null").unwrap();
                    rv.set(null_str.into());
                }
                Err(e) => {
                    let err_str =
                        V8String::new(scope, &format!("sqlite row error: {}", e)).unwrap();
                    rv.set(err_str.into());
                }
            }
        },
    );
    let key = V8String::new(scope, "__sqliteGet").unwrap().into();
    global.set(scope, key, sqlite_get_fn.unwrap().into());

    let sqlite_all_fn = Function::new(
        scope,
        move |scope: &mut PinScope<'_, '_>,
              args: v8::FunctionCallbackArguments<'_>,
              mut rv: v8::ReturnValue<'_>| {
            let stmt_id_arg = args.get(0);
            let stmt_id = stmt_id_arg.uint32_value(scope).unwrap_or(0);
            let params_json_arg = args.get(1);
            let params_json = params_json_arg.to_rust_string_lossy(scope);

            let (conn_id, sql) = {
                let map = stmts().lock().unwrap();
                match map.get(&stmt_id) {
                    Some(e) => e.clone(),
                    None => {
                        let err_str = V8String::new(scope, "unknown statement").unwrap();
                        rv.set(err_str.into());
                        return;
                    }
                }
            };

            let params: Vec<serde_json::Value> =
                serde_json::from_str(&params_json).unwrap_or_default();
            let map = conns().lock().unwrap();
            let conn = match map.get(&conn_id) {
                Some(c) => c,
                None => {
                    let err_str = V8String::new(scope, "unknown connection").unwrap();
                    rv.set(err_str.into());
                    return;
                }
            };
            let conn = conn.lock().unwrap();

            let params_rusqlite: Vec<Box<dyn rusqlite::ToSql>> = params
                .iter()
                .map(|v| -> Box<dyn rusqlite::ToSql> {
                    match v {
                        serde_json::Value::Null => Box::new(rusqlite::types::Value::Null),
                        serde_json::Value::Bool(b) => Box::new(*b as i64),
                        serde_json::Value::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                Box::new(i)
                            } else {
                                Box::new(n.as_f64().unwrap_or(0.0))
                            }
                        }
                        serde_json::Value::String(s) => Box::new(s.clone()),
                        _ => Box::new(serde_json::to_string(v).unwrap_or_default()),
                    }
                })
                .collect();

            let params_refs: Vec<&dyn rusqlite::ToSql> =
                params_rusqlite.iter().map(|b| b.as_ref()).collect();

            let mut stmt = match conn.prepare(&sql) {
                Ok(s) => s,
                Err(e) => {
                    let err_str =
                        V8String::new(scope, &format!("sqlite prepare error: {}", e)).unwrap();
                    rv.set(err_str.into());
                    return;
                }
            };

            let cols: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
            let mut rows = match stmt.query(params_refs.as_slice()) {
                Ok(r) => r,
                Err(e) => {
                    let err_str =
                        V8String::new(scope, &format!("sqlite query error: {}", e)).unwrap();
                    rv.set(err_str.into());
                    return;
                }
            };

            let mut result = Vec::new();
            while let Ok(Some(row)) = rows.next() {
                let mut obj = serde_json::Map::new();
                for (i, col) in cols.iter().enumerate() {
                    let val: rusqlite::types::Value =
                        row.get(i).unwrap_or(rusqlite::types::Value::Null);
                    obj.insert(
                        col.clone(),
                        match val {
                            rusqlite::types::Value::Null => serde_json::Value::Null,
                            rusqlite::types::Value::Integer(n) => {
                                serde_json::Value::Number(n.into())
                            }
                            rusqlite::types::Value::Real(f) => serde_json::json!(f),
                            rusqlite::types::Value::Text(s) => serde_json::Value::String(s),
                            rusqlite::types::Value::Blob(b) => serde_json::Value::String(
                                base64::engine::general_purpose::STANDARD.encode(b),
                            ),
                        },
                    );
                }
                result.push(serde_json::Value::Object(obj));
            }

            let json = serde_json::to_string(&result).unwrap_or_else(|_| "[]".to_string());
            let result_str = V8String::new(scope, &json).unwrap();
            rv.set(result_str.into());
        },
    );
    let key = V8String::new(scope, "__sqliteAll").unwrap().into();
    global.set(scope, key, sqlite_all_fn.unwrap().into());

    let js_code = r#"
        (function() {
            function _parseParams(args) {
                return JSON.stringify(Array.prototype.slice.call(args));
            }

            function StatementSync(connId, stmtId) {
                this._conn = connId;
                this._id = stmtId;
            }
            StatementSync.prototype.run = function() {
                var r = JSON.parse(__sqliteRun(this._id, _parseParams(arguments)));
                return r;
            };
            StatementSync.prototype.get = function() {
                var r = __sqliteGet(this._id, _parseParams(arguments));
                return r === 'null' ? undefined : JSON.parse(r);
            };
            StatementSync.prototype.all = function() {
                return JSON.parse(__sqliteAll(this._id, _parseParams(arguments)));
            };

            function DatabaseSync(path) {
                if (typeof path !== 'string') throw new TypeError('DatabaseSync requires a path string');
                this._id = __sqliteOpen(path);
            }
            DatabaseSync.prototype.exec = function(sql) {
                __sqliteExec(this._id, sql);
            };
            DatabaseSync.prototype.prepare = function(sql) {
                var sid = __sqlitePrepare(this._id, sql);
                return new StatementSync(this._id, sid);
            };
            DatabaseSync.prototype.close = function() {
                __sqliteClose(this._id);
            };

            var sqliteMod = { DatabaseSync: DatabaseSync, StatementSync: StatementSync };
            if (globalThis.__requireCache) {
                globalThis.__requireCache['node:sqlite'] = sqliteMod;
                globalThis.__requireCache['sqlite'] = sqliteMod;
            }
        })();
    "#;
    let source = V8String::new(scope, js_code).unwrap();
    let _ = Script::compile(scope, source, None).and_then(|s| s.run(scope));

    Ok(())
}
