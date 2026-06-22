//! IRC (Internet Relay Chat) client built-in module
//!
//! Provides: `require('irc')` with `Client` class
//!
//! Native functions:
//! - `__ircCreate(server, port, nick)` -> id
//! - `__ircConnect(id, host, port, useTls)`
//! - `__ircSend(id, line)`
//! - `__ircClose(id)`

use rquickjs::{Ctx, Function, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use vvva_permissions::{Capability, PermissionState};

type IrcId = u32;

struct IrcState {
    #[allow(dead_code)]
    server: String,
    #[allow(dead_code)]
    port: u16,
    #[allow(dead_code)]
    nick: String,
    connected: bool,
}

static IRC_REGISTRY: OnceLock<Mutex<HashMap<IrcId, IrcState>>> = OnceLock::new();

fn irc_registry() -> &'static Mutex<HashMap<IrcId, IrcState>> {
    IRC_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_irc_id() -> IrcId {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub fn inject_irc(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();
    let _perms = permissions.clone();

    let create_fn = Function::new(
        ctx.clone(),
        move |server: String, port: u16, nick: String| -> IrcId {
            let id = next_irc_id();
            irc_registry().lock().unwrap().insert(
                id,
                IrcState {
                    server,
                    port,
                    nick,
                    connected: false,
                },
            );
            id
        },
    )?;
    globals.set("__ircCreate", create_fn)?;

    let perms2 = permissions.clone();
    let connect_fn = Function::new(
        ctx.clone(),
        move |id: IrcId, host: String, _port: u16, _use_tls: bool| -> Option<String> {
            if !perms2.check(&Capability::Network(host.clone())) {
                return Some(format!("EACCES: permission denied (--allow-net={})", host));
            }
            let mut reg = irc_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                state.connected = true;
                None
            } else {
                Some("Invalid IRC ID".to_string())
            }
        },
    )?;
    globals.set("__ircConnect", connect_fn)?;

    let send_fn = Function::new(
        ctx.clone(),
        move |id: IrcId, _line: String| -> Option<String> {
            let reg = irc_registry().lock().unwrap();
            if let Some(_state) = reg.get(&id) {
                if !_state.connected {
                    return Some("Not connected".to_string());
                }
                None
            } else {
                Some("Invalid IRC ID".to_string())
            }
        },
    )?;
    globals.set("__ircSend", send_fn)?;

    let close_fn = Function::new(ctx.clone(), move |id: IrcId| -> bool {
        irc_registry().lock().unwrap().remove(&id);
        true
    })?;
    globals.set("__ircClose", close_fn)?;

    ctx.eval::<(), _>(r#"
    (function() {
        function Client(serverOrOptions, nickArg, options) {
            var opts = (serverOrOptions && typeof serverOrOptions === 'object')
                ? serverOrOptions : (options || {});
            var server = (typeof serverOrOptions === 'string') ? serverOrOptions
                : (opts.host || opts.server || 'localhost');
            var nickname = (typeof nickArg === 'string') ? nickArg : (opts.nick || 'user');
            this.server = server;
            this.port = opts.port || 6667;
            this.nickname = nickname;
            this.username = opts.username || nickname;
            this.realname = opts.realname || nickname;
            this.password = opts.password || null;
            this.secure = opts.secure || opts.tls || false;
            this.channels = opts.channels || [];
            this._id = __ircCreate(server, this.port, nickname);
            this._connected = false;
            this._handlers = {};
        }

        Client.prototype.connect = function(callback) {
            var err = __ircConnect(this._id, this.server, this.port, this.secure);
            if (err) { if (callback) callback(new Error(err)); return; }
            this._connected = true;
            if (callback) callback();
        };

        Client.prototype.send = Client.prototype.raw = function(line, callback) {
            if (this._connected) {
                var err = __ircSend(this._id, line);
                if (err) throw new Error(err);
            }
            if (callback) callback();
        };

        Client.prototype.nick = function(newnick, callback) {
            this.nickname = newnick;
            if (this._connected) this.send("NICK " + newnick);
            if (callback) callback();
        };

        Client.prototype.user = function(username, mode, realname, callback) {
            if (this._connected) this.send("USER " + username + " " + (mode||0) + " * :" + (realname||username));
            if (callback) callback();
        };

        Client.prototype.join = function(channel, key, callback) {
            var line = "JOIN " + channel;
            if (typeof key === 'string') line += " " + key;
            if (this._connected) this.send(line);
            if (typeof key === 'function') key();
            else if (callback) callback();
        };

        Client.prototype.part = function(channel, reason, callback) {
            var line = "PART " + channel;
            if (typeof reason === 'string') line += " :" + reason;
            if (this._connected) this.send(line);
            if (typeof reason === 'function') reason();
            else if (callback) callback();
        };

        Client.prototype.privmsg = function(target, text, callback) {
            if (this._connected) this.send("PRIVMSG " + target + " :" + text);
            if (callback) callback();
        };

        Client.prototype.notice = function(target, text, callback) {
            if (this._connected) this.send("NOTICE " + target + " :" + text);
            if (callback) callback();
        };

        Client.prototype.kick = function(channel, nick, reason, callback) {
            var line = "KICK " + channel + " " + nick;
            if (typeof reason === 'string') line += " :" + reason;
            if (this._connected) this.send(line);
            if (typeof reason === 'function') reason();
            else if (callback) callback();
        };

        Client.prototype.mode = function(target, mode, callback) {
            if (this._connected) this.send("MODE " + target + (mode ? " " + mode : ""));
            if (callback) callback();
        };

        Client.prototype.quit = function(reason, callback) {
            var line = "QUIT";
            if (typeof reason === 'string') line += " :" + reason;
            if (this._connected) { this.send(line); }
            this._connected = false;
            if (typeof reason === 'function') reason();
            else if (callback) callback();
        };

        Client.prototype.disconnect = function() {
            __ircClose(this._id);
            this._connected = false;
        };

        Client.prototype.on = Client.prototype.addListener = function(event, handler) {
            this._handlers[event] = this._handlers[event] || [];
            this._handlers[event].push(handler);
            return this;
        };

        Client.prototype.off = Client.prototype.removeListener = function(event, handler) {
            if (this._handlers[event]) {
                var idx = this._handlers[event].indexOf(handler);
                if (idx >= 0) this._handlers[event].splice(idx, 1);
            }
            return this;
        };

        Client.prototype.removeAllListeners = function(event) {
            if (event) this._handlers[event] = [];
            else this._handlers = {};
        };

        Client.prototype.emit = function(event) {
            var args = Array.prototype.slice.call(arguments, 1);
            (this._handlers[event] || []).forEach(function(h) { h.apply(null, args); });
        };

        globalThis.__requireCache = globalThis.__requireCache || {};
        globalThis.__requireCache['irc'] = { Client: Client };
        globalThis.__requireCache['node:irc'] = { Client: Client };
        globalThis.irc = { Client: Client };
    })();
    "#)?;

    Ok(())
}
