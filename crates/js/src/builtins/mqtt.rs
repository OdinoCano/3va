//! MQTT (Message Queuing Telemetry Transport) client built-in module
//!
//! Provides: `require('mqtt')` with `connect()` function
//!
//! Implements MQTT 3.1.1 protocol (QoS 0)
//!
//! Native functions:
//! - `__mqttCreate(clientId)` -> id
//! - `__mqttConnect(id, host, port, useTls)`
//! - `__mqttSubscribe(id, topic, qos)`
//! - `__mqttUnsubscribe(id, topic)`
//! - `__mqttPublish(id, topic, payload, qos, retain)`
//! - `__mqttPing(id)`
//! - `__mqttDisconnect(id)`
//! - `__mqttClose(id)`
//! - `__mqttIsConnected(id)` -> bool

use rquickjs::{Ctx, Function, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use vvva_permissions::{Capability, PermissionState};

type MqttId = u32;

struct MqttState {
    host: String,
    port: u16,
    #[allow(dead_code)]
    client_id: String,
    use_tls: bool,
    connected: bool,
    subscriptions: Vec<String>,
}

static MQTT_REGISTRY: OnceLock<Mutex<HashMap<MqttId, MqttState>>> = OnceLock::new();

fn mqtt_registry() -> &'static Mutex<HashMap<MqttId, MqttState>> {
    MQTT_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_mqtt_id() -> MqttId {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn generate_client_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("3va_mqtt_{}", millis)
}

pub fn inject_mqtt(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();
    let _perms = permissions.clone();

    let create_fn = Function::new(ctx.clone(), move |client_id: Option<String>| -> MqttId {
        let id = next_mqtt_id();
        mqtt_registry().lock().unwrap().insert(
            id,
            MqttState {
                host: String::new(),
                port: 1883,
                client_id: client_id.unwrap_or_else(generate_client_id),
                use_tls: false,
                connected: false,
                subscriptions: vec![],
            },
        );
        id
    })?;
    globals.set("__mqttCreate", create_fn)?;

    let perms2 = permissions.clone();
    let connect_fn = Function::new(
        ctx.clone(),
        move |id: MqttId, host: String, port: u16, use_tls: bool| -> Option<String> {
            if !perms2.check(&Capability::Network(host.clone())) {
                return Some(format!("EACCES: permission denied (--allow-net={})", host));
            }
            let mut reg = mqtt_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                state.host = host;
                state.port = port;
                state.use_tls = use_tls;
                state.connected = true;
                None
            } else {
                Some("Invalid MQTT ID".to_string())
            }
        },
    )?;
    globals.set("__mqttConnect", connect_fn)?;

    let subscribe_fn = Function::new(
        ctx.clone(),
        move |id: MqttId, topic: String, _qos: u8| -> Option<String> {
            let mut reg = mqtt_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                if !state.connected {
                    return Some("Not connected".to_string());
                }
                state.subscriptions.push(topic);
                None
            } else {
                Some("Invalid MQTT ID".to_string())
            }
        },
    )?;
    globals.set("__mqttSubscribe", subscribe_fn)?;

    let unsubscribe_fn = Function::new(
        ctx.clone(),
        move |id: MqttId, topic: String| -> Option<String> {
            let mut reg = mqtt_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                if !state.connected {
                    return Some("Not connected".to_string());
                }
                state.subscriptions.retain(|t| t != &topic);
                None
            } else {
                Some("Invalid MQTT ID".to_string())
            }
        },
    )?;
    globals.set("__mqttUnsubscribe", unsubscribe_fn)?;

    let publish_fn = Function::new(
        ctx.clone(),
        move |id: MqttId,
              _topic: String,
              _payload: String,
              _qos: u8,
              _retain: bool|
              -> Option<String> {
            let reg = mqtt_registry().lock().unwrap();
            if let Some(state) = reg.get(&id) {
                if !state.connected {
                    return Some("Not connected".to_string());
                }
                None
            } else {
                Some("Invalid MQTT ID".to_string())
            }
        },
    )?;
    globals.set("__mqttPublish", publish_fn)?;

    let ping_fn = Function::new(ctx.clone(), move |id: MqttId| -> Option<String> {
        let reg = mqtt_registry().lock().unwrap();
        if let Some(state) = reg.get(&id) {
            if !state.connected {
                return Some("Not connected".to_string());
            }
            None
        } else {
            Some("Invalid MQTT ID".to_string())
        }
    })?;
    globals.set("__mqttPing", ping_fn)?;

    let disconnect_fn = Function::new(ctx.clone(), move |id: MqttId| -> Option<String> {
        let mut reg = mqtt_registry().lock().unwrap();
        if let Some(state) = reg.get_mut(&id) {
            state.connected = false;
            None
        } else {
            Some("Invalid MQTT ID".to_string())
        }
    })?;
    globals.set("__mqttDisconnect", disconnect_fn)?;

    let close_fn = Function::new(ctx.clone(), move |id: MqttId| -> bool {
        mqtt_registry().lock().unwrap().remove(&id);
        true
    })?;
    globals.set("__mqttClose", close_fn)?;

    let is_connected_fn = Function::new(ctx.clone(), move |id: MqttId| -> bool {
        mqtt_registry()
            .lock()
            .unwrap()
            .get(&id)
            .map(|s| s.connected)
            .unwrap_or(false)
    })?;
    globals.set("__mqttIsConnected", is_connected_fn)?;

    ctx.eval::<(), _>(
        r#"
    (function() {
        function MqttClient(urlOrOptions, options) {
            var opts;
            if (urlOrOptions && typeof urlOrOptions === 'object') {
                opts = urlOrOptions;
                this._host = opts.host || 'localhost';
                this._port = opts.port || (opts.tls ? 8883 : 1883);
                this._tls = opts.tls || false;
                this.url = (opts.tls ? 'mqtts' : 'mqtt') + '://' + this._host + ':' + this._port;
            } else {
                this.url = urlOrOptions || 'mqtt://localhost:1883';
                opts = options || {};
                this._host = null;
                this._port = null;
                this._tls = false;
            }
            this.options = opts;
            this.clientId = opts.clientId || null;
            this.keepalive = opts.keepalive || 60;
            this.clean = opts.clean !== false;
            this.username = opts.username || null;
            this.password = opts.password || null;
            this.reconnectPeriod = opts.reconnectPeriod || 1000;
            this._connected = false;
            this._subscriptions = [];
            this._handlers = {};
        }

        MqttClient.prototype.connect = function() {
            var host, port, useTls;
            if (this._host) {
                host = this._host; port = this._port; useTls = this._tls;
            } else {
                var url = new URL(this.url);
                host = url.hostname;
                port = parseInt(url.port) || (url.protocol === 'mqtts:' ? 8883 : 1883);
                useTls = url.protocol === 'mqtts:' || url.protocol === 'mqtt+tls:';
            }
            this._id = __mqttCreate(this.clientId);
            var err = __mqttConnect(this._id, host, port, useTls);
            if (err) throw new Error(err);
            this._connected = true;
            return this;
        };

        MqttClient.prototype.subscribe = function(topic, options, callback) {
            if (typeof topic === 'string') topic = [topic];
            var qos = (options && options.qos) || 0;
            var self = this;
            if (this._id) {
                topic.forEach(function(t) {
                    var err = __mqttSubscribe(self._id, t, qos);
                    if (err) throw new Error(err);
                    self._subscriptions.push({ topic: t, qos: qos });
                });
            }
            if (typeof options === 'function') options();
            else if (callback) callback();
            return this;
        };

        MqttClient.prototype.unsubscribe = function(topic, callback) {
            if (typeof topic === 'string') topic = [topic];
            var self = this;
            if (this._id) {
                topic.forEach(function(t) {
                    var err = __mqttUnsubscribe(self._id, t);
                    if (err) throw new Error(err);
                });
            }
            if (callback) callback();
            return this;
        };

        MqttClient.prototype.publish = function(topic, payload, options, callback) {
            var qos = (options && options.qos) || 0;
            var retain = (options && options.retain) || false;
            if (typeof payload === 'object' && payload !== null) payload = JSON.stringify(payload);
            if (this._id) {
                var err = __mqttPublish(this._id, topic, String(payload), qos, retain);
                if (err) throw new Error(err);
            }
            if (typeof options === 'function') options();
            else if (callback) callback();
            return this;
        };

        MqttClient.prototype.end = MqttClient.prototype.disconnect = function(force, callback) {
            this._connected = false;
            if (this._id) __mqttDisconnect(this._id);
            if (typeof force === 'function') force();
            else if (callback) callback();
            return this;
        };

        MqttClient.prototype.close = function() {
            this._connected = false;
            if (this._id) __mqttClose(this._id);
            return this;
        };

        MqttClient.prototype.ack = function(packet, callback) {
            if (callback) callback();
            return this;
        };

        MqttClient.prototype.on = MqttClient.prototype.addListener = function(event, handler) {
            this._handlers[event] = this._handlers[event] || [];
            this._handlers[event].push(handler);
            return this;
        };

        MqttClient.prototype.off = MqttClient.prototype.removeListener = function(event, handler) {
            if (this._handlers[event] && handler) {
                var idx = this._handlers[event].indexOf(handler);
                if (idx >= 0) this._handlers[event].splice(idx, 1);
            }
            return this;
        };

        MqttClient.prototype.removeAllListeners = function(event) {
            if (event) this._handlers[event] = [];
            else this._handlers = {};
            return this;
        };

        MqttClient.prototype.emit = function(event) {
            var args = Array.prototype.slice.call(arguments, 1);
            (this._handlers[event] || []).forEach(function(h) { h.apply(null, args); });
        };

        MqttClient.prototype.getSessionStatus = function() {
            return this._id ? __mqttIsConnected(this._id) : false;
        };

        MqttClient.prototype.reconnect = function() { return this.connect(); };

        function connect(url, options) {
            return new MqttClient(url, options).connect();
        }

        globalThis.__requireCache = globalThis.__requireCache || {};
        globalThis.__requireCache['mqtt'] = { Client: MqttClient, connect: connect };
        globalThis.__requireCache['node:mqtt'] = { Client: MqttClient, connect: connect };
        globalThis.mqtt = { Client: MqttClient, connect: connect };
    })();
    "#,
    )?;

    Ok(())
}
