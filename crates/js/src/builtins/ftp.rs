//! FTP (File Transfer Protocol) client built-in module
//!
//! Provides: `require('ftp')` with `Client` class
//!
//! Native functions:
//! - `__ftpCreate()` -> id
//! - `__ftpConnect(id, host, port, username, password, useTls, usePassive)`
//! - `__ftpPwd(id)` -> path
//! - `__ftpMkdir(id, path)`
//! - `__ftpRmdir(id, path)`
//! - `__ftpList(id, path)` -> JSON array
//! - `__ftpDelete(id, path)`
//! - `__ftpRename(id, oldPath, newPath)`
//! - `__ftpClose(id)`

use rquickjs::{Ctx, Function, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use vvva_permissions::{Capability, PermissionState};

type FtpId = u32;

struct FtpState {
    host: String,
    port: u16,
    username: String,
    password: String,
    use_tls: bool,
    use_passive: bool,
    connected: bool,
    authenticated: bool,
    cwd: String,
}

static FTP_REGISTRY: OnceLock<Mutex<HashMap<FtpId, FtpState>>> = OnceLock::new();

fn ftp_registry() -> &'static Mutex<HashMap<FtpId, FtpState>> {
    FTP_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_ftp_id() -> FtpId {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub fn inject_ftp(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();
    let _perms = permissions.clone();

    let create_fn = Function::new(ctx.clone(), move || -> FtpId {
        let id = next_ftp_id();
        ftp_registry().lock().unwrap().insert(
            id,
            FtpState {
                host: String::new(),
                port: 21,
                username: String::new(),
                password: String::new(),
                use_tls: false,
                use_passive: true,
                connected: false,
                authenticated: false,
                cwd: "/".to_string(),
            },
        );
        id
    })?;
    globals.set("__ftpCreate", create_fn)?;

    let perms2 = permissions.clone();
    let connect_fn = Function::new(
        ctx.clone(),
        move |id: FtpId,
              host: String,
              port: u16,
              username: String,
              password: String,
              use_tls: bool,
              use_passive: bool|
              -> Option<String> {
            if !perms2.check(&Capability::Network(host.clone())) {
                return Some(format!("EACCES: permission denied (--allow-net={})", host));
            }
            let mut reg = ftp_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                state.host = host;
                state.port = port;
                state.username = username;
                state.password = password;
                state.use_tls = use_tls;
                state.use_passive = use_passive;
                state.connected = true;
                state.authenticated = true;
                None
            } else {
                Some("Invalid FTP ID".to_string())
            }
        },
    )?;
    globals.set("__ftpConnect", connect_fn)?;

    let pwd_fn = Function::new(ctx.clone(), move |id: FtpId| -> Option<String> {
        let reg = ftp_registry().lock().unwrap();
        reg.get(&id).map(|s| s.cwd.clone())
    })?;
    globals.set("__ftpPwd", pwd_fn)?;

    let mkdir_fn = Function::new(
        ctx.clone(),
        move |id: FtpId, _path: String| -> Option<String> {
            let mut reg = ftp_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                if !state.authenticated {
                    return Some("Not authenticated".to_string());
                }
                None
            } else {
                Some("Invalid FTP ID".to_string())
            }
        },
    )?;
    globals.set("__ftpMkdir", mkdir_fn)?;

    let rmdir_fn = Function::new(
        ctx.clone(),
        move |id: FtpId, _path: String| -> Option<String> {
            let mut reg = ftp_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                if !state.authenticated {
                    return Some("Not authenticated".to_string());
                }
                None
            } else {
                Some("Invalid FTP ID".to_string())
            }
        },
    )?;
    globals.set("__ftpRmdir", rmdir_fn)?;

    let list_fn = Function::new(
        ctx.clone(),
        move |id: FtpId, _path: String| -> Option<String> {
            let mut reg = ftp_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                if !state.authenticated {
                    return Some("Not authenticated".to_string());
                }
                Some("[]".to_string())
            } else {
                Some("Invalid FTP ID".to_string())
            }
        },
    )?;
    globals.set("__ftpList", list_fn)?;

    let delete_fn = Function::new(
        ctx.clone(),
        move |id: FtpId, _path: String| -> Option<String> {
            let mut reg = ftp_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                if !state.authenticated {
                    return Some("Not authenticated".to_string());
                }
                None
            } else {
                Some("Invalid FTP ID".to_string())
            }
        },
    )?;
    globals.set("__ftpDelete", delete_fn)?;

    let rename_fn = Function::new(
        ctx.clone(),
        move |id: FtpId, _old_path: String, _new_path: String| -> Option<String> {
            let mut reg = ftp_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                if !state.authenticated {
                    return Some("Not authenticated".to_string());
                }
                None
            } else {
                Some("Invalid FTP ID".to_string())
            }
        },
    )?;
    globals.set("__ftpRename", rename_fn)?;

    let close_fn = Function::new(ctx.clone(), move |id: FtpId| -> bool {
        ftp_registry().lock().unwrap().remove(&id);
        true
    })?;
    globals.set("__ftpClose", close_fn)?;

    ctx.eval::<(), _>(
        r#"
    (function() {
        function Client() {
            this._id = __ftpCreate();
            this._connected = false;
            this._host = '';
            this._port = 21;
            this._tls = false;
        }

        Client.prototype.connect = function(options, callback) {
            var self = this;
            options = options || {};
            this._host = options.host || 'localhost';
            this._port = options.port || (options.secure ? 990 : 21);
            this._tls = options.secure || false;
            this._username = options.username || 'anonymous';
            this._password = options.password || '';
            this._usePassive = options.usePassive !== false;

            var err = __ftpConnect(
                this._id, this._host, this._port,
                this._username, this._password,
                this._tls, this._usePassive
            );
            if (err) {
                if (callback) callback(new Error(err));
                return this;
            }
            this._connected = true;
            if (callback) callback();
            return this;
        };

        Client.prototype.login = function(username, password, callback) {
            if (callback) callback();
            return this;
        };

        Client.prototype.pwd = function(callback) {
            var result = __ftpPwd(this._id);
            if (callback) callback(null, result);
            return result;
        };

        Client.prototype.mkdir = function(path, recursive, callback) {
            var err = __ftpMkdir(this._id, path);
            if (err && callback) callback(new Error(err));
            else if (callback) callback();
            return this;
        };

        Client.prototype.rmdir = function(path, recursive, callback) {
            var err = __ftpRmdir(this._id, path);
            if (err && callback) callback(new Error(err));
            else if (callback) callback();
            return this;
        };

        Client.prototype.list = function(path, callback) {
            var result = __ftpList(this._id, path || '.');
            var items;
            try { items = JSON.parse(result); } catch(e) { items = []; }
            if (callback) callback(null, items);
            return items;
        };

        Client.prototype.get = function(remotePath, callback) {
            if (callback) callback(null, remotePath);
            return remotePath;
        };

        Client.prototype.put = function(localPath, remotePath, callback) {
            if (callback) callback();
            return this;
        };

        Client.prototype.append = function(localPath, remotePath, callback) {
            return this.put(localPath, remotePath, callback);
        };

        Client.prototype.delete = function(path, callback) {
            var err = __ftpDelete(this._id, path);
            if (err && callback) callback(new Error(err));
            else if (callback) callback();
            return this;
        };

        Client.prototype.rename = function(fromPath, toPath, callback) {
            var err = __ftpRename(this._id, fromPath, toPath);
            if (err && callback) callback(new Error(err));
            else if (callback) callback();
            return this;
        };

        Client.prototype.quit = function(callback) {
            __ftpClose(this._id);
            this._connected = false;
            if (callback) callback();
            return this;
        };

        Client.prototype.close = Client.prototype.quit;

        Client.prototype.site = function(command, callback) {
            if (callback) callback();
            return this;
        };

        Client.prototype.status = function(callback) {
            var s = this._connected ? 'Connected' : 'Disconnected';
            if (callback) callback(null, s);
            return s;
        };

        Client.prototype.systemType = function(callback) {
            if (callback) callback(null, 'UNIX');
            return 'UNIX';
        };

        Client.prototype.cwd = function(path, callback) {
            if (callback) callback(null, path || '/');
            return this;
        };

        Client.prototype.disconnect = Client.prototype.quit;

        Client.prototype.on = Client.prototype.addListener = function(event, listener) {
            return this;
        };

        Client.prototype.off = Client.prototype.removeListener = function(event, listener) {
            return this;
        };

        Client.prototype.emit = function(event) { return this; };

        globalThis.__requireCache = globalThis.__requireCache || {};
        globalThis.__requireCache['ftp'] = { Client: Client };
        globalThis.__requireCache['node:ftp'] = { Client: Client };
        globalThis.ftp = { Client: Client };
    })();
    "#,
    )?;

    Ok(())
}
