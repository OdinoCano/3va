use rquickjs::{Ctx, Function, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type ConnMap = Arc<Mutex<HashMap<u32, Arc<Mutex<rusqlite::Connection>>>>>;
type StmtMap = Arc<Mutex<HashMap<u32, (u32, String)>>>; // stmt_id → (conn_id, sql)

pub fn inject_sqlite(ctx: &Ctx) -> Result<()> {
    let conns: ConnMap = Arc::new(Mutex::new(HashMap::new()));
    let stmts: StmtMap = Arc::new(Mutex::new(HashMap::new()));
    let conn_counter = Arc::new(Mutex::new(0u32));
    let stmt_counter = Arc::new(Mutex::new(0u32));

    // __sqliteOpen(path) → conn_id
    let conns2 = conns.clone();
    let counter2 = conn_counter.clone();
    ctx.globals().set(
        "__sqliteOpen",
        Function::new(ctx.clone(), move |path: String| -> rquickjs::Result<u32> {
            let conn = rusqlite::Connection::open(&path).map_err(|e| {
                rquickjs::Error::new_from_js_message("sqlite", "open", e.to_string())
            })?;
            let mut id = counter2.lock().unwrap();
            *id += 1;
            let cid = *id;
            conns2
                .lock()
                .unwrap()
                .insert(cid, Arc::new(Mutex::new(conn)));
            Ok(cid)
        })?,
    )?;

    // __sqliteClose(conn_id)
    let conns2 = conns.clone();
    ctx.globals().set(
        "__sqliteClose",
        Function::new(ctx.clone(), move |id: u32| {
            conns2.lock().unwrap().remove(&id);
        })?,
    )?;

    // __sqliteExec(conn_id, sql) → void
    let conns2 = conns.clone();
    ctx.globals().set(
        "__sqliteExec",
        Function::new(
            ctx.clone(),
            move |id: u32, sql: String| -> rquickjs::Result<()> {
                let map = conns2.lock().unwrap();
                let conn = map.get(&id).ok_or_else(|| {
                    rquickjs::Error::new_from_js_message("sqlite", "exec", "unknown connection")
                })?;
                conn.lock().unwrap().execute_batch(&sql).map_err(|e| {
                    rquickjs::Error::new_from_js_message("sqlite", "exec", e.to_string())
                })
            },
        )?,
    )?;

    // __sqlitePrepare(conn_id, sql) → stmt_id
    let stmts2 = stmts.clone();
    let scounter = stmt_counter.clone();
    let conns2 = conns.clone();
    ctx.globals().set(
        "__sqlitePrepare",
        Function::new(
            ctx.clone(),
            move |conn_id: u32, sql: String| -> rquickjs::Result<u32> {
                // Validate the SQL parses (compile once)
                {
                    let map = conns2.lock().unwrap();
                    let conn = map.get(&conn_id).ok_or_else(|| {
                        rquickjs::Error::new_from_js_message(
                            "sqlite",
                            "prepare",
                            "unknown connection",
                        )
                    })?;
                    let _ = conn.lock().unwrap().prepare(&sql).map_err(|e| {
                        rquickjs::Error::new_from_js_message("sqlite", "prepare", e.to_string())
                    })?;
                }
                let mut id = scounter.lock().unwrap();
                *id += 1;
                let sid = *id;
                stmts2.lock().unwrap().insert(sid, (conn_id, sql));
                Ok(sid)
            },
        )?,
    )?;

    // __sqliteRun(stmt_id, params_json) → "{changes, lastInsertRowid}"
    let stmts2 = stmts.clone();
    let conns2 = conns.clone();
    ctx.globals().set(
        "__sqliteRun",
        Function::new(
            ctx.clone(),
            move |stmt_id: u32, params_json: String| -> rquickjs::Result<String> {
                let (conn_id, sql) = {
                    let map = stmts2.lock().unwrap();
                    let e = map.get(&stmt_id).ok_or_else(|| {
                        rquickjs::Error::new_from_js_message("sqlite", "run", "unknown statement")
                    })?;
                    e.clone()
                };
                let params: Vec<serde_json::Value> =
                    serde_json::from_str(&params_json).unwrap_or_default();
                let map = conns2.lock().unwrap();
                let conn = map.get(&conn_id).ok_or_else(|| {
                    rquickjs::Error::new_from_js_message("sqlite", "run", "unknown connection")
                })?;
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
                let changes = conn.execute(&sql, params_refs.as_slice()).map_err(|e| {
                    rquickjs::Error::new_from_js_message("sqlite", "run", e.to_string())
                })?;
                let rowid = conn.last_insert_rowid();
                Ok(format!(
                    "{{\"changes\":{},\"lastInsertRowid\":{}}}",
                    changes, rowid
                ))
            },
        )?,
    )?;

    // __sqliteGet(stmt_id, params_json) → JSON row or "null"
    let stmts2 = stmts.clone();
    let conns2 = conns.clone();
    ctx.globals().set(
        "__sqliteGet",
        Function::new(
            ctx.clone(),
            move |stmt_id: u32, params_json: String| -> rquickjs::Result<String> {
                let (conn_id, sql) = {
                    let map = stmts2.lock().unwrap();
                    let e = map.get(&stmt_id).ok_or_else(|| {
                        rquickjs::Error::new_from_js_message("sqlite", "get", "unknown statement")
                    })?;
                    e.clone()
                };
                let params: Vec<serde_json::Value> =
                    serde_json::from_str(&params_json).unwrap_or_default();
                let map = conns2.lock().unwrap();
                let conn = map.get(&conn_id).ok_or_else(|| {
                    rquickjs::Error::new_from_js_message("sqlite", "get", "unknown connection")
                })?;
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
                let mut stmt = conn.prepare(&sql).map_err(|e| {
                    rquickjs::Error::new_from_js_message("sqlite", "get", e.to_string())
                })?;
                let cols: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
                let mut rows = stmt.query(params_refs.as_slice()).map_err(|e| {
                    rquickjs::Error::new_from_js_message("sqlite", "get", e.to_string())
                })?;
                match rows.next().map_err(|e| {
                    rquickjs::Error::new_from_js_message("sqlite", "get", e.to_string())
                })? {
                    None => Ok("null".to_string()),
                    Some(row) => {
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
                                    rusqlite::types::Value::Blob(b) => {
                                        serde_json::Value::String(base64::Engine::encode(
                                            &base64::engine::general_purpose::STANDARD,
                                            b,
                                        ))
                                    }
                                },
                            );
                        }
                        Ok(serde_json::to_string(&obj).unwrap_or_else(|_| "null".to_string()))
                    }
                }
            },
        )?,
    )?;

    // __sqliteAll(stmt_id, params_json) → JSON array
    let stmts2 = stmts.clone();
    let conns2 = conns.clone();
    ctx.globals().set(
        "__sqliteAll",
        Function::new(
            ctx.clone(),
            move |stmt_id: u32, params_json: String| -> rquickjs::Result<String> {
                let (conn_id, sql) = {
                    let map = stmts2.lock().unwrap();
                    let e = map.get(&stmt_id).ok_or_else(|| {
                        rquickjs::Error::new_from_js_message("sqlite", "all", "unknown statement")
                    })?;
                    e.clone()
                };
                let params: Vec<serde_json::Value> =
                    serde_json::from_str(&params_json).unwrap_or_default();
                let map = conns2.lock().unwrap();
                let conn = map.get(&conn_id).ok_or_else(|| {
                    rquickjs::Error::new_from_js_message("sqlite", "all", "unknown connection")
                })?;
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
                let mut stmt = conn.prepare(&sql).map_err(|e| {
                    rquickjs::Error::new_from_js_message("sqlite", "all", e.to_string())
                })?;
                let cols: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
                let mut rows = stmt.query(params_refs.as_slice()).map_err(|e| {
                    rquickjs::Error::new_from_js_message("sqlite", "all", e.to_string())
                })?;
                let mut result = Vec::new();
                while let Some(row) = rows.next().map_err(|e| {
                    rquickjs::Error::new_from_js_message("sqlite", "all", e.to_string())
                })? {
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
                                rusqlite::types::Value::Blob(b) => {
                                    serde_json::Value::String(base64::Engine::encode(
                                        &base64::engine::general_purpose::STANDARD,
                                        b,
                                    ))
                                }
                            },
                        );
                    }
                    result.push(serde_json::Value::Object(obj));
                }
                Ok(serde_json::to_string(&result).unwrap_or_else(|_| "[]".to_string()))
            },
        )?,
    )?;

    // JS wrapper — registers node:sqlite in __requireCache
    ctx.eval::<(), _>(
        r#"
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
    "#,
    )?;

    Ok(())
}
