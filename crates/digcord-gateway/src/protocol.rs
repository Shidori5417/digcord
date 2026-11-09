//! Gateway mesajları: ayrıştırma, giden payload'lar, close kodları, geri çekilme
//! ve heartbeat takibi. IO yok; hepsi birim testli.
//! Kaynak: docs/PROTOCOL_NOTES.md §3 ve §3.1.

use std::time::Duration;

use digcord_model::{ClientProperties, Ready, Token};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

/// Gözlenen web istemcisi URL'si (parametre sırası dahil).
pub const GATEWAY_URL: &str = "wss://gateway.discord.gg/?encoding=json&v=9&compress=zlib-stream";
/// `resume_gateway_url` sorgu parametresiz gelir; bunlar eklenir.
pub const GATEWAY_QUERY: &str = "/?encoding=json&v=9&compress=zlib-stream";

/// Gözlenen IDENTIFY `capabilities` değeri.
const CAPABILITIES: u64 = 1_767_421;
/// Gözlenen op 40 `qos.ver` değeri.
const QOS_VERSION: u32 = 31;

pub mod op {
    pub const DISPATCH: u8 = 0;
    pub const HEARTBEAT: u8 = 1;
    pub const IDENTIFY: u8 = 2;
    pub const RESUME: u8 = 6;
    pub const RECONNECT: u8 = 7;
    pub const INVALID_SESSION: u8 = 9;
    pub const HELLO: u8 = 10;
    pub const HEARTBEAT_ACK: u8 = 11;
    /// Web istemcisinin op 1 yerine kullandığı QoS heartbeat (PROTOCOL_NOTES §3.1).
    pub const QOS_HEARTBEAT: u8 = 40;
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("gateway mesajı ayrıştırılamadı: {0}")]
    Json(#[from] serde_json::Error),
    #[error("gateway mesajında `d` eksik (op {0})")]
    MissingData(u8),
}

/// Sunucudan gelen, işlenmiş mesaj.
#[derive(Debug)]
pub enum Event {
    Hello {
        heartbeat_interval: Duration,
    },
    Dispatch {
        seq: Option<u64>,
        dispatch: Dispatch,
    },
    HeartbeatRequest,
    HeartbeatAck,
    Reconnect,
    InvalidSession {
        resumable: bool,
    },
    Unknown {
        op: u8,
    },
}

#[derive(Debug)]
pub enum Dispatch {
    Ready(Box<Ready>),
    Resumed,
    /// Bu aşamada işlenmeyen olay; `d` hiç ayrıştırılmaz.
    Other,
}

#[derive(Deserialize)]
struct Envelope<'a> {
    op: u8,
    s: Option<u64>,
    #[serde(borrow)]
    t: Option<&'a str>,
    #[serde(borrow)]
    d: Option<&'a RawValue>,
}

/// Önce sadece zarf (`op`, `s`, `t`) ayrıştırılır; `d` gerekirse hedef tipe çevrilir.
pub fn parse(text: &str) -> Result<Event, ProtocolError> {
    let env: Envelope<'_> = serde_json::from_str(text)?;
    let data = || env.d.ok_or(ProtocolError::MissingData(env.op));
    Ok(match env.op {
        op::DISPATCH => {
            let dispatch = match env.t {
                Some("READY") => Dispatch::Ready(serde_json::from_str(data()?.get())?),
                Some("RESUMED") => Dispatch::Resumed,
                _ => Dispatch::Other,
            };
            Event::Dispatch {
                seq: env.s,
                dispatch,
            }
        }
        op::HELLO => {
            #[derive(Deserialize)]
            struct Hello {
                heartbeat_interval: u64,
            }
            let h: Hello = serde_json::from_str(data()?.get())?;
            Event::Hello {
                heartbeat_interval: Duration::from_millis(h.heartbeat_interval),
            }
        }
        op::HEARTBEAT => Event::HeartbeatRequest,
        op::HEARTBEAT_ACK => Event::HeartbeatAck,
        op::RECONNECT => Event::Reconnect,
        op::INVALID_SESSION => Event::InvalidSession {
            resumable: env.d.is_some_and(|d| d.get() == "true"),
        },
        op => Event::Unknown { op },
    })
}

#[derive(Serialize)]
struct Payload<D> {
    op: u8,
    d: D,
}

fn to_json<D: Serialize>(op: u8, d: D) -> String {
    // Sabit yapılı, string anahtarlı tipler; serileştirme başarısız olamaz.
    serde_json::to_string(&Payload { op, d }).unwrap_or_default()
}

/// op 2. Alanlar ve sıraları gözlenen web istemcisiyle aynı. Dönen metin token
/// içerir: **asla log'a yazma.**
pub fn identify(token: &Token, properties: &ClientProperties) -> String {
    #[derive(Serialize)]
    struct Presence {
        status: &'static str,
        since: u64,
        activities: [(); 0],
        afk: bool,
    }
    #[derive(Serialize)]
    struct ClientState {
        guild_versions: Empty,
    }
    #[derive(Serialize)]
    struct Empty {}
    #[derive(Serialize)]
    struct Identify<'a> {
        token: &'a Token,
        capabilities: u64,
        properties: &'a ClientProperties,
        presence: Presence,
        compress: bool,
        client_state: ClientState,
    }
    to_json(
        op::IDENTIFY,
        Identify {
            token,
            capabilities: CAPABILITIES,
            properties,
            presence: Presence {
                status: "unknown",
                since: 0,
                activities: [],
                afk: false,
            },
            compress: false,
            client_state: ClientState {
                guild_versions: Empty {},
            },
        },
    )
}

/// op 6. Web istemcisinin resume payload'ı henüz gözlenmedi (doğrulanmadı);
/// standart alanlar kullanılıyor. Token içerir: **asla log'a yazma.**
pub fn resume(token: &Token, session_id: &str, seq: Option<u64>) -> String {
    #[derive(Serialize)]
    struct Resume<'a> {
        token: &'a Token,
        session_id: &'a str,
        seq: Option<u64>,
    }
    to_json(
        op::RESUME,
        Resume {
            token,
            session_id,
            seq,
        },
    )
}

/// op 40 QoS heartbeat (web istemcisi op 1 göndermiyor, PROTOCOL_NOTES §3.1).
pub fn heartbeat(seq: Option<u64>) -> String {
    #[derive(Serialize)]
    struct Qos {
        active: bool,
        ver: u32,
        reasons: [(); 0],
    }
    #[derive(Serialize)]
    struct Beat {
        seq: Option<u64>,
        qos: Qos,
    }
    to_json(
        op::QOS_HEARTBEAT,
        Beat {
            seq,
            qos: Qos {
                active: false,
                ver: QOS_VERSION,
                reasons: [],
            },
        },
    )
}

/// Bağlantı kapandığında ne yapılacağı.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// Aynı oturumla resume dene.
    Resume,
    /// Oturum geçersiz; yeni IDENTIFY.
    Reidentify,
    /// Token geçersiz (4004): token silinip giriş ekranına dönülür.
    InvalidToken,
    /// Yeniden denemenin anlamı yok.
    Fatal,
}

/// Close kodu tablosu — tek yer. `None`: close çerçevesi olmadan kopan bağlantı.
pub fn classify_close(code: Option<u16>) -> CloseAction {
    match code {
        Some(4004) => CloseAction::InvalidToken,
        // 4003 kimlik doğrulanmadı, 4007 geçersiz seq, 4009 oturum zaman aşımı
        Some(4003 | 4007 | 4009) => CloseAction::Reidentify,
        // 4010-4014: shard / API sürümü / intents hataları — bizim hatamız, tekrar denemek boşuna
        Some(4010..=4014) => CloseAction::Fatal,
        // 4000-4002, 4005, 4008, 1000/1001 dahil diğerleri: resume edilebilir
        _ => CloseAction::Resume,
    }
}

/// Üstel geri çekilme + jitter, en fazla ~1 dk.
#[derive(Debug, Default)]
pub struct Backoff {
    attempt: u32,
}

impl Backoff {
    const MAX: Duration = Duration::from_secs(60);

    /// Sonraki bekleme: `min(60 sn, 1 sn × 2^n) × [0.5, 1.0)`.
    pub fn next_delay(&mut self) -> Duration {
        let base = Duration::from_secs(1u64 << self.attempt.min(6)).min(Self::MAX);
        self.attempt = self.attempt.saturating_add(1);
        base.mul_f64(0.5 + fastrand::f64() * 0.5)
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

/// Heartbeat ve zombi bağlantı takibi.
#[derive(Debug)]
pub struct Heartbeat {
    interval: Duration,
    awaiting_ack: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Beat {
    Send,
    /// Önceki vuruşun ACK'i gelmedi: bağlantı ölü sayılır, kapatılıp resume edilir.
    Zombie,
}

impl Heartbeat {
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            awaiting_ack: false,
        }
    }

    pub fn interval(&self) -> Duration {
        self.interval
    }

    /// İlk vuruşa kadar bekleme: `interval × jitter`.
    pub fn first_delay(&self) -> Duration {
        self.interval.mul_f64(fastrand::f64())
    }

    /// Zamanlayıcı doldu.
    pub fn on_tick(&mut self) -> Beat {
        if self.awaiting_ack {
            return Beat::Zombie;
        }
        self.awaiting_ack = true;
        Beat::Send
    }

    /// Zamanlayıcı dışında gönderilen vuruş (READY sonrası, sunucu op 1 isteği).
    pub fn on_extra_sent(&mut self) {
        self.awaiting_ack = true;
    }

    pub fn on_ack(&mut self) {
        self.awaiting_ack = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn props() -> ClientProperties {
        ClientProperties::chrome_linux("tr-TR", 155, 667_580)
    }

    #[test]
    fn parses_hello() {
        let e =
            parse(r#"{"t":null,"s":null,"op":10,"d":{"heartbeat_interval":41250,"_trace":["x"]}}"#)
                .expect("hello");
        assert!(
            matches!(e, Event::Hello { heartbeat_interval } if heartbeat_interval == Duration::from_millis(41_250))
        );
    }

    #[test]
    fn parses_control_ops() {
        let cases = [
            (r#"{"t":null,"s":null,"op":11,"d":null}"#, "ack"),
            (r#"{"op":1,"d":null}"#, "req"),
            (r#"{"op":7,"d":null}"#, "reconnect"),
            (r#"{"op":9,"d":true}"#, "inv-true"),
            (r#"{"op":9,"d":false}"#, "inv-false"),
            (r#"{"op":99,"d":{}}"#, "unknown"),
        ];
        for (json, name) in cases {
            let e = parse(json).expect(name);
            let ok = match name {
                "ack" => matches!(e, Event::HeartbeatAck),
                "req" => matches!(e, Event::HeartbeatRequest),
                "reconnect" => matches!(e, Event::Reconnect),
                "inv-true" => matches!(e, Event::InvalidSession { resumable: true }),
                "inv-false" => matches!(e, Event::InvalidSession { resumable: false }),
                _ => matches!(e, Event::Unknown { op: 99 }),
            };
            assert!(ok, "{name}: {e:?}");
        }
    }

    #[test]
    fn parses_ready_dispatch() {
        let ready = include_str!("../../digcord-model/tests/fixtures/ready.json");
        let msg = format!(r#"{{"t":"READY","s":1,"op":0,"d":{ready}}}"#);
        match parse(&msg).expect("ready") {
            Event::Dispatch {
                seq: Some(1),
                dispatch: Dispatch::Ready(r),
            } => assert_eq!(r.guilds.len(), 3),
            e => panic!("beklenmeyen: {e:?}"),
        }
    }

    #[test]
    fn unknown_dispatch_is_not_parsed() {
        // Sözdizimi bozuk JSON her durumda reddedilir.
        let e = parse(r#"{"t":"TYPING_START","s":5,"op":0,"d":{"bozuk":}}"#);
        assert!(e.is_err());
        // İşlenmeyen olayın `d`'si hiçbir tipe çevrilmez; şekli ne olursa olsun hata yok.
        let e = parse(r#"{"t":"TYPING_START","s":5,"op":0,"d":12345}"#).expect("diğer");
        assert!(matches!(
            e,
            Event::Dispatch {
                seq: Some(5),
                dispatch: Dispatch::Other
            }
        ));
    }

    #[test]
    fn missing_data_is_error() {
        assert!(matches!(
            parse(r#"{"op":10,"d":null}"#),
            Err(ProtocolError::MissingData(10))
        ));
    }

    #[test]
    fn identify_matches_observed_shape() {
        let token = Token::new("sahte-token").expect("token");
        let v: Value = serde_json::from_str(&identify(&token, &props())).expect("json");
        assert_eq!(v["op"], 2);
        let d = &v["d"];
        let keys: Vec<&str> = d
            .as_object()
            .expect("nesne")
            .keys()
            .map(String::as_str)
            .collect();
        // serde_json varsayılanı anahtarları sıralar; sıra testi aşağıda metin üzerinde.
        assert_eq!(keys.len(), 6, "{keys:?}");
        assert_eq!(d["token"], "sahte-token");
        assert_eq!(d["capabilities"], 1_767_421);
        assert_eq!(d["properties"]["browser"], "Chrome");
        assert_eq!(
            d["presence"],
            serde_json::json!({"status":"unknown","since":0,"activities":[],"afk":false})
        );
        assert_eq!(d["compress"], false);
        assert_eq!(d["client_state"], serde_json::json!({"guild_versions":{}}));
        assert!(d.get("intents").is_none());

        let text = identify(&token, &props());
        let order = [
            "\"token\"",
            "\"capabilities\"",
            "\"properties\"",
            "\"presence\"",
            "\"compress\"",
            "\"client_state\"",
        ];
        let pos: Vec<usize> = order
            .iter()
            .map(|k| text.find(k).expect("anahtar"))
            .collect();
        assert!(pos.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    }

    #[test]
    fn heartbeat_matches_observed() {
        assert_eq!(
            heartbeat(Some(7)),
            r#"{"op":40,"d":{"seq":7,"qos":{"active":false,"ver":31,"reasons":[]}}}"#
        );
        assert_eq!(
            heartbeat(None),
            r#"{"op":40,"d":{"seq":null,"qos":{"active":false,"ver":31,"reasons":[]}}}"#
        );
    }

    #[test]
    fn resume_payload() {
        let token = Token::new("sahte-token").expect("token");
        assert_eq!(
            resume(&token, "abc", Some(42)),
            r#"{"op":6,"d":{"token":"sahte-token","session_id":"abc","seq":42}}"#
        );
    }

    #[test]
    fn close_code_table() {
        use CloseAction::*;
        for (code, want) in [
            (None, Resume),
            (Some(1000), Resume),
            (Some(1006), Resume),
            (Some(4000), Resume),
            (Some(4008), Resume),
            (Some(4003), Reidentify),
            (Some(4007), Reidentify),
            (Some(4009), Reidentify),
            (Some(4004), InvalidToken),
            (Some(4010), Fatal),
            (Some(4014), Fatal),
        ] {
            assert_eq!(classify_close(code), want, "{code:?}");
        }
    }

    #[test]
    fn backoff_grows_caps_and_resets() {
        let mut b = Backoff::default();
        let first = b.next_delay();
        assert!(first >= Duration::from_millis(500) && first < Duration::from_secs(1));
        for _ in 0..20 {
            assert!(b.next_delay() <= Duration::from_secs(60));
        }
        let late = b.next_delay();
        assert!(late >= Duration::from_secs(30), "{late:?}");
        b.reset();
        assert!(b.next_delay() < Duration::from_secs(1));
    }

    #[test]
    fn heartbeat_detects_zombie() {
        let mut hb = Heartbeat::new(Duration::from_millis(41_250));
        assert!(hb.first_delay() <= hb.interval());
        assert_eq!(hb.on_tick(), Beat::Send);
        hb.on_ack();
        assert_eq!(hb.on_tick(), Beat::Send);
        // ACK gelmeden ikinci vuruş zamanı → zombi
        assert_eq!(hb.on_tick(), Beat::Zombie);
    }

    #[test]
    fn extra_beat_needs_ack_too() {
        let mut hb = Heartbeat::new(Duration::from_secs(1));
        hb.on_extra_sent();
        assert_eq!(hb.on_tick(), Beat::Zombie);
        hb.on_ack();
        assert_eq!(hb.on_tick(), Beat::Send);
    }
}
