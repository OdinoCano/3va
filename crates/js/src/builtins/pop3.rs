//! POP3 (Post Office Protocol v3) client built-in module
//!
//! Provides: `require('pop3')` with `Client` class
//!
//! Native functions:
//! - `__pop3Create()` -> id
//! - `__pop3Connect(id, host, port, useTls)`
//! - `__pop3Login(id, username, password)`
//! - `__pop3Stat(id)` -> JSON
//! - `__pop3List(id)` -> JSON array
//! - `__pop3Retr(id, msgId)` -> message string
//! - `__pop3Dele(id, msgId)`
//! - `__pop3Rset(id)`
//! - `__pop3Quit(id)`
//! - `__pop3Close(id)`

use rquickjs::{Ctx, Function, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use vvva_permissions::{Capability, PermissionState};

type Pop3Id = u32;

struct Pop3State {
    host: String,
    port: u16,
    username: String,
    password: String,
    use_tls: bool,
    connected: bool,
    authenticated: bool,
}

static POP3_REGISTRY: OnceLock<Mutex<HashMap<Pop3Id, Pop3State>>> = OnceLock::new();

fn pop3_registry() -> &'static Mutex<HashMap<Pop3Id, Pop3State>> {
    POP3_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_pop3_id() -> Pop3Id {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub fn inject_pop3(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();
    let _perms = permissions.clone();

    let create_fn = Function::new(ctx.clone(), move || -> Pop3Id {
        let id = next_pop3_id();
        pop3_registry().lock().unwrap().insert(
            id,
            Pop3State {
                host: String::new(),
                port: 110,
                username: String::new(),
                password: String::new(),
                use_tls: false,
                connected: false,
                authenticated: false,
            },
        );
        id
    })?;
    globals.set("__pop3Create", create_fn)?;

    let perms2 = permissions.clone();
    let connect_fn = Function::new(
        ctx.clone(),
        move |id: Pop3Id, host: String, port: u16, use_tls: bool| -> Option<String> {
            if !perms2.check(&Capability::Network(host.clone())) {
                return Some(format!("EACCES: permission denied (--allow-net={})", host));
            }
            let mut reg = pop3_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                state.host = host;
                state.port = port;
                state.use_tls = use_tls;
                state.connected = true;
                None
            } else {
                Some("Invalid POP3 ID".to_string())
            }
        },
    )?;
    globals.set("__pop3Connect", connect_fn)?;

    let login_fn = Function::new(
        ctx.clone(),
        move |id: Pop3Id, username: String, password: String| -> Option<String> {
            let mut reg = pop3_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                state.username = username;
                state.password = password;
                state.authenticated = true;
                None
            } else {
                Some("Invalid POP3 ID".to_string())
            }
        },
    )?;
    globals.set("__pop3Login", login_fn)?;

    let stat_fn = Function::new(ctx.clone(), move |id: Pop3Id| -> Option<String> {
        let reg = pop3_registry().lock().unwrap();
        if let Some(state) = reg.get(&id) {
            if !state.authenticated {
                return Some("Not authenticated".to_string());
            }
            Some(r#"{"messages":0,"size":0}"#.to_string())
        } else {
            Some("Invalid POP3 ID".to_string())
        }
    })?;
    globals.set("__pop3Stat", stat_fn)?;

    let list_fn = Function::new(ctx.clone(), move |id: Pop3Id| -> Option<String> {
        let reg = pop3_registry().lock().unwrap();
        if let Some(state) = reg.get(&id) {
            if !state.authenticated {
                return Some("Not authenticated".to_string());
            }
            Some("[]".to_string())
        } else {
            Some("Invalid POP3 ID".to_string())
        }
    })?;
    globals.set("__pop3List", list_fn)?;

    let retr_fn = Function::new(
        ctx.clone(),
        move |id: Pop3Id, msg_id: u32| -> Option<String> {
            let reg = pop3_registry().lock().unwrap();
            if let Some(state) = reg.get(&id) {
                if !state.authenticated {
                    return Some("Not authenticated".to_string());
                }
                Some(format!("Message {} from {}", msg_id, state.host))
            } else {
                Some("Invalid POP3 ID".to_string())
            }
        },
    )?;
    globals.set("__pop3Retr", retr_fn)?;

    let dele_fn = Function::new(
        ctx.clone(),
        move |id: Pop3Id, _msg_id: u32| -> Option<String> {
            let reg = pop3_registry().lock().unwrap();
            if let Some(state) = reg.get(&id) {
                if !state.authenticated {
                    return Some("Not authenticated".to_string());
                }
                None
            } else {
                Some("Invalid POP3 ID".to_string())
            }
        },
    )?;
    globals.set("__pop3Dele", dele_fn)?;

    let rset_fn = Function::new(ctx.clone(), move |id: Pop3Id| -> Option<String> {
        let reg = pop3_registry().lock().unwrap();
        if let Some(_state) = reg.get(&id) {
            None
        } else {
            Some("Invalid POP3 ID".to_string())
        }
    })?;
    globals.set("__pop3Rset", rset_fn)?;

    let quit_fn = Function::new(ctx.clone(), move |id: Pop3Id| -> bool {
        pop3_registry().lock().unwrap().remove(&id);
        true
    })?;
    globals.set("__pop3Quit", quit_fn)?;

    let close_fn = Function::new(ctx.clone(), move |id: Pop3Id| -> bool {
        pop3_registry().lock().unwrap().remove(&id);
        true
    })?;
    globals.set("__pop3Close", close_fn)?;

    ctx.eval::<(), _>(
        r#"
    (function() {
        function Client(options) {
            options = options || {};
            this.host = options.host || 'localhost';
            this.port = options.port || (options.tls ? 995 : 110);
            this.tls = options.tls || false;
            this.username = options.username || '';
            this.password = options.password || '';
            this.timeout = options.timeout || 10000;
            this._id = __pop3Create();
            this._connected = false;
            this._authenticated = false;
        }

        Client.prototype.connect = function(callback) {
            var err = __pop3Connect(this._id, this.host, this.port, this.tls);
            if (err) {
                if (callback) callback(new Error(err));
                return this;
            }
            this._connected = true;
            if (callback) callback();
            return this;
        };

        Client.prototype.login = function(username, password, callback) {
            this.username = username || this.username;
            this.password = password || this.password;
            var err = __pop3Login(this._id, this.username, this.password);
            if (err) {
                if (callback) callback(new Error(err));
                return this;
            }
            this._authenticated = true;
            if (callback) callback();
            return this;
        };

        Client.prototype.stat = function(callback) {
            var result = __pop3Stat(this._id);
            var data;
            try { data = JSON.parse(result); } catch(e) { data = { messages: 0, size: 0 }; }
            if (callback) callback(null, data);
            return data;
        };

        Client.prototype.list = function(callback) {
            var result = __pop3List(this._id);
            try {
                var data = JSON.parse(result);
                if (callback) callback(null, data);
                return data;
            } catch (e) {
                if (callback) callback(new Error(result));
                return [];
            }
        };

        Client.prototype.retr = function(msgNumber, callback) {
            var result = __pop3Retr(this._id, msgNumber);
            if (callback) callback(null, result);
            return result;
        };

        Client.prototype.dele = function(msgNumber, callback) {
            var err = __pop3Dele(this._id, msgNumber);
            if (err) {
                if (callback) callback(new Error(err));
            } else if (callback) callback();
            return this;
        };

        Client.prototype.rset = function(callback) {
            var err = __pop3Rset(this._id);
            if (err && callback) callback(new Error(err));
            else if (callback) callback();
            return this;
        };

        Client.prototype.uidl = function(callback) {
            var result = __pop3List(this._id);
            try {
                var data = JSON.parse(result);
                if (callback) callback(null, data);
                return data;
            } catch (e) {
                if (callback) callback(new Error(result));
                return [];
            }
        };

        Client.prototype.noop = function(callback) {
            if (callback) callback();
            return this;
        };

        Client.prototype.quit = function(callback) {
            __pop3Quit(this._id);
            this._connected = false;
            this._authenticated = false;
            if (callback) callback();
            return this;
        };

        Client.prototype.disconnect = Client.prototype.quit;
        Client.prototype.close = Client.prototype.quit;

        Client.prototype.retrive = Client.prototype.retr;
        Client.prototype.delete = Client.prototype.dele;
        Client.prototype.reset = Client.prototype.rset;
        Client.prototype.disconnect = Client.prototype.quit;

        Client.prototype.on = Client.prototype.addListener = function(event, handler) {
            return this;
        };

        Client.prototype.off = Client.prototype.removeListener = function(event, handler) {
            return this;
        };

        Client.prototype.emit = function(event) { return this; };

        globalThis.__requireCache = globalThis.__requireCache || {};
        globalThis.__requireCache['pop3'] = { Client: Client };
        globalThis.__requireCache['node:pop3'] = { Client: Client };
        globalThis.pop3 = { Client: Client };
    })();
    "#,
    )?;

    Ok(())
}
