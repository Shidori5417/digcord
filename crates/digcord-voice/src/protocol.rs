//! Ses gateway'i (v8) JSON mesajları. DAVE'in ikili mesajları `dave` modülünde.
//!
//! Sunucudan gelen JSON mesajlarında `seq` alanı vardır; heartbeat ve resume'da `seq_ack`
//! olarak geri gönderilir.

use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

pub mod op {
    pub const IDENTIFY: u8 = 0;
    pub const SELECT_PROTOCOL: u8 = 1;
    pub const READY: u8 = 2;
    pub const HEARTBEAT: u8 = 3;
    pub const SESSION_DESCRIPTION: u8 = 4;
    pub const SPEAKING: u8 = 5;
    pub const HEARTBEAT_ACK: u8 = 6;
    pub const RESUME: u8 = 7;
    pub const HELLO: u8 = 8;
    pub const RESUMED: u8 = 9;
    pub const CLIENTS_CONNECT: u8 = 11;
    pub const CLIENT_DISCONNECT: u8 = 13;
    pub const DAVE_PREPARE_TRANSITION: u8 = 21;
    pub const DAVE_EXECUTE_TRANSITION: u8 = 22;
    pub const DAVE_TRANSITION_READY: u8 = 23;
    pub const DAVE_PREPARE_EPOCH: u8 = 24;
    pub const DAVE_MLS_INVALID_COMMIT_WELCOME: u8 = 31;
}

/// Desteklenen en yüksek DAVE protokol sürümü.
pub const DAVE_PROTOCOL_VERSION: u16 = 1;
/// Konuşma bayrağı: mikrofon.
pub const SPEAKING_MICROPHONE: u8 = 1;

/// Ses oturumunu açmak için gereken bilgiler (ana gateway'den).
#[derive(Clone)]
pub struct SessionInfo {
    /// Guild id'si, DM aramasında kanal id'si.
    pub server_id: u64,
    pub user_id: u64,
    pub session_id: String,
    /// Gizli: log'a yazılmaz.
    pub token: String,
    /// `host:port` (şemasız).
    pub endpoint: String,
}

impl std::fmt::Debug for SessionInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionInfo")
            .field("server_id", &self.server_id)
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl SessionInfo {
    pub fn url(&self) -> String {
        format!("wss://{}/?v=8", self.endpoint.trim_end_matches(":80"))
    }
}

#[derive(Serialize)]
struct Out<T> {
    op: u8,
    d: T,
}

fn to_json<T: Serialize>(op: u8, d: T) -> String {
    // Sadece sabit yapılı tipler serileştirilir; hata imkânsız.
    serde_json::to_string(&Out { op, d }).unwrap_or_default()
}

pub fn identify(s: &SessionInfo) -> String {
    #[derive(Serialize)]
    struct Identify<'a> {
        server_id: String,
        user_id: String,
        session_id: &'a str,
        token: &'a str,
        max_dave_protocol_version: u16,
    }
    to_json(
        op::IDENTIFY,
        Identify {
            server_id: s.server_id.to_string(),
            user_id: s.user_id.to_string(),
            session_id: &s.session_id,
            token: &s.token,
            max_dave_protocol_version: DAVE_PROTOCOL_VERSION,
        },
    )
}

pub fn resume(s: &SessionInfo, seq_ack: Option<u64>) -> String {
    #[derive(Serialize)]
    struct Resume<'a> {
        server_id: String,
        session_id: &'a str,
        token: &'a str,
        seq_ack: i64,
    }
    to_json(
        op::RESUME,
        Resume {
            server_id: s.server_id.to_string(),
            session_id: &s.session_id,
            token: &s.token,
            seq_ack: seq_ack.map_or(-1, |s| i64::try_from(s).unwrap_or(i64::MAX)),
        },
    )
}

pub fn heartbeat(nonce: u64, seq_ack: Option<u64>) -> String {
    #[derive(Serialize)]
    struct Beat {
        t: u64,
        seq_ack: i64,
    }
    to_json(
        op::HEARTBEAT,
        Beat {
            t: nonce,
            seq_ack: seq_ack.map_or(-1, |s| i64::try_from(s).unwrap_or(i64::MAX)),
        },
    )
}

pub fn select_protocol(address: &str, port: u16, mode: &str) -> String {
    #[derive(Serialize)]
    struct Data<'a> {
        address: &'a str,
        port: u16,
        mode: &'a str,
    }
    #[derive(Serialize)]
    struct Select<'a> {
        protocol: &'static str,
        data: Data<'a>,
    }
    to_json(
        op::SELECT_PROTOCOL,
        Select {
            protocol: "udp",
            data: Data {
                address,
                port,
                mode,
            },
        },
    )
}

pub fn speaking(ssrc: u32, on: bool) -> String {
    #[derive(Serialize)]
    struct Speaking {
        speaking: u8,
        delay: u8,
        ssrc: u32,
    }
    to_json(
        op::SPEAKING,
        Speaking {
            speaking: if on { SPEAKING_MICROPHONE } else { 0 },
            delay: 0,
            ssrc,
        },
    )
}

pub fn transition_ready(transition_id: u16) -> String {
    #[derive(Serialize)]
    struct Ready {
        transition_id: u16,
    }
    to_json(op::DAVE_TRANSITION_READY, Ready { transition_id })
}

pub fn invalid_commit_welcome(transition_id: u16) -> String {
    #[derive(Serialize)]
    struct Invalid {
        transition_id: u16,
    }
    to_json(
        op::DAVE_MLS_INVALID_COMMIT_WELCOME,
        Invalid { transition_id },
    )
}

/// Sunucudan gelen JSON mesajları.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Hello {
        heartbeat_interval_ms: f64,
    },
    Ready {
        ssrc: u32,
        ip: String,
        port: u16,
        modes: Vec<String>,
    },
    SessionDescription {
        mode: String,
        secret_key: Vec<u8>,
        dave_protocol_version: u16,
    },
    HeartbeatAck,
    Resumed,
    Speaking {
        user_id: u64,
        ssrc: u32,
        speaking: u8,
    },
    ClientsConnect(Vec<u64>),
    ClientDisconnect(u64),
    DavePrepareTransition {
        transition_id: u16,
        protocol_version: u16,
    },
    DaveExecuteTransition {
        transition_id: u16,
    },
    DavePrepareEpoch {
        epoch: u64,
        protocol_version: u16,
    },
    Other(u8),
}

#[derive(Deserialize)]
struct Envelope<'a> {
    op: u8,
    seq: Option<u64>,
    #[serde(borrow)]
    d: Option<&'a RawValue>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("op {0} için veri yok")]
    MissingData(u8),
}

/// Snowflake'ler string gelir.
fn id_de<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Id {
        S(String),
        N(u64),
    }
    match Id::deserialize(d)? {
        Id::S(s) => s.parse().map_err(serde::de::Error::custom),
        Id::N(n) => Ok(n),
    }
}

fn ids_de<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u64>, D::Error> {
    #[derive(Deserialize)]
    struct W(#[serde(deserialize_with = "id_de")] u64);
    Ok(Vec::<W>::deserialize(d)?.into_iter().map(|w| w.0).collect())
}

/// Bir JSON mesajını ayrıştırır; `seq` varsa birlikte döner.
pub fn parse(text: &str) -> Result<(Option<u64>, Event), ProtocolError> {
    let env: Envelope<'_> = serde_json::from_str(text)?;
    let raw = || {
        env.d
            .map(RawValue::get)
            .ok_or(ProtocolError::MissingData(env.op))
    };
    let event = match env.op {
        op::HELLO => {
            #[derive(Deserialize)]
            struct H {
                heartbeat_interval: f64,
            }
            let h: H = serde_json::from_str(raw()?)?;
            Event::Hello {
                heartbeat_interval_ms: h.heartbeat_interval,
            }
        }
        op::READY => {
            #[derive(Deserialize)]
            struct R {
                ssrc: u32,
                ip: String,
                port: u16,
                #[serde(default)]
                modes: Vec<String>,
            }
            let r: R = serde_json::from_str(raw()?)?;
            Event::Ready {
                ssrc: r.ssrc,
                ip: r.ip,
                port: r.port,
                modes: r.modes,
            }
        }
        op::SESSION_DESCRIPTION => {
            #[derive(Deserialize)]
            struct S {
                mode: String,
                secret_key: Vec<u8>,
                #[serde(default)]
                dave_protocol_version: u16,
            }
            let s: S = serde_json::from_str(raw()?)?;
            Event::SessionDescription {
                mode: s.mode,
                secret_key: s.secret_key,
                dave_protocol_version: s.dave_protocol_version,
            }
        }
        op::HEARTBEAT_ACK => Event::HeartbeatAck,
        op::RESUMED => Event::Resumed,
        op::SPEAKING => {
            #[derive(Deserialize)]
            struct Sp {
                #[serde(deserialize_with = "id_de")]
                user_id: u64,
                ssrc: u32,
                #[serde(default)]
                speaking: u8,
            }
            let s: Sp = serde_json::from_str(raw()?)?;
            Event::Speaking {
                user_id: s.user_id,
                ssrc: s.ssrc,
                speaking: s.speaking,
            }
        }
        op::CLIENTS_CONNECT => {
            #[derive(Deserialize)]
            struct C {
                #[serde(deserialize_with = "ids_de")]
                user_ids: Vec<u64>,
            }
            Event::ClientsConnect(serde_json::from_str::<C>(raw()?)?.user_ids)
        }
        op::CLIENT_DISCONNECT => {
            #[derive(Deserialize)]
            struct D {
                #[serde(deserialize_with = "id_de")]
                user_id: u64,
            }
            Event::ClientDisconnect(serde_json::from_str::<D>(raw()?)?.user_id)
        }
        op::DAVE_PREPARE_TRANSITION => {
            #[derive(Deserialize)]
            struct P {
                transition_id: u16,
                protocol_version: u16,
            }
            let p: P = serde_json::from_str(raw()?)?;
            Event::DavePrepareTransition {
                transition_id: p.transition_id,
                protocol_version: p.protocol_version,
            }
        }
        op::DAVE_EXECUTE_TRANSITION => {
            #[derive(Deserialize)]
            struct X {
                transition_id: u16,
            }
            Event::DaveExecuteTransition {
                transition_id: serde_json::from_str::<X>(raw()?)?.transition_id,
            }
        }
        op::DAVE_PREPARE_EPOCH => {
            #[derive(Deserialize)]
            struct E {
                epoch: u64,
                protocol_version: u16,
            }
            let e: E = serde_json::from_str(raw()?)?;
            Event::DavePrepareEpoch {
                epoch: e.epoch,
                protocol_version: e.protocol_version,
            }
        }
        other => Event::Other(other),
    };
    Ok((env.seq, event))
}

/// Ses gateway'i kapanış kodları.
pub mod close {
    pub const UNKNOWN_OPCODE: u16 = 4001;
    pub const DECODE_ERROR: u16 = 4002;
    pub const NOT_AUTHENTICATED: u16 = 4003;
    pub const AUTHENTICATION_FAILED: u16 = 4004;
    pub const ALREADY_AUTHENTICATED: u16 = 4005;
    pub const SESSION_INVALID: u16 = 4006;
    pub const SESSION_TIMEOUT: u16 = 4009;
    pub const SERVER_NOT_FOUND: u16 = 4011;
    pub const UNKNOWN_PROTOCOL: u16 = 4012;
    pub const DISCONNECTED: u16 = 4014;
    pub const SERVER_CRASHED: u16 = 4015;
    pub const UNKNOWN_ENCRYPTION_MODE: u16 = 4016;
    pub const E2EE_REQUIRED: u16 = 4017;
    pub const BAD_REQUEST: u16 = 4020;
    pub const RATE_LIMITED: u16 = 4021;
    pub const CALL_TERMINATED: u16 = 4022;

    /// Resume denenebilir mi (aksi halde yeni oturum ya da vazgeç).
    pub fn resumable(code: u16) -> bool {
        !matches!(
            code,
            AUTHENTICATION_FAILED
                | SESSION_INVALID
                | SERVER_NOT_FOUND
                | UNKNOWN_PROTOCOL
                | DISCONNECTED
                | UNKNOWN_ENCRYPTION_MODE
                | E2EE_REQUIRED
                | BAD_REQUEST
                | CALL_TERMINATED
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> SessionInfo {
        SessionInfo {
            server_id: 1,
            user_id: 2,
            session_id: "s".into(),
            token: "gizli".into(),
            endpoint: "c-ist01.discord.media:443".into(),
        }
    }

    fn json(s: &str) -> serde_json::Value {
        serde_json::from_str(s).expect("json")
    }

    #[test]
    fn outgoing_messages() {
        assert_eq!(
            json(&identify(&info())),
            serde_json::json!({"op": 0, "d": {"server_id": "1", "user_id": "2", "session_id": "s",
                "token": "gizli", "max_dave_protocol_version": 1}})
        );
        assert_eq!(
            json(&heartbeat(5, None)),
            serde_json::json!({"op": 3, "d": {"t": 5, "seq_ack": -1}})
        );
        assert_eq!(json(&resume(&info(), Some(9)))["d"]["seq_ack"], 9);
        assert_eq!(
            json(&select_protocol("1.2.3.4", 5, "aead_aes256_gcm_rtpsize")),
            serde_json::json!({"op": 1, "d": {"protocol": "udp",
                "data": {"address": "1.2.3.4", "port": 5, "mode": "aead_aes256_gcm_rtpsize"}}})
        );
        assert_eq!(
            json(&speaking(7, true)),
            serde_json::json!({"op": 5, "d": {"speaking": 1, "delay": 0, "ssrc": 7}})
        );
        assert_eq!(
            json(&transition_ready(3)),
            serde_json::json!({"op": 23, "d": {"transition_id": 3}})
        );
        assert!(!format!("{:?}", info()).contains("gizli"));
        assert_eq!(info().url(), "wss://c-ist01.discord.media:443/?v=8");
    }

    #[test]
    fn incoming_messages() {
        assert_eq!(
            parse(r#"{"op":8,"d":{"heartbeat_interval":13750.5}}"#).expect("hello"),
            (
                None,
                Event::Hello {
                    heartbeat_interval_ms: 13750.5
                }
            )
        );
        let (seq, ev) = parse(
            r#"{"op":2,"seq":1,"d":{"ssrc":42,"ip":"1.2.3.4","port":50000,"modes":["aead_aes256_gcm_rtpsize"],"experiments":[]}}"#,
        )
        .expect("ready");
        assert_eq!(seq, Some(1));
        assert!(matches!(
            ev,
            Event::Ready {
                ssrc: 42,
                port: 50000,
                ..
            }
        ));
        let (_, ev) = parse(
            r#"{"op":4,"seq":2,"d":{"mode":"aead_aes256_gcm_rtpsize","secret_key":[1,2,3],"dave_protocol_version":1}}"#,
        )
        .expect("session");
        assert_eq!(
            ev,
            Event::SessionDescription {
                mode: "aead_aes256_gcm_rtpsize".into(),
                secret_key: vec![1, 2, 3],
                dave_protocol_version: 1
            }
        );
        assert_eq!(
            parse(r#"{"op":11,"seq":3,"d":{"user_ids":["5","6"]}}"#)
                .expect("connect")
                .1,
            Event::ClientsConnect(vec![5, 6])
        );
        assert_eq!(
            parse(r#"{"op":5,"seq":4,"d":{"user_id":"5","ssrc":9,"speaking":1}}"#)
                .expect("speaking")
                .1,
            Event::Speaking {
                user_id: 5,
                ssrc: 9,
                speaking: 1
            }
        );
        assert_eq!(
            parse(r#"{"op":21,"seq":5,"d":{"transition_id":2,"protocol_version":0}}"#)
                .expect("prepare")
                .1,
            Event::DavePrepareTransition {
                transition_id: 2,
                protocol_version: 0
            }
        );
        assert_eq!(
            parse(r#"{"op":6,"d":{"t":5}}"#).expect("ack").1,
            Event::HeartbeatAck
        );
        assert_eq!(
            parse(r#"{"op":99,"d":{}}"#).expect("diğer").1,
            Event::Other(99)
        );
        assert!(parse(r#"{"op":2,"d":{"ssrc":"x"}}"#).is_err());
    }

    #[test]
    fn close_codes() {
        assert!(!close::resumable(close::E2EE_REQUIRED));
        assert!(!close::resumable(close::DISCONNECTED));
        assert!(close::resumable(close::SERVER_CRASHED));
        assert!(close::resumable(close::SESSION_TIMEOUT));
    }
}
