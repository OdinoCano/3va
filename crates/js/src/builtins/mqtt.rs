//! MQTT (Message Queuing Telemetry Transport) client built-in module
//!
//! Native functions:
//! - `__mqttCreate(clientId)` -> id
//! - `__mqttConnect(id, host, port, useTls)` -> throws on failure
//! - `__mqttSend(id, packetType, data)` -> throws on failure
//! - `__mqttRead(id, maxBytes)` -> Vec<u8> | throws EAGAIN | throws EOF
//! - `__mqttClose(id)`

use crate::builtins::v8_compat::{uint8array_from_bytes, uint8array_to_vec};
use native_tls::TlsStream;
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, OnceLock};
use vvva_permissions::{Capability, PermissionState};

type MqttId = u32;

#[allow(clippy::large_enum_variant)]
enum MqttConn {
    Plain(TcpStream),
    Tls(TlsStream<TcpStream>),
}

impl MqttConn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            MqttConn::Plain(s) => s.read(buf),
            MqttConn::Tls(s) => s.read(buf),
        }
    }
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        match self {
            MqttConn::Plain(s) => s.write_all(data),
            MqttConn::Tls(s) => s.write_all(data),
        }
    }
    fn shutdown(&mut self) {
        match self {
            MqttConn::Plain(s) => {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
            MqttConn::Tls(s) => {
                let _ = s.shutdown();
            }
        }
    }
}

#[allow(dead_code)]
struct MqttState {
    conn: Option<MqttConn>,
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

const MQTT_DISCONNECT: u8 = 0xe0;

fn encode_remaining_length(len: usize) -> Vec<u8> {
    let mut result = Vec::new();
    let mut x = len;
    loop {
        let mut byte = (x % 128) as u8;
        x /= 128;
        if x > 0 {
            byte |= 0x80;
        }
        result.push(byte);
        if x == 0 {
            break;
        }
    }
    result
}

// Thread-local, not a process-wide static — see the identical fix (and
// rationale) in fs.rs's FS_PERMISSIONS: a `OnceLock` here only keeps the
// *first* engine's permissions ever created in the process, so every later
// `JsEngine` (every other test, or a second engine in a long-lived process)
// silently inherits the first one's grants instead of its own.
thread_local! {
    static MQTT_PERMISSIONS: std::cell::RefCell<Option<Arc<PermissionState>>> =
        const { std::cell::RefCell::new(None) };
}
fn perms() -> Arc<PermissionState> {
    MQTT_PERMISSIONS.with(|p| {
        p.borrow()
            .clone()
            .expect("inject_mqtt not called on this thread")
    })
}

pub fn inject_mqtt(
    scope: &mut v8::ContextScope<v8::HandleScope>,
    permissions: Arc<PermissionState>,
) {
    let context = scope.get_current_context();
    let global = context.global(scope);
    MQTT_PERMISSIONS.with(|p| *p.borrow_mut() = Some(permissions));

    let create_fn = v8::Function::new(
        scope,
        move |scope: &mut v8::PinScope<'_, '_>,
              args: v8::FunctionCallbackArguments<'_>,
              mut rv: v8::ReturnValue<'_>| {
            let client_id_arg = args.get(0);
            let client_id = if client_id_arg.is_undefined() {
                None
            } else {
                Some(client_id_arg.to_rust_string_lossy(scope))
            };
            let id = next_mqtt_id();
            mqtt_registry().lock().unwrap().insert(
                id,
                MqttState {
                    conn: None,
                    client_id: client_id.unwrap_or_else(generate_client_id),
                    use_tls: false,
                    connected: false,
                    subscriptions: vec![],
                },
            );
            rv.set(v8::Number::new(scope, id as f64).into());
        },
    )
    .unwrap();
    let key = v8::String::new(scope, "__mqttCreate").unwrap().into();
    global.set(scope, key, create_fn.into());

    let connect_fn = v8::Function::new(
        scope,
        move |scope: &mut v8::PinScope<'_, '_>,
              args: v8::FunctionCallbackArguments<'_>,
              mut rv: v8::ReturnValue<'_>| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as MqttId;
            let host = args.get(1).to_rust_string_lossy(scope);
            let port = args.get(2).uint32_value(scope).unwrap_or(1883) as u16;
            let use_tls = args.get(3).boolean_value(scope);

            let perms = perms().clone();

            if !perms.check(&Capability::Network(host.clone())) {
                let msg = v8::String::new(
                    scope,
                    &format!("Network access denied. Run with --allow-net={}", host),
                )
                .unwrap();
                let err = v8::Exception::error(scope, msg);
                rv.set(err);
                return;
            }

            match TcpStream::connect(format!("{}:{}", host, port)) {
                Ok(tcp) => {
                    let conn = if use_tls {
                        match native_tls::TlsConnector::new() {
                            Ok(connector) => {
                                let fallback = tcp.try_clone().ok();
                                match connector.connect(&host, tcp) {
                                    Ok(tls) => {
                                        if tls.get_ref().set_nonblocking(true).is_ok() {
                                            MqttConn::Tls(tls)
                                        } else if let Some(tcp) = fallback {
                                            MqttConn::Plain(tcp)
                                        } else {
                                            return;
                                        }
                                    }
                                    Err(_) => {
                                        if let Some(tcp) = fallback {
                                            MqttConn::Plain(tcp)
                                        } else {
                                            return;
                                        }
                                    }
                                }
                            }
                            Err(_) => MqttConn::Plain(tcp),
                        }
                    } else {
                        let _ = tcp.set_nonblocking(true);
                        MqttConn::Plain(tcp)
                    };

                    let mut reg = mqtt_registry().lock().unwrap();
                    if let Some(state) = reg.get_mut(&id) {
                        state.conn = Some(conn);
                        state.use_tls = use_tls;
                        state.connected = true;
                    }
                    rv.set(v8::undefined(scope).into());
                }
                Err(e) => {
                    let msg = v8::String::new(scope, &format!("Connection failed: {}", e)).unwrap();
                    let err = v8::Exception::error(scope, msg);
                    rv.set(err);
                }
            }
        },
    )
    .unwrap();
    let key = v8::String::new(scope, "__mqttConnect").unwrap().into();
    global.set(scope, key, connect_fn.into());

    let send_fn = v8::Function::new(
        scope,
        move |scope: &mut v8::PinScope<'_, '_>,
              args: v8::FunctionCallbackArguments<'_>,
              mut rv: v8::ReturnValue<'_>| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as MqttId;
            let packet_type = args.get(1).uint32_value(scope).unwrap_or(0) as u8;
            let data = v8::Local::<v8::Uint8Array>::try_from(args.get(2))
                .map(|arr| uint8array_to_vec(scope, arr))
                .unwrap_or_default();

            let remaining_len = data.len();
            let len_bytes = encode_remaining_length(remaining_len);

            let mut packet = Vec::with_capacity(1 + len_bytes.len() + data.len());
            packet.push(packet_type);
            packet.extend_from_slice(&len_bytes);
            packet.extend_from_slice(&data);

            let mut reg = mqtt_registry().lock().unwrap();
            match reg.get_mut(&id).and_then(|s| s.conn.as_mut()) {
                Some(conn) => match conn.write_all(&packet) {
                    Ok(_) => rv.set(v8::undefined(scope).into()),
                    Err(e) => {
                        let msg = v8::String::new(scope, &e.to_string()).unwrap();
                        let err = v8::Exception::error(scope, msg);
                        rv.set(err);
                    }
                },
                None => {
                    let msg = v8::String::new(scope, "not connected").unwrap();
                    let err = v8::Exception::error(scope, msg);
                    rv.set(err);
                }
            }
        },
    )
    .unwrap();
    let key = v8::String::new(scope, "__mqttSend").unwrap().into();
    global.set(scope, key, send_fn.into());

    let read_fn = v8::Function::new(
        scope,
        move |scope: &mut v8::PinScope<'_, '_>,
              args: v8::FunctionCallbackArguments<'_>,
              mut rv: v8::ReturnValue<'_>| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as MqttId;
            let max_bytes = args.get(1).uint32_value(scope).unwrap_or(65536) as usize;
            let max = max_bytes.min(65536);

            let mut buf = vec![0u8; max];
            let mut reg = mqtt_registry().lock().unwrap();
            match reg.get_mut(&id).and_then(|s| s.conn.as_mut()) {
                Some(conn) => match conn.read(&mut buf) {
                    Ok(0) => {
                        let msg = v8::String::new(scope, "connection closed").unwrap();
                        let err = v8::Exception::error(scope, msg);
                        rv.set(err);
                    }
                    Ok(n) => {
                        buf.truncate(n);
                        rv.set(uint8array_from_bytes(scope, &buf).into());
                    }
                    Err(ref e)
                        if e.kind() == io::ErrorKind::WouldBlock
                            || e.kind() == io::ErrorKind::TimedOut =>
                    {
                        let msg = v8::String::new(scope, "no data available").unwrap();
                        let err = v8::Exception::error(scope, msg);
                        rv.set(err);
                    }
                    Err(e) => {
                        let msg = v8::String::new(scope, &e.to_string()).unwrap();
                        let err = v8::Exception::error(scope, msg);
                        rv.set(err);
                    }
                },
                None => {
                    let msg = v8::String::new(scope, "not connected").unwrap();
                    let err = v8::Exception::error(scope, msg);
                    rv.set(err);
                }
            }
        },
    )
    .unwrap();
    let key = v8::String::new(scope, "__mqttRead").unwrap().into();
    global.set(scope, key, read_fn.into());

    let is_connected_fn = v8::Function::new(
        scope,
        move |scope: &mut v8::PinScope<'_, '_>,
              args: v8::FunctionCallbackArguments<'_>,
              mut rv: v8::ReturnValue<'_>| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as MqttId;
            let connected = mqtt_registry()
                .lock()
                .unwrap()
                .get(&id)
                .map(|s| s.connected)
                .unwrap_or(false);
            rv.set(v8::Boolean::new(scope, connected).into());
        },
    )
    .unwrap();
    let key = v8::String::new(scope, "__mqttIsConnected").unwrap().into();
    global.set(scope, key, is_connected_fn.into());

    let disconnect_fn = v8::Function::new(
        scope,
        move |scope: &mut v8::PinScope<'_, '_>,
              args: v8::FunctionCallbackArguments<'_>,
              mut rv: v8::ReturnValue<'_>| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as MqttId;
            let mut reg = mqtt_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                if let Some(conn) = state.conn.as_mut() {
                    let packet = vec![MQTT_DISCONNECT, 0x00];
                    let _ = conn.write_all(&packet);
                }
                state.connected = false;
            }
            rv.set(v8::undefined(scope).into());
        },
    )
    .unwrap();
    let key = v8::String::new(scope, "__mqttDisconnect").unwrap().into();
    global.set(scope, key, disconnect_fn.into());

    let close_fn = v8::Function::new(
        scope,
        move |scope: &mut v8::PinScope<'_, '_>,
              args: v8::FunctionCallbackArguments<'_>,
              mut rv: v8::ReturnValue<'_>| {
            let id = args.get(0).uint32_value(scope).unwrap_or(0) as MqttId;
            if let Some(mut state) = mqtt_registry().lock().unwrap().remove(&id)
                && let Some(mut conn) = state.conn.take()
            {
                conn.shutdown();
            }
            rv.set(v8::Boolean::new(scope, true).into());
        },
    )
    .unwrap();
    let key = v8::String::new(scope, "__mqttClose").unwrap().into();
    global.set(scope, key, close_fn.into());

    let js_code = r#"
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
            this._id = null;
            this._lineBuffer = '';
            this._pollTimer = null;
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
            __mqttConnect(this._id, host, port, useTls);
            this._connected = true;
            this._startPoll();
            this._sendConnect();
            return this;
        };

        MqttClient.prototype._sendConnect = function() {
            if (!this._id) return;
            var clientId = this.clientId || ('3va_' + Math.floor(Math.random() * 1000000));
            var protocol = 'MQTT';
            var protocolLevel = 0x04;
            var connectFlags = 0x02;
            if (this.username) connectFlags |= 0x80;
            if (this.password) connectFlags |= 0x40;

            var variableHeader = [];
            variableHeader.push(0x00);
            variableHeader.push(protocol.length);
            for (var i = 0; i < protocol.length; i++) {
                variableHeader.push(protocol.charCodeAt(i));
            }
            variableHeader.push(protocolLevel);
            variableHeader.push(connectFlags);
            variableHeader.push((this.keepalive >> 8) & 0xFF);
            variableHeader.push(this.keepalive & 0xFF);

            var payload = [];
            var clientIdBytes = [];
            clientIdBytes.push((clientId.length >> 8) & 0xFF);
            clientIdBytes.push(clientId.length & 0xFF);
            for (var i = 0; i < clientId.length; i++) {
                clientIdBytes.push(clientId.charCodeAt(i));
            }
            payload = payload.concat(clientIdBytes);

            if (this.username) {
                var usernameBytes = [0x00, 0x00];
                var usernameLen = this.username.length;
                usernameBytes[0] = (usernameLen >> 8) & 0xFF;
                usernameBytes[1] = usernameLen & 0xFF;
                for (var i = 0; i < this.username.length; i++) {
                    usernameBytes.push(this.username.charCodeAt(i));
                }
                payload = payload.concat(usernameBytes);
            }

            if (this.password) {
                var passwordBytes = [0x00, 0x00];
                var passwordLen = this.password.length;
                passwordBytes[0] = (passwordLen >> 8) & 0xFF;
                passwordBytes[1] = passwordLen & 0xFF;
                for (var i = 0; i < this.password.length; i++) {
                    passwordBytes.push(this.password.charCodeAt(i));
                }
                payload = payload.concat(passwordBytes);
            }

            var data = variableHeader.concat(payload);
            __mqttSend(this._id, 0x10, data);
        };

        MqttClient.prototype._startPoll = function() {
            var self = this;
            var delay = 1;
            function poll() {
                if (!self._connected) { self._pollTimer = null; return; }
                try {
                    var chunk = __mqttRead(self._id, 65536);
                    delay = 1;
                    if (chunk && chunk.length > 0) {
                        self._handlePacket(new Uint8Array(chunk));
                    }
                    self._pollTimer = setTimeout(poll, 0);
                } catch (e) {
                    if (e && e.code === 'EAGAIN') {
                        delay = Math.min(delay * 2, 100);
                        self._pollTimer = setTimeout(poll, delay);
                        return;
                    }
                    self._pollTimer = null;
                    self._connected = false;
                    if (e && e.code === 'EOF') self.emit('close');
                    else self.emit('error', e);
                }
            }
            self._pollTimer = setTimeout(poll, 0);
        };

        MqttClient.prototype._handlePacket = function(data) {
            var offset = 0;
            while (offset < data.length) {
                var packetType = data[offset];
                offset++;
                var multiplier = 1;
                var remainingLength = 0;
                var encodedByte;
                do {
                    if (offset >= data.length) return;
                    encodedByte = data[offset++];
                    remainingLength += (encodedByte & 0x7F) * multiplier;
                    multiplier *= 128;
                } while ((encodedByte & 0x80) !== 0);

                var packetData = data.slice(offset, offset + remainingLength);
                offset += remainingLength;

                this._processPacket(packetType, packetData);
            }
        };

        MqttClient.prototype._processPacket = function(packetType, data) {
            var packetTypeName = packetType & 0xF0;
            switch (packetTypeName) {
                case 0x20:
                    this.emit('connect');
                    if (data.length >= 2) {
                        var rc = data[1];
                        if (rc === 0) {
                            this.emit('connected');
                        } else {
                            this.emit('error', new Error('Connection refused: ' + rc));
                        }
                    }
                    break;
                case 0x30:
                    this._handlePublish(data);
                    break;
                case 0x40:
                    this.emit('puback', data);
                    break;
                case 0x90:
                    this.emit('suback', data);
                    break;
                case 0xb0:
                    this.emit('unsuback', data);
                    break;
                case 0xd0:
                    break;
            }
        };

        MqttClient.prototype._handlePublish = function(data) {
            var offset = 0;
            var topicLen = (data[offset] << 8) | data[offset + 1];
            offset += 2;
            var topic = '';
            for (var i = 0; i < topicLen; i++) {
                topic += String.fromCharCode(data[offset++]);
            }
            if (data.length > offset) {
                var payloadLen = data.length - offset;
                var payload = '';
                for (var i = 0; i < payloadLen; i++) {
                    payload += String.fromCharCode(data[offset++]);
                }
                this.emit('message', topic, payload);
            }
        };

        MqttClient.prototype.subscribe = function(topic, options, callback) {
            if (typeof topic === 'string') topic = [topic];
            var qos = (options && options.qos) || 0;
            var self = this;
            if (this._id && this._connected) {
                var variableHeader = [0x00, 0x01];
                var payload = [];
                for (var i = 0; i < topic.length; i++) {
                    payload.push((topic[i].length >> 8) & 0xFF);
                    payload.push(topic[i].length & 0xFF);
                    for (var j = 0; j < topic[i].length; j++) {
                        payload.push(topic[i].charCodeAt(j));
                    }
                    payload.push(qos);
                    self._subscriptions.push({ topic: topic[i], qos: qos });
                }
                var data = variableHeader.concat(payload);
                __mqttSend(this._id, 0x82, data);
            }
            if (typeof options === 'function') options();
            else if (callback) callback();
            return this;
        };

        MqttClient.prototype.unsubscribe = function(topic, callback) {
            if (typeof topic === 'string') topic = [topic];
            if (this._id && this._connected) {
                var variableHeader = [0x00, 0x01];
                var payload = [];
                for (var i = 0; i < topic.length; i++) {
                    payload.push((topic[i].length >> 8) & 0xFF);
                    payload.push(topic[i].length & 0xFF);
                    for (var j = 0; j < topic[i].length; j++) {
                        payload.push(topic[i].charCodeAt(j));
                    }
                }
                var data = variableHeader.concat(payload);
                __mqttSend(this._id, 0xa2, data);
            }
            if (callback) callback();
            return this;
        };

        MqttClient.prototype.publish = function(topic, payload, options, callback) {
            var qos = (options && options.qos) || 0;
            var retain = (options && options.retain) || false;
            if (typeof payload === 'object' && payload !== null) payload = JSON.stringify(payload);
            if (this._id && this._connected) {
                var header = [0x00];
                if (retain) header[0] |= 0x01;
                header[0] |= (qos << 1);

                var topicBytes = [(topic.length >> 8) & 0xFF, topic.length & 0xFF];
                var topicData = [];
                for (var i = 0; i < topic.length; i++) {
                    topicData.push(topic.charCodeAt(i));
                }
                var payloadData = [];
                if (typeof payload === 'string') {
                    for (var i = 0; i < payload.length; i++) {
                        payloadData.push(payload.charCodeAt(i));
                    }
                } else if (payload instanceof Uint8Array || payload instanceof ArrayBuffer) {
                    payloadData = Array.from(payload);
                }

                var data = header.concat(topicBytes).concat(topicData).concat(payloadData);
                __mqttSend(this._id, 0x30, data);
            }
            if (typeof options === 'function') options();
            else if (callback) callback();
            return this;
        };

        MqttClient.prototype.end = function(force, callback) {
            if (this._id) {
                __mqttDisconnect(this._id);
            }
            this._connected = false;
            if (this._pollTimer) {
                clearTimeout(this._pollTimer);
                this._pollTimer = null;
            }
            if (typeof force === 'function') force();
            else if (callback) callback();
            return this;
        };

        MqttClient.prototype.disconnect = function() {
            if (this._id) {
                __mqttDisconnect(this._id);
            }
            this._connected = false;
        };

        MqttClient.prototype.close = function() {
            if (this._id) {
                __mqttClose(this._id);
            }
            this._connected = false;
            if (this._pollTimer) {
                clearTimeout(this._pollTimer);
                this._pollTimer = null;
            }
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
    "#;

    let source = v8::String::new(scope, js_code).unwrap();
    if let Some(script) = v8::Script::compile(scope, source, None) {
        let _ = script.run(scope);
    }
}
