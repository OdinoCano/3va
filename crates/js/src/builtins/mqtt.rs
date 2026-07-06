//! MQTT (Message Queuing Telemetry Transport) client built-in module
//!
//! Provides: `require('mqtt')` with `connect()` function, backed by real TCP (or TLS)
//! sockets implementing MQTT 3.1.1 protocol (QoS 0 only).
//!
//! Native functions:
//! - `__mqttCreate(clientId)` -> id
//! - `__mqttConnect(id, host, port, useTls)` -> throws on failure
//! - `__mqttSend(id, packetType, data)` -> throws on failure
//! - `__mqttRead(id, maxBytes)` -> Vec<u8> | throws EAGAIN | throws EOF
//! - `__mqttClose(id)`

use native_tls::TlsStream;
use rquickjs::{Ctx, Function, Result};
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

fn js_code_err(ctx: &Ctx<'_>, code: &str, msg: String) -> rquickjs::Error {
    let escaped_msg = msg.replace('\\', "\\\\").replace('"', "\\\"");
    let src = format!(
        "(function(){{var e=new Error(\"{msg}\");e.code=\"{code}\";return e;}})()",
        msg = escaped_msg,
        code = code
    );
    match ctx.eval::<rquickjs::Value<'_>, _>(src) {
        Ok(v) => ctx.throw(v),
        Err(e) => e,
    }
}

// MQTT packet types
const MQTT_DISCONNECT: u8 = 0xe0;

// Encode remaining length in MQTT variable length format
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

pub fn inject_mqtt(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();

    // __mqttCreate(clientId) -> id
    let create_fn = Function::new(ctx.clone(), move |client_id: Option<String>| -> MqttId {
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
        id
    })?;
    globals.set("__mqttCreate", create_fn)?;

    // __mqttConnect(id, host, port, useTls) -> undefined | throws
    let perms = permissions.clone();
    let connect_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, id: MqttId, host: String, port: u16, use_tls: bool| -> Result<()> {
            if !perms.check(&Capability::Network(host.clone())) {
                return Err(js_code_err(
                    &ctx,
                    "EACCES",
                    format!("Network access denied. Run with --allow-net={}", host),
                ));
            }

            let tcp = TcpStream::connect(format!("{host}:{port}"))
                .map_err(|e| js_code_err(&ctx, "ECONNREFUSED", e.to_string()))?;

            let conn = if use_tls {
                let connector = native_tls::TlsConnector::new()
                    .map_err(|e| js_code_err(&ctx, "EIO", format!("TLS init failed: {e}")))?;
                let tls = connector.connect(&host, tcp).map_err(|e| {
                    js_code_err(&ctx, "ECONNRESET", format!("TLS handshake failed: {e}"))
                })?;
                tls.get_ref()
                    .set_nonblocking(true)
                    .map_err(|e| js_code_err(&ctx, "EIO", e.to_string()))?;
                MqttConn::Tls(tls)
            } else {
                tcp.set_nonblocking(true)
                    .map_err(|e| js_code_err(&ctx, "EIO", e.to_string()))?;
                MqttConn::Plain(tcp)
            };

            let mut reg = mqtt_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                state.conn = Some(conn);
                state.use_tls = use_tls;
                state.connected = true;
                Ok(())
            } else {
                Err(js_code_err(&ctx, "ENOTCONN", "Invalid MQTT ID".to_string()))
            }
        },
    )?;
    globals.set("__mqttConnect", connect_fn)?;

    // __mqttSend(id, packetType, data) -> throws on failure
    let send_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, id: MqttId, packet_type: u8, data: Vec<u8>| -> Result<()> {
            let mut reg = mqtt_registry().lock().unwrap();
            let conn = reg
                .get_mut(&id)
                .and_then(|s| s.conn.as_mut())
                .ok_or_else(|| js_code_err(&ctx, "ENOTCONN", "not connected".to_string()))?;

            // Build packet: packet_type + remaining_length + payload
            let remaining_len = data.len();
            let len_bytes = encode_remaining_length(remaining_len);

            let mut packet = Vec::with_capacity(1 + len_bytes.len() + data.len());
            packet.push(packet_type);
            packet.extend_from_slice(&len_bytes);
            packet.extend_from_slice(&data);

            conn.write_all(&packet)
                .map_err(|e| js_code_err(&ctx, "EPIPE", e.to_string()))
        },
    )?;
    globals.set("__mqttSend", send_fn)?;

    // __mqttRead(id, maxBytes) -> Vec<u8> | throws EAGAIN | throws EOF
    let read_fn = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'_>, id: MqttId, max_bytes: u32| -> Result<Vec<u8>> {
            let max = (max_bytes as usize).min(65536);
            let mut buf = vec![0u8; max];
            let mut reg = mqtt_registry().lock().unwrap();
            let conn = reg
                .get_mut(&id)
                .and_then(|s| s.conn.as_mut())
                .ok_or_else(|| js_code_err(&ctx, "ENOTCONN", "not connected".to_string()))?;

            match conn.read(&mut buf) {
                Ok(0) => Err(js_code_err(&ctx, "EOF", "connection closed".into())),
                Ok(n) => {
                    buf.truncate(n);
                    Ok(buf)
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    Err(js_code_err(&ctx, "EAGAIN", "no data available".into()))
                }
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {
                    Err(js_code_err(&ctx, "EAGAIN", "no data available".into()))
                }
                Err(e) => Err(js_code_err(&ctx, "EIO", e.to_string())),
            }
        },
    )?;
    globals.set("__mqttRead", read_fn)?;

    // __mqttIsConnected(id) -> bool
    let is_connected_fn = Function::new(ctx.clone(), move |id: MqttId| -> bool {
        mqtt_registry()
            .lock()
            .unwrap()
            .get(&id)
            .map(|s| s.connected)
            .unwrap_or(false)
    })?;
    globals.set("__mqttIsConnected", is_connected_fn)?;

    // __mqttDisconnect(id)
    let disconnect_fn = Function::new(
        ctx.clone(),
        move |_ctx: Ctx<'_>, id: MqttId| -> Result<()> {
            let mut reg = mqtt_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                if let Some(conn) = state.conn.as_mut() {
                    // Send DISCONNECT packet
                    let packet = vec![MQTT_DISCONNECT, 0x00];
                    let _ = conn.write_all(&packet);
                }
                state.connected = false;
            }
            Ok(())
        },
    )?;
    globals.set("__mqttDisconnect", disconnect_fn)?;

    // __mqttClose(id)
    let close_fn = Function::new(ctx.clone(), move |id: MqttId| -> bool {
        if let Some(mut state) = mqtt_registry().lock().unwrap().remove(&id)
            && let Some(mut conn) = state.conn.take()
        {
            conn.shutdown();
        }
        true
    })?;
    globals.set("__mqttClose", close_fn)?;

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
            // Build CONNECT packet
            var protocol = 'MQTT';
            var protocolLevel = 0x04; // MQTT 3.1.1
            var connectFlags = 0x02; // Clean session
            if (this.username) connectFlags |= 0x80;
            if (this.password) connectFlags |= 0x40;

            // Variable header
            var variableHeader = [];
            // Protocol name
            variableHeader.push(0x00); // Length MSB
            variableHeader.push(protocol.length); // Length LSB
            for (var i = 0; i < protocol.length; i++) {
                variableHeader.push(protocol.charCodeAt(i));
            }
            variableHeader.push(protocolLevel); // Protocol level
            variableHeader.push(connectFlags); // Connect flags
            variableHeader.push((this.keepalive >> 8) & 0xFF); // Keepalive MSB
            variableHeader.push(this.keepalive & 0xFF); // Keepalive LSB

            // Payload
            var payload = [];
            // Client ID
            var clientIdBytes = [];
            clientIdBytes.push((clientId.length >> 8) & 0xFF);
            clientIdBytes.push(clientId.length & 0xFF);
            for (var i = 0; i < clientId.length; i++) {
                clientIdBytes.push(clientId.charCodeAt(i));
            }
            payload = payload.concat(clientIdBytes);

            // Username
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

            // Password
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
                // Read remaining length
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
                case 0x20: // CONNACK
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
                case 0x30: // PUBLISH
                    this._handlePublish(data);
                    break;
                case 0x40: // PUBACK
                    this.emit('puback', data);
                    break;
                case 0x90: // SUBACK
                    this.emit('suback', data);
                    break;
                case 0xb0: // UNSUBACK
                    this.emit('unsuback', data);
                    break;
                case 0xd0: // PINGRESP
                    // Heartbeat response, ignore
                    break;
                default:
                    break;
            }
        };

        MqttClient.prototype._handlePublish = function(data) {
            var offset = 0;
            // Read topic length
            var topicLen = (data[offset] << 8) | data[offset + 1];
            offset += 2;
            var topic = '';
            for (var i = 0; i < topicLen; i++) {
                topic += String.fromCharCode(data[offset++]);
            }
            // Skip packet identifier if QoS > 0 (not implemented here)
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
                // Build SUBSCRIBE packet
                var variableHeader = [0x00, 0x01]; // Packet identifier
                var payload = [];
                for (var i = 0; i < topic.length; i++) {
                    // Topic name length
                    payload.push((topic[i].length >> 8) & 0xFF);
                    payload.push(topic[i].length & 0xFF);
                    // Topic name
                    for (var j = 0; j < topic[i].length; j++) {
                        payload.push(topic[i].charCodeAt(j));
                    }
                    // QoS
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
                // For simplicity, QoS 0 only in this implementation
                header[0] |= (qos << 1);

                // Topic length
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
    "#,
    )?;

    Ok(())
}
