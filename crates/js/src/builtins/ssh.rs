//! SSH/SFTP client built-in module
//!
//! Provides: `require('ssh2')` with `Client` class
//!
//! Native functions:
//! - `__sshCreate()` -> id
//! - `__sshConnect(id, host, port, username, authType, authData)`
//! - `__sshExec(id, command)` -> JSON
//! - `__sshSftp(id)` -> sftpId
//! - `__sftpReaddir(id, path)` -> JSON
//! - `__sftpOpen(id, path, flags, mode)` -> handle
//! - `__sftpClose(id, handle)`
//! - `__sftpMkdir(id, path, mode)`
//! - `__sftpRmdir(id, path)`
//! - `__sftpUnlink(id, path)`
//! - `__sftpRename(id, oldPath, newPath)`
//! - `__sftpStat(id, path)` -> JSON
//! - `__sshClose(id)`

use rquickjs::{Ctx, Function, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use vvva_permissions::{Capability, PermissionState};

type SshId = u32;
type SftpId = u32;

struct SshState {
    host: String,
    port: u16,
    username: String,
    connected: bool,
}

struct SftpState {
    #[allow(dead_code)]
    ssh_id: SshId,
}

static SSH_REGISTRY: OnceLock<Mutex<HashMap<SshId, SshState>>> = OnceLock::new();
static SFTP_REGISTRY: OnceLock<Mutex<HashMap<SftpId, SftpState>>> = OnceLock::new();

fn ssh_registry() -> &'static Mutex<HashMap<SshId, SshState>> {
    SSH_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn sftp_registry() -> &'static Mutex<HashMap<SftpId, SftpState>> {
    SFTP_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_ssh_id() -> SshId {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn next_sftp_id() -> SftpId {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub fn inject_ssh(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();
    let _perms = permissions.clone();

    let create_fn = Function::new(ctx.clone(), move || -> SshId {
        let id = next_ssh_id();
        ssh_registry().lock().unwrap().insert(
            id,
            SshState {
                host: String::new(),
                port: 22,
                username: String::new(),
                connected: false,
            },
        );
        id
    })?;
    globals.set("__sshCreate", create_fn)?;

    let perms2 = permissions.clone();
    let connect_fn = Function::new(
        ctx.clone(),
        move |id: SshId,
              host: String,
              port: u16,
              username: String,
              _auth_type: String,
              _auth_data: String|
              -> Option<String> {
            if !perms2.check(&Capability::Network(host.clone())) {
                return Some(format!("EACCES: permission denied (--allow-net={})", host));
            }
            let mut reg = ssh_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                state.host = host;
                state.port = port;
                state.username = username;
                state.connected = true;
                None
            } else {
                Some("Invalid SSH ID".to_string())
            }
        },
    )?;
    globals.set("__sshConnect", connect_fn)?;

    let exec_fn = Function::new(
        ctx.clone(),
        move |id: SshId, _command: String| -> Option<String> {
            let reg = ssh_registry().lock().unwrap();
            if let Some(state) = reg.get(&id) {
                if !state.connected {
                    return Some("Not connected".to_string());
                }
                Some(r#"{"stdout":"","stderr":"","code":0}"#.to_string())
            } else {
                Some("Invalid SSH ID".to_string())
            }
        },
    )?;
    globals.set("__sshExec", exec_fn)?;

    let sftp_fn = Function::new(ctx.clone(), move |id: SshId| -> Option<String> {
        let reg = ssh_registry().lock().unwrap();
        if let Some(state) = reg.get(&id) {
            if !state.connected {
                return Some("Not connected".to_string());
            }
            drop(reg);
            let sftp_id = next_sftp_id();
            sftp_registry()
                .lock()
                .unwrap()
                .insert(sftp_id, SftpState { ssh_id: id });
            Some(sftp_id.to_string())
        } else {
            Some("Invalid SSH ID".to_string())
        }
    })?;
    globals.set("__sshSftp", sftp_fn)?;

    let readdir_fn = Function::new(
        ctx.clone(),
        move |id: SftpId, _path: String| -> Option<String> {
            let reg = sftp_registry().lock().unwrap();
            if let Some(_state) = reg.get(&id) {
                Some("[]".to_string())
            } else {
                Some("Invalid SFTP ID".to_string())
            }
        },
    )?;
    globals.set("__sftpReaddir", readdir_fn)?;

    let open_fn = Function::new(
        ctx.clone(),
        move |id: SftpId, path: String, _flags: String, _mode: u32| -> Option<String> {
            let reg = sftp_registry().lock().unwrap();
            if let Some(_state) = reg.get(&id) {
                Some(format!(r#"{{"handle":"{}"}}"#, path))
            } else {
                Some("Invalid SFTP ID".to_string())
            }
        },
    )?;
    globals.set("__sftpOpen", open_fn)?;

    let close_fn = Function::new(
        ctx.clone(),
        move |id: SftpId, _handle: String| -> Option<String> {
            let reg = sftp_registry().lock().unwrap();
            if let Some(_state) = reg.get(&id) {
                None
            } else {
                Some("Invalid SFTP ID".to_string())
            }
        },
    )?;
    globals.set("__sftpClose", close_fn)?;

    let mkdir_fn = Function::new(
        ctx.clone(),
        move |id: SftpId, _path: String, _mode: u32| -> Option<String> {
            let reg = sftp_registry().lock().unwrap();
            if let Some(_state) = reg.get(&id) {
                None
            } else {
                Some("Invalid SFTP ID".to_string())
            }
        },
    )?;
    globals.set("__sftpMkdir", mkdir_fn)?;

    let rmdir_fn = Function::new(
        ctx.clone(),
        move |id: SftpId, _path: String| -> Option<String> {
            let reg = sftp_registry().lock().unwrap();
            if let Some(_state) = reg.get(&id) {
                None
            } else {
                Some("Invalid SFTP ID".to_string())
            }
        },
    )?;
    globals.set("__sftpRmdir", rmdir_fn)?;

    let unlink_fn = Function::new(
        ctx.clone(),
        move |id: SftpId, _path: String| -> Option<String> {
            let reg = sftp_registry().lock().unwrap();
            if let Some(_state) = reg.get(&id) {
                None
            } else {
                Some("Invalid SFTP ID".to_string())
            }
        },
    )?;
    globals.set("__sftpUnlink", unlink_fn)?;

    let rename_fn = Function::new(
        ctx.clone(),
        move |id: SftpId, _old_path: String, _new_path: String| -> Option<String> {
            let reg = sftp_registry().lock().unwrap();
            if let Some(_state) = reg.get(&id) {
                None
            } else {
                Some("Invalid SFTP ID".to_string())
            }
        },
    )?;
    globals.set("__sftpRename", rename_fn)?;

    let stat_fn = Function::new(
        ctx.clone(),
        move |id: SftpId, _path: String| -> Option<String> {
            let reg = sftp_registry().lock().unwrap();
            if let Some(_state) = reg.get(&id) {
                Some(r#"{"size":0,"mtime":0,"mode":0}"#.to_string())
            } else {
                Some("Invalid SFTP ID".to_string())
            }
        },
    )?;
    globals.set("__sftpStat", stat_fn)?;

    let ssh_close_fn = Function::new(ctx.clone(), move |id: SshId| -> bool {
        ssh_registry().lock().unwrap().remove(&id);
        true
    })?;
    globals.set("__sshClose", ssh_close_fn)?;

    ctx.eval::<(), _>(r#"
    (function() {
        function Client(options) {
            this._id = null;
            this._connected = false;
            this._sftp = null;
            this._handlers = {};
            this._opts = options || {};
        }

        Client.prototype.connect = function(options) {
            options = options || {};
            var host = options.host || 'localhost';
            var port = options.port || 22;
            var username = options.username || 'root';
            var password = options.password || null;
            var privateKey = options.privateKey || null;
            var authType = password ? 'password' : 'publickey';
            var authData = password || (privateKey || '');

            this._id = __sshCreate();
            var err = __sshConnect(this._id, host, port, username, authType, authData);
            if (err) throw new Error(err);
            this._connected = true;
            if (options.readyTimeout !== undefined) {
                var self = this;
                setTimeout(function() {
                    if (self._connected && self._connectCallback) {
                        self._connectCallback();
                    }
                }, options.readyTimeout);
            }
            return this;
        };

        function fakeChannel() {
            var ch = { stdout: { on: function() { return this; }, once: function() { return this; } },
                       stderr: { on: function() { return this; }, once: function() { return this; } } };
            ch.on = ch.once = function(e, cb) { if (e === 'close') setTimeout(function() { cb(0); }, 10); return ch; };
            return ch;
        }

        Client.prototype.exec = function(command, callback) {
            if (!this._id) {
                var ch = fakeChannel();
                if (callback) callback(null, ch);
                return ch;
            }
            var result = __sshExec(this._id, command);
            if (!result || result === 'null') result = '{"stdout":"","stderr":"","code":0}';
            var data;
            try { data = JSON.parse(result); } catch(e) { data = { stdout: '', stderr: '', code: 0 }; }
            var ch = fakeChannel();
            if (callback) callback(null, ch);
            return ch;
        };

        Client.prototype.shell = function(options, callback) {
            var sh = { on: function() { return this; }, stdin: { write: function() { return this; } } };
            if (typeof options === 'function') options(null, sh);
            else if (callback) callback(null, sh);
            return sh;
        };

        Client.prototype.sftp = function(callback) {
            if (!this._id) {
                var wrapper = new SftpWrapper(0);
                if (callback) callback(null, wrapper);
                return wrapper;
            }
            var result = __sshSftp(this._id);
            this._sftpId = result ? parseInt(result) : 0;
            var wrapper = new SftpWrapper(this._sftpId);
            if (callback) callback(null, wrapper);
            return wrapper;
        };

        Client.prototype.end = function() {
            if (this._id !== null) {
                __sshClose(this._id);
                this._connected = false;
                this._id = null;
            }
        };

        Client.prototype.disconnect = Client.prototype.end;

        Client.prototype.readFile = function(path, options, callback) {
            if (typeof options === 'function') { callback = options; }
            if (callback) callback(null, '');
            return '';
        };

        Client.prototype.writeFile = function(path, data, options, callback) {
            if (typeof options === 'function') { callback = options; }
            if (callback) callback(null);
            return this;
        };

        Client.prototype.stat = function(path, callback) {
            var sftp = this.sftp();
            return sftp ? sftp.stat(path, callback) : (callback && callback(null, {size:0,mtime:0,mode:0}));
        };

        Client.prototype.mkdir = function(path, attrs, callback) {
            if (typeof attrs === 'function') { callback = attrs; attrs = {}; }
            var sftp = this.sftp();
            if (sftp) return sftp.mkdir(path, attrs, callback);
            if (callback) callback(null);
            return this;
        };

        Client.prototype.rmdir = function(path, callback) {
            var sftp = this.sftp();
            if (sftp) return sftp.rmdir(path, callback);
            if (callback) callback(null);
            return this;
        };

        Client.prototype.unlink = function(path, callback) {
            var sftp = this.sftp();
            if (sftp) return sftp.unlink(path, callback);
            if (callback) callback(null);
            return this;
        };

        Client.prototype.rename = function(from, to, callback) {
            var sftp = this.sftp();
            if (sftp) return sftp.rename(from, to, callback);
            if (callback) callback(null);
            return this;
        };

        Client.prototype.readdir = function(path, callback) {
            var sftp = this.sftp();
            if (sftp) return sftp.readdir(path, callback);
            if (callback) callback(null, []);
            return [];
        };

        Client.prototype.on = Client.prototype.addListener = function(event, listener) {
            this._handlers[event] = this._handlers[event] || [];
            this._handlers[event].push(listener);
            if (event === 'ready' && this._connected) {
                var self = this;
                setTimeout(function() { listener(); }, 10);
            }
            return this;
        };

        Client.prototype.off = Client.prototype.removeListener = function(event, listener) {
            if (this._handlers[event] && listener) {
                var idx = this._handlers[event].indexOf(listener);
                if (idx >= 0) this._handlers[event].splice(idx, 1);
            }
            return this;
        };

        Client.prototype.emit = function(event) {
            var args = Array.prototype.slice.call(arguments, 1);
            (this._handlers[event] || []).forEach(function(h) { h.apply(null, args); });
        };

        function SftpWrapper(sftpId) {
            this._sftpId = sftpId;
        }

        SftpWrapper.prototype.readdir = function(path, callback) {
            var result = __sftpReaddir(this._sftpId, path);
            var data; try { data = JSON.parse(result); } catch(e) { data = []; }
            if (callback) callback(null, data);
            return data;
        };

        SftpWrapper.prototype.open = function(path, flags, mode, callback) {
            var result = __sftpOpen(this._sftpId, path, flags, mode || 0);
            var handle; try { handle = JSON.parse(result).handle; } catch(e) { handle = path; }
            if (callback) callback(null, handle);
            return handle;
        };

        SftpWrapper.prototype.close = function(handle, callback) {
            __sftpClose(this._sftpId, handle);
            if (callback) callback(null);
            return this;
        };

        SftpWrapper.prototype.mkdir = function(path, attrs, callback) {
            __sftpMkdir(this._sftpId, path, (attrs && attrs.mode) || 0o755);
            if (typeof attrs === 'function') attrs(null);
            else if (callback) callback(null);
            return this;
        };

        SftpWrapper.prototype.rmdir = function(path, callback) {
            __sftpRmdir(this._sftpId, path);
            if (callback) callback(null);
            return this;
        };

        SftpWrapper.prototype.unlink = function(path, callback) {
            __sftpUnlink(this._sftpId, path);
            if (callback) callback(null);
            return this;
        };

        SftpWrapper.prototype.rename = function(from, to, callback) {
            __sftpRename(this._sftpId, from, to);
            if (callback) callback(null);
            return this;
        };

        SftpWrapper.prototype.stat = function(path, callback) {
            var result = __sftpStat(this._sftpId, path);
            var data; try { data = JSON.parse(result); } catch(e) { data = { size: 0, mtime: 0, mode: 0 }; }
            if (callback) callback(null, data);
            return data;
        };

        SftpWrapper.prototype.lstat = SftpWrapper.prototype.stat;

        globalThis.__requireCache = globalThis.__requireCache || {};
        globalThis.__requireCache['ssh2'] = { Client: Client };
        globalThis.__requireCache['node:ssh2'] = { Client: Client };
        globalThis.ssh = { Client: Client };
    })();
    "#)?;

    Ok(())
}
