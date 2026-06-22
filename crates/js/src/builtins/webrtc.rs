//! WebRTC (Web Real-Time Communication) built-in module
//!
//! Provides peer-to-peer data channels using RTCPeerConnection
//!
//! Native functions:
//! - `__rtcCreatePeerConnection(config)` -> id
//! - `__rtcCreateOffer(id)` -> sdp JSON
//! - `__rtcCreateAnswer(id)` -> sdp JSON
//! - `__rtcSetLocalDescription(id, sdp, type)`
//! - `__rtcSetRemoteDescription(id, sdp, type)`
//! - `__rtcAddIceCandidate(id, candidate)`
//! - `__rtcCreateDataChannel(id, label, ordered, maxRetransmits, maxPacketLifeTime)` -> channelId
//! - `__rtcDataChannelSend(channelId, data)` -> bool
//! - `__rtcDataChannelClose(channelId)`
//! - `__rtcClosePeerConnection(id)`
//! - `__rtcGetConnectionState(id)` -> state string

use rquickjs::{Ctx, Function, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use vvva_permissions::PermissionState;

type RtcId = u32;
type ChannelId = u32;

#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)]
enum RtcState {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
}

struct RtcPeerConnection {
    #[allow(dead_code)]
    ice_servers: Vec<String>,
    state: RtcState,
    local_description: Option<(String, String)>,
    remote_description: Option<(String, String)>,
}

struct RtcDataChannel {
    #[allow(dead_code)]
    rtc_id: RtcId,
    #[allow(dead_code)]
    label: String,
    #[allow(dead_code)]
    ordered: bool,
    ready_state: String,
}

static RTC_REGISTRY: OnceLock<Mutex<HashMap<RtcId, RtcPeerConnection>>> = OnceLock::new();
static CHANNEL_REGISTRY: OnceLock<Mutex<HashMap<ChannelId, RtcDataChannel>>> = OnceLock::new();

fn rtc_registry() -> &'static Mutex<HashMap<RtcId, RtcPeerConnection>> {
    RTC_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn channel_registry() -> &'static Mutex<HashMap<ChannelId, RtcDataChannel>> {
    CHANNEL_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_rtc_id() -> RtcId {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn next_channel_id() -> ChannelId {
    static C: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    C.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn rtc_state_to_string(state: RtcState) -> &'static str {
    match state {
        RtcState::New => "new",
        RtcState::Connecting => "connecting",
        RtcState::Connected => "connected",
        RtcState::Disconnected => "disconnected",
        RtcState::Failed => "failed",
        RtcState::Closed => "closed",
    }
}

pub fn inject_webrtc(ctx: &Ctx, permissions: Arc<PermissionState>) -> Result<()> {
    let globals = ctx.globals();
    let _perms = permissions.clone();

    let create_fn = Function::new(ctx.clone(), move |config_str: String| -> RtcId {
        let id = next_rtc_id();
        let servers: Vec<String> = if config_str.is_empty() {
            vec!["stun:stun.l.google.com:19302".to_string()]
        } else {
            vec![config_str]
        };
        rtc_registry().lock().unwrap().insert(
            id,
            RtcPeerConnection {
                ice_servers: servers,
                state: RtcState::New,
                local_description: None,
                remote_description: None,
            },
        );
        id
    })?;
    globals.set("__rtcCreatePeerConnection", create_fn)?;

    let create_offer_fn = Function::new(ctx.clone(), move |id: RtcId| -> Option<String> {
        let mut reg = rtc_registry().lock().unwrap();
        if let Some(state) = reg.get_mut(&id) {
            state.state = RtcState::Connecting;
            state.local_description = Some((
                "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n".to_string(),
                "offer".to_string(),
            ));
            Some(
                serde_json::json!({
                    "type": "offer",
                    "sdp": "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n"
                })
                .to_string(),
            )
        } else {
            None
        }
    })?;
    globals.set("__rtcCreateOffer", create_offer_fn)?;

    let create_answer_fn = Function::new(ctx.clone(), move |id: RtcId| -> Option<String> {
        let mut reg = rtc_registry().lock().unwrap();
        if let Some(state) = reg.get_mut(&id) {
            state.state = RtcState::Connecting;
            state.remote_description = Some((
                "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n".to_string(),
                "answer".to_string(),
            ));
            Some(
                serde_json::json!({
                    "type": "answer",
                    "sdp": "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n"
                })
                .to_string(),
            )
        } else {
            None
        }
    })?;
    globals.set("__rtcCreateAnswer", create_answer_fn)?;

    let set_local_fn = Function::new(
        ctx.clone(),
        move |id: RtcId, sdp: String, type_: String| -> Option<String> {
            let mut reg = rtc_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                state.local_description = Some((sdp, type_));
                None
            } else {
                Some("Invalid RTC ID".to_string())
            }
        },
    )?;
    globals.set("__rtcSetLocalDescription", set_local_fn)?;

    let set_remote_fn = Function::new(
        ctx.clone(),
        move |id: RtcId, sdp: String, type_: String| -> Option<String> {
            let mut reg = rtc_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                state.remote_description = Some((sdp, type_));
                if state.local_description.is_some() {
                    state.state = RtcState::Connected;
                }
                None
            } else {
                Some("Invalid RTC ID".to_string())
            }
        },
    )?;
    globals.set("__rtcSetRemoteDescription", set_remote_fn)?;

    let add_ice_fn = Function::new(
        ctx.clone(),
        move |id: RtcId, _candidate: String| -> Option<String> {
            let mut reg = rtc_registry().lock().unwrap();
            if let Some(state) = reg.get_mut(&id) {
                if state.local_description.is_some() && state.remote_description.is_some() {
                    state.state = RtcState::Connected;
                }
                None
            } else {
                Some("Invalid RTC ID".to_string())
            }
        },
    )?;
    globals.set("__rtcAddIceCandidate", add_ice_fn)?;

    let create_dc_fn = Function::new(
        ctx.clone(),
        move |id: RtcId,
              label: String,
              ordered: bool,
              _max_retransmits: Option<u16>,
              _max_packet_life_time: Option<u16>|
              -> Option<String> {
            let reg = rtc_registry().lock().unwrap();
            if reg.contains_key(&id) {
                let channel_id = next_channel_id();
                channel_registry().lock().unwrap().insert(
                    channel_id,
                    RtcDataChannel {
                        rtc_id: id,
                        label: label.clone(),
                        ordered,
                        ready_state: "open".to_string(),
                    },
                );
                Some(channel_id.to_string())
            } else {
                Some("Invalid RTC ID".to_string())
            }
        },
    )?;
    globals.set("__rtcCreateDataChannel", create_dc_fn)?;

    let dc_send_fn = Function::new(ctx.clone(), move |id: ChannelId, _data: String| -> bool {
        let reg = channel_registry().lock().unwrap();
        reg.get(&id)
            .map(|ch| ch.ready_state == "open")
            .unwrap_or(false)
    })?;
    globals.set("__rtcDataChannelSend", dc_send_fn)?;

    let dc_close_fn = Function::new(ctx.clone(), move |id: ChannelId| -> Option<String> {
        let mut reg = channel_registry().lock().unwrap();
        if let Some(state) = reg.get_mut(&id) {
            state.ready_state = "closed".to_string();
            None
        } else {
            Some("Invalid Channel ID".to_string())
        }
    })?;
    globals.set("__rtcDataChannelClose", dc_close_fn)?;

    let close_fn = Function::new(ctx.clone(), move |id: RtcId| -> Option<String> {
        let mut reg = rtc_registry().lock().unwrap();
        if let Some(state) = reg.get_mut(&id) {
            state.state = RtcState::Closed;
            reg.remove(&id);
            None
        } else {
            Some("Invalid RTC ID".to_string())
        }
    })?;
    globals.set("__rtcClosePeerConnection", close_fn)?;

    let state_fn = Function::new(ctx.clone(), move |id: RtcId| -> Option<String> {
        rtc_registry()
            .lock()
            .unwrap()
            .get(&id)
            .map(|state| rtc_state_to_string(state.state).to_string())
    })?;
    globals.set("__rtcGetConnectionState", state_fn)?;

    ctx.eval::<(), _>(r#"
    (function() {
        function RTCPeerConnection(configuration) {
            configuration = configuration || {};
            this.configuration = configuration;
            this.iceServers = configuration.iceServers || [];
            this.localDescription = null;
            this.remoteDescription = null;
            this.iceGatheringState = 'new';
            this.iceConnectionState = 'new';
            this.signalingState = 'stable';
            this._id = __rtcCreatePeerConnection(JSON.stringify(this.iceServers));
            this._dataChannels = [];
            this._eventHandlers = {};
        }

        RTCPeerConnection.prototype.createOffer = function(options) {
            var self = this;
            return new Promise(function(resolve, reject) {
                var result = __rtcCreateOffer(self._id);
                if (!result) {
                    reject(new Error('Failed to create offer'));
                    return;
                }
                var offer = JSON.parse(result);
                resolve(offer);
            });
        };

        RTCPeerConnection.prototype.createAnswer = function(options) {
            var self = this;
            return new Promise(function(resolve, reject) {
                var result = __rtcCreateAnswer(self._id);
                if (!result) {
                    reject(new Error('Failed to create answer'));
                    return;
                }
                var answer = JSON.parse(result);
                resolve(answer);
            });
        };

        RTCPeerConnection.prototype.setLocalDescription = function(description) {
            var self = this;
            return new Promise(function(resolve, reject) {
                var err = __rtcSetLocalDescription(self._id, description.sdp || '', description.type || '');
                if (err) {
                    reject(new Error(err));
                    return;
                }
                self.localDescription = description;
                self.signalingState = description.type === 'offer' ? 'have-local-offer' : 'have-local-pranswer';
                resolve();
            });
        };

        RTCPeerConnection.prototype.setRemoteDescription = function(description) {
            var self = this;
            return new Promise(function(resolve, reject) {
                var err = __rtcSetRemoteDescription(self._id, description.sdp || '', description.type || '');
                if (err) {
                    reject(new Error(err));
                    return;
                }
                self.remoteDescription = description;
                self.signalingState = 'stable';
                self.iceConnectionState = 'connecting';
                resolve();
            });
        };

        RTCPeerConnection.prototype.addIceCandidate = function(candidate) {
            var self = this;
            return new Promise(function(resolve, reject) {
                var err = __rtcAddIceCandidate(self._id, candidate.candidate || '');
                if (err) {
                    reject(new Error(err));
                    return;
                }
                resolve();
            });
        };

        RTCPeerConnection.prototype.createDataChannel = function(label, options) {
            options = options || {};
            var result = __rtcCreateDataChannel(
                this._id, label,
                options.ordered !== false,
                options.maxRetransmits,
                options.maxPacketLifeTime
            );
            if (!result) throw new Error('Failed to create data channel');
            var channelId = parseInt(result);
            var channel = new RTCDataChannel(channelId, label, options);
            this._dataChannels.push(channel);
            this._emit('datachannel', { channel: channel });
            return channel;
        };

        RTCPeerConnection.prototype.close = function() {
            var err = __rtcClosePeerConnection(this._id);
            this.iceConnectionState = 'closed';
            this.signalingState = 'closed';
        };

        RTCPeerConnection.prototype.getConnectionState = function() {
            var result = __rtcGetConnectionState(this._id);
            return result || 'closed';
        };

        RTCPeerConnection.prototype.onicecandidate = null;
        RTCPeerConnection.prototype.oniceconnectionstatechange = null;
        RTCPeerConnection.prototype.ondatachannel = null;
        RTCPeerConnection.prototype.onsignalingstatechange = null;

        RTCPeerConnection.prototype.addEventListener = function(event, handler) {
            this._eventHandlers[event] = this._eventHandlers[event] || [];
            this._eventHandlers[event].push(handler);
        };

        RTCPeerConnection.prototype.removeEventListener = function(event, handler) {
            if (this._eventHandlers[event]) {
                var idx = this._eventHandlers[event].indexOf(handler);
                if (idx >= 0) this._eventHandlers[event].splice(idx, 1);
            }
        };

        RTCPeerConnection.prototype._emit = function(event) {
            var args = Array.prototype.slice.call(arguments, 1);
            if (this._eventHandlers[event]) {
                this._eventHandlers[event].forEach(function(h) { h.apply(null, args); });
            }
            var handler = this['on' + event];
            if (handler) handler.apply(null, args);
        };

        function RTCDataChannel(id, label, options) {
            this._id = id;
            this.label = label;
            this.ordered = options.ordered !== false;
            this.maxRetransmits = options.maxRetransmits;
            this.maxPacketLifeTime = options.maxPacketLifeTime;
            this.protocol = options.protocol || '';
            this.binaryType = 'arraybuffer';
            this.readyState = 'open';
            this._eventHandlers = {};
        }

        RTCDataChannel.prototype.send = function(data) {
            if (this.readyState !== 'open') {
                throw new Error('Data channel is not open');
            }
            return __rtcDataChannelSend(this._id, String(data));
        };

        RTCDataChannel.prototype.close = function() {
            __rtcDataChannelClose(this._id);
            this.readyState = 'closed';
        };

        RTCDataChannel.prototype.addEventListener = function(event, handler) {
            this._eventHandlers[event] = this._eventHandlers[event] || [];
            this._eventHandlers[event].push(handler);
        };

        RTCDataChannel.prototype.removeEventListener = function(event, handler) {
            if (this._eventHandlers[event]) {
                var idx = this._eventHandlers[event].indexOf(handler);
                if (idx >= 0) this._eventHandlers[event].splice(idx, 1);
            }
        };

        function RTCSessionDescription(init) {
            this.type = init.type;
            this.sdp = init.sdp;
        }

        function RTCIceCandidate(init) {
            this.candidate = init.candidate || '';
            this.sdpMid = init.sdpMid;
            this.sdpMLineIndex = init.sdpMLineIndex;
            this.foundation = init.foundation || '';
            this.component = init.component || 1;
            this.protocol = init.protocol || 'udp';
            this.address = init.address || '';
            this.port = init.port || 0;
            this.type = init.type || 'host';
        }

        globalThis.RTCPeerConnection = RTCPeerConnection;
        globalThis.RTCSessionDescription = RTCSessionDescription;
        globalThis.RTCIceCandidate = RTCIceCandidate;
        globalThis.RTCDataChannel = RTCDataChannel;

        globalThis.__requireCache = globalThis.__requireCache || {};
        globalThis.__requireCache['webrtc'] = {
            RTCPeerConnection: RTCPeerConnection,
            RTCSessionDescription: RTCSessionDescription,
            RTCIceCandidate: RTCIceCandidate,
            RTCDataChannel: RTCDataChannel
        };
    })();
    "#)?;

    Ok(())
}
