//! Gateway mesajları: ayrıştırma, giden payload'lar, close kodları, geri çekilme
//! ve heartbeat takibi. IO yok; hepsi birim testli.
//! Kaynak: docs/PROTOCOL_NOTES.md §3 ve §3.1.

use std::time::Duration;

use digcord_model::{
    Channel, ClientProperties, Guild, GuildMembersChunk, MemberListUpdate, Message, MessageAck,
    MessageDelete, ReactionEvent, Ready, Snowflake, ThreadListSync, Token, TypingStart,
    VoiceServer, VoiceState,
};
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
/// Gözlenen op 4 `flags` değeri.
const VOICE_STATE_FLAGS: u8 = 2;

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
    pub const VOICE_STATE_UPDATE: u8 = 4;
    /// Web istemcisinin IDENTIFY sonrası gönderdiği başlatma bilgisi (PROTOCOL_NOTES §3.2).
    pub const CLIENT_INIT: u8 = 41;
    pub const REQUEST_GUILD_MEMBERS: u8 = 8;
    pub const GUILD_SUBSCRIPTIONS_BULK: u8 = 37;
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
    Event(GatewayEvent),
    /// İşlenmeyen olay ya da `d`'si beklenen biçimde olmayan olay (bağlantı düşürülmez).
    Other,
}

/// Uygulamaya iletilen olaylar.
#[derive(Debug)]
pub enum GatewayEvent {
    MessageCreate(Box<Message>),
    MessageUpdate(Box<Message>),
    MessageDelete(MessageDelete),
    ChannelCreate(Box<Channel>),
    ChannelUpdate(Box<Channel>),
    ChannelDelete(Box<Channel>),
    /// Thread olayları: biçim kanal nesnesidir (silmede sadece `id`, `guild_id`, `parent_id`,
    /// `type`). Kullanıcı hesabı davranışı doğrulanmadı (PROTOCOL_NOTES §4.4).
    ThreadCreate(Box<Channel>),
    ThreadUpdate(Box<Channel>),
    ThreadDelete(Box<Channel>),
    ThreadListSync(ThreadListSync),
    GuildCreate(Box<Guild>),
    GuildMembersChunk(GuildMembersChunk),
    /// Kanal üye listesi güncellemesi (PROTOCOL_NOTES §4.6).
    MemberListUpdate(Box<MemberListUpdate>),
    ReactionAdd(ReactionEvent),
    ReactionRemove(ReactionEvent),
    TypingStart(TypingStart),
    /// Başka oturumda okundu işareti (alanlar doğrulanmadı).
    MessageAck(MessageAck),
    VoiceStateUpdate(Box<VoiceState>),
    VoiceServerUpdate(VoiceServer),
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
                // READY bozuksa oturum kullanılamaz: hata.
                Some("READY") => Dispatch::Ready(serde_json::from_str(data()?.get())?),
                Some("RESUMED") => Dispatch::Resumed,
                Some(t) => match env.d {
                    Some(d) => parse_event(t, d.get()),
                    None => Dispatch::Other,
                },
                None => Dispatch::Other,
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

/// Bilinen olayın `d`'sini çevirir. Biçim beklenmedikse olay atlanır, bağlantı sürer.
fn parse_event(t: &str, d: &str) -> Dispatch {
    use GatewayEvent as E;
    fn de<T: serde::de::DeserializeOwned>(t: &str, d: &str) -> Option<T> {
        serde_json::from_str(d)
            .map_err(|e| tracing::warn!(event = t, "gateway olayı ayrıştırılamadı: {e}"))
            .ok()
    }
    let event = match t {
        "MESSAGE_CREATE" => de(t, d).map(|m| E::MessageCreate(Box::new(m))),
        "MESSAGE_UPDATE" => de(t, d).map(|m| E::MessageUpdate(Box::new(m))),
        "MESSAGE_DELETE" => de(t, d).map(E::MessageDelete),
        "CHANNEL_CREATE" => de(t, d).map(|c| E::ChannelCreate(Box::new(c))),
        "CHANNEL_UPDATE" => de(t, d).map(|c| E::ChannelUpdate(Box::new(c))),
        "CHANNEL_DELETE" => de(t, d).map(|c| E::ChannelDelete(Box::new(c))),
        "THREAD_CREATE" => de(t, d).map(|c| E::ThreadCreate(Box::new(c))),
        "THREAD_UPDATE" => de(t, d).map(|c| E::ThreadUpdate(Box::new(c))),
        "THREAD_DELETE" => de(t, d).map(|c| E::ThreadDelete(Box::new(c))),
        "THREAD_LIST_SYNC" => de(t, d).map(E::ThreadListSync),
        "GUILD_CREATE" => de(t, d).map(|g| E::GuildCreate(Box::new(g))),
        "GUILD_MEMBERS_CHUNK" => de(t, d).map(E::GuildMembersChunk),
        "GUILD_MEMBER_LIST_UPDATE" => MemberListUpdate::parse(d)
            .map(|u| E::MemberListUpdate(Box::new(u)))
            .or_else(|| {
                tracing::warn!(event = t, "gateway olayı ayrıştırılamadı");
                None
            }),
        "MESSAGE_REACTION_ADD" => de(t, d).map(E::ReactionAdd),
        "MESSAGE_REACTION_REMOVE" => de(t, d).map(E::ReactionRemove),
        "TYPING_START" => de(t, d).map(E::TypingStart),
        "MESSAGE_ACK" => de(t, d).map(E::MessageAck),
        "VOICE_STATE_UPDATE" => de(t, d).map(E::VoiceStateUpdate),
        "VOICE_SERVER_UPDATE" => de(t, d).map(E::VoiceServerUpdate),
        _ => None,
    };
    event.map_or(Dispatch::Other, Dispatch::Event)
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

/// op 37 guild aboneliği: web istemcisi küçük/büyük her guild için aynı tam biçimi gönderir
/// (PROTOCOL_NOTES §4.1).
pub fn subscribe_guild(guild: Snowflake) -> String {
    #[derive(Serialize)]
    struct Sub {
        typing: bool,
        threads: bool,
        activities: bool,
        members: [(); 0],
        member_updates: bool,
        channels: Empty,
        thread_member_lists: [(); 0],
    }
    #[derive(Serialize)]
    struct Empty {}
    to_json(
        op::GUILD_SUBSCRIPTIONS_BULK,
        Subs {
            subscriptions: [(
                guild.to_string(),
                Sub {
                    typing: true,
                    threads: true,
                    activities: true,
                    members: [],
                    member_updates: false,
                    channels: Empty {},
                    thread_member_lists: [],
                },
            )]
            .into(),
        },
    )
}

#[derive(Serialize)]
struct Subs<T> {
    subscriptions: std::collections::BTreeMap<String, T>,
}

/// op 37, üye listesi görünürken: kanalın istenen aralıkları (`[[0,99]]`, kaydırınca
/// `[[0,99],[100,199]]`; çoklu aralık doğrulanmadı, PROTOCOL_NOTES §4.6).
pub fn subscribe_channels(guild: Snowflake, channel: Snowflake, ranges: &[(u32, u32)]) -> String {
    #[derive(Serialize)]
    struct Sub<'a> {
        channels: std::collections::BTreeMap<String, &'a [(u32, u32)]>,
    }
    to_json(
        op::GUILD_SUBSCRIPTIONS_BULK,
        Subs {
            subscriptions: [(
                guild.to_string(),
                Sub {
                    channels: [(channel.to_string(), ranges)].into(),
                },
            )]
            .into(),
        },
    )
}

/// op 8: mesaj yazarlarının üye bilgisi (takma ad, roller) — `presences: false` (gözlenen).
pub fn request_members(guild: Snowflake, user_ids: &[Snowflake]) -> String {
    #[derive(Serialize)]
    struct Req<'a> {
        guild_id: [Snowflake; 1],
        user_ids: &'a [Snowflake],
        presences: bool,
    }
    to_json(
        op::REQUEST_GUILD_MEMBERS,
        Req {
            guild_id: [guild],
            user_ids,
            presences: false,
        },
    )
}

/// op 4: ses kanalına katıl (`channel: None` → ayrıl). DM aramasında `guild: None`.
/// Alan seti belgelere göre + `flags: 2` (boştaki op 4'te gözlendi; katılırken aynı olduğu
/// varsayımı doğrulanmadı, PROTOCOL_NOTES §3.2).
pub fn voice_state_update(
    guild: Option<Snowflake>,
    channel: Option<Snowflake>,
    self_mute: bool,
    self_deaf: bool,
) -> String {
    #[derive(Serialize)]
    struct Vs {
        guild_id: Option<Snowflake>,
        channel_id: Option<Snowflake>,
        self_mute: bool,
        self_deaf: bool,
        self_video: bool,
        flags: u8,
    }
    to_json(
        op::VOICE_STATE_UPDATE,
        Vs {
            guild_id: guild,
            channel_id: channel,
            self_mute,
            self_deaf,
            self_video: false,
            flags: VOICE_STATE_FLAGS,
        },
    )
}

/// IDENTIFY'dan hemen sonra gönderilen op 4 (kanalsız, `self_mute: true`; gözlenen).
pub fn initial_voice_state() -> String {
    voice_state_update(None, None, true, false)
}

/// op 41: `initialization_timestamp` (unix ms), `session_id` = `client_heartbeat_session_id`,
/// `client_launch_id`; hepsi [`ClientProperties`]'ten.
pub fn client_init(properties: &ClientProperties) -> String {
    #[derive(Serialize)]
    struct Init<'a> {
        initialization_timestamp: u64,
        session_id: &'a str,
        client_launch_id: &'a str,
    }
    to_json(
        op::CLIENT_INIT,
        Init {
            initialization_timestamp: properties.initialization_timestamp_ms(),
            session_id: properties.client_heartbeat_session_id(),
            client_launch_id: properties.client_launch_id(),
        },
    )
}

/// op 40 QoS heartbeat (web istemcisi op 1 göndermiyor, PROTOCOL_NOTES §3.1).
/// `focused`: pencere odakta → `active: true, reasons: ["foregrounded"]` (PROTOCOL_NOTES §3.2).
pub fn heartbeat(seq: Option<u64>, focused: bool) -> String {
    #[derive(Serialize)]
    struct Qos {
        active: bool,
        ver: u32,
        reasons: &'static [&'static str],
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
                active: focused,
                ver: QOS_VERSION,
                reasons: if focused { &["foregrounded"] } else { &[] },
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
    fn parses_message_events_and_survives_bad_shape() {
        let create = r#"{"t":"MESSAGE_CREATE","s":3,"op":0,"d":{"id":"5","channel_id":"2","author":{"id":"1","username":"a","global_name":null},"content":"x"}}"#;
        assert!(matches!(
            parse(create).expect("create"),
            Event::Dispatch { seq: Some(3), dispatch: Dispatch::Event(GatewayEvent::MessageCreate(m)) } if m.content == "x"
        ));
        let delete = r#"{"t":"MESSAGE_DELETE","s":4,"op":0,"d":{"id":"5","channel_id":"2"}}"#;
        assert!(matches!(
            parse(delete).expect("delete"),
            Event::Dispatch {
                dispatch: Dispatch::Event(GatewayEvent::MessageDelete(_)),
                ..
            }
        ));
        // Yazarsız (kısmi) güncelleme: atlanır ama hata değil, seq yine işlenir.
        let partial =
            r#"{"t":"MESSAGE_UPDATE","s":6,"op":0,"d":{"id":"5","channel_id":"2","embeds":[]}}"#;
        assert!(matches!(
            parse(partial).expect("kısmi"),
            Event::Dispatch {
                seq: Some(6),
                dispatch: Dispatch::Other
            }
        ));
        let chunk = r#"{"t":"GUILD_MEMBERS_CHUNK","s":7,"op":0,"d":{"guild_id":"9","members":[{"user":{"id":"1","username":"a","global_name":null},"nick":"N","roles":[]}],"chunk_index":0,"chunk_count":1,"not_found":[]}}"#;
        assert!(matches!(
            parse(chunk).expect("chunk"),
            Event::Dispatch { dispatch: Dispatch::Event(GatewayEvent::GuildMembersChunk(c)), .. } if c.members[0].nick.as_deref() == Some("N")
        ));
    }

    #[test]
    fn parses_thread_events_and_survives_bad_shape() {
        let create = r#"{"t":"THREAD_CREATE","s":1,"op":0,"d":{"id":"30","type":11,"guild_id":"9","parent_id":"2","owner_id":"1","name":"konu","message_count":0,"member_count":1,"thread_metadata":{"archived":false,"locked":false,"archive_timestamp":"2026-01-01T00:00:00.000000+00:00","auto_archive_duration":1440},"member":{"id":"30","user_id":"1","flags":1}}}"#;
        match parse(create).expect("create") {
            Event::Dispatch {
                dispatch: Dispatch::Event(GatewayEvent::ThreadCreate(c)),
                ..
            } => {
                assert_eq!(c.parent_id, Some(Snowflake(2)));
                assert!(c.is_thread() && !c.is_archived() && !c.is_locked());
                assert!(c.member.is_some());
            }
            other => panic!("beklenmeyen: {other:?}"),
        }
        let update = r#"{"t":"THREAD_UPDATE","s":2,"op":0,"d":{"id":"30","type":11,"guild_id":"9","parent_id":"2","name":"konu","thread_metadata":{"archived":true,"locked":true}}}"#;
        assert!(matches!(
            parse(update).expect("update"),
            Event::Dispatch { dispatch: Dispatch::Event(GatewayEvent::ThreadUpdate(c)), .. } if c.is_archived() && c.is_locked()
        ));
        let delete = r#"{"t":"THREAD_DELETE","s":3,"op":0,"d":{"id":"30","guild_id":"9","parent_id":"2","type":11}}"#;
        assert!(matches!(
            parse(delete).expect("delete"),
            Event::Dispatch { dispatch: Dispatch::Event(GatewayEvent::ThreadDelete(c)), .. } if c.id == Snowflake(30)
        ));
        let sync = r#"{"t":"THREAD_LIST_SYNC","s":4,"op":0,"d":{"guild_id":"9","channel_ids":["2"],"threads":[{"id":"30","type":11,"parent_id":"2","name":"a"}],"members":[]}}"#;
        assert!(matches!(
            parse(sync).expect("sync"),
            Event::Dispatch { dispatch: Dispatch::Event(GatewayEvent::ThreadListSync(s)), .. } if s.threads.len() == 1 && s.channel_ids.as_deref() == Some(&[Snowflake(2)][..])
        ));
        // Bozuk biçim: id yok / tip yanlış → atlanır, hata değil.
        for bad in [
            r#"{"t":"THREAD_CREATE","s":5,"op":0,"d":{"type":11}}"#,
            r#"{"t":"THREAD_UPDATE","s":6,"op":0,"d":{"id":"30","type":"x"}}"#,
            r#"{"t":"THREAD_LIST_SYNC","s":7,"op":0,"d":{"threads":[]}}"#,
            r#"{"t":"THREAD_DELETE","s":8,"op":0,"d":null}"#,
        ] {
            assert!(matches!(
                parse(bad).expect("bozuk"),
                Event::Dispatch {
                    dispatch: Dispatch::Other,
                    ..
                }
            ));
        }
    }

    #[test]
    fn ready_carries_threads() {
        let ready = r#"{"t":"READY","s":1,"op":0,"d":{"v":9,"user":{"id":"1","username":"a","global_name":null},"session_id":"s","resume_gateway_url":"wss://x","guilds":[{"id":"9","channels":[],"threads":[{"id":"30","type":12,"parent_id":"2","name":"x"},{"id":"31","type":15}]}]}}"#;
        match parse(ready).expect("ready") {
            Event::Dispatch {
                dispatch: Dispatch::Ready(r),
                ..
            } => {
                assert_eq!(r.guilds[0].threads.len(), 2);
                assert_eq!(r.guilds[0].threads[0].kind, 12);
            }
            other => panic!("beklenmeyen: {other:?}"),
        }
    }

    #[test]
    fn parses_reactions_and_typing() {
        let add = r#"{"t":"MESSAGE_REACTION_ADD","s":8,"op":0,"d":{"user_id":"1","channel_id":"2","message_id":"3","emoji":{"id":null,"name":"👍"}}}"#;
        assert!(matches!(
            parse(add).expect("ekle"),
            Event::Dispatch { dispatch: Dispatch::Event(GatewayEvent::ReactionAdd(r)), .. } if r.message_id == Snowflake(3)
        ));
        let typing = r#"{"t":"TYPING_START","s":9,"op":0,"d":{"channel_id":"2","user_id":"1","timestamp":1767268800}}"#;
        assert!(matches!(
            parse(typing).expect("yazıyor"),
            Event::Dispatch {
                dispatch: Dispatch::Event(GatewayEvent::TypingStart(_)),
                ..
            }
        ));
    }

    #[test]
    fn parses_member_list_update_and_survives_garbage() {
        let ok = r#"{"t":"GUILD_MEMBER_LIST_UPDATE","s":11,"op":0,"d":{"id":"x","guild_id":"1","member_count":1,"online_count":0,"groups":[{"id":"offline","count":1}],"ops":[{"op":"DELETE","index":0}]}}"#;
        assert!(matches!(
            parse(ok).expect("liste"),
            Event::Dispatch { dispatch: Dispatch::Event(GatewayEvent::MemberListUpdate(u)), .. } if u.guild_id == Snowflake(1)
        ));
        let bad = r#"{"t":"GUILD_MEMBER_LIST_UPDATE","s":12,"op":0,"d":{"id":"x","ops":[{"op":"DELETE"}]}}"#;
        assert!(matches!(
            parse(bad).expect("bozuk olay hata değil"),
            Event::Dispatch {
                dispatch: Dispatch::Other,
                ..
            }
        ));
    }

    #[test]
    fn subscription_payloads_match_observed() {
        assert_eq!(
            subscribe_guild(Snowflake(9)),
            r#"{"op":37,"d":{"subscriptions":{"9":{"typing":true,"threads":true,"activities":true,"members":[],"member_updates":false,"channels":{},"thread_member_lists":[]}}}}"#
        );
        assert_eq!(
            subscribe_channels(Snowflake(9), Snowflake(5), &[(0, 99)]),
            r#"{"op":37,"d":{"subscriptions":{"9":{"channels":{"5":[[0,99]]}}}}}"#
        );
        assert_eq!(
            subscribe_channels(Snowflake(9), Snowflake(5), &[(0, 99), (100, 199)]),
            r#"{"op":37,"d":{"subscriptions":{"9":{"channels":{"5":[[0,99],[100,199]]}}}}}"#
        );
        assert_eq!(
            request_members(Snowflake(9), &[Snowflake(1), Snowflake(2)]),
            r#"{"op":8,"d":{"guild_id":["9"],"user_ids":["1","2"],"presences":false}}"#
        );
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
            heartbeat(Some(7), false),
            r#"{"op":40,"d":{"seq":7,"qos":{"active":false,"ver":31,"reasons":[]}}}"#
        );
        assert_eq!(
            heartbeat(None, false),
            r#"{"op":40,"d":{"seq":null,"qos":{"active":false,"ver":31,"reasons":[]}}}"#
        );
        assert_eq!(
            heartbeat(Some(1), true),
            r#"{"op":40,"d":{"seq":1,"qos":{"active":true,"ver":31,"reasons":["foregrounded"]}}}"#
        );
    }

    #[test]
    fn post_identify_frames_match_observed() {
        assert_eq!(
            initial_voice_state(),
            r#"{"op":4,"d":{"guild_id":null,"channel_id":null,"self_mute":true,"self_deaf":false,"self_video":false,"flags":2}}"#
        );
        let p = props();
        let v: Value = serde_json::from_str(&client_init(&p)).expect("json");
        assert_eq!(v["op"], 41);
        assert_eq!(
            v["d"]["initialization_timestamp"],
            p.initialization_timestamp_ms()
        );
        assert_eq!(v["d"]["session_id"], p.client_heartbeat_session_id());
        assert_eq!(v["d"]["client_launch_id"], p.client_launch_id());
        let text = client_init(&p);
        let pos: Vec<usize> = ["initialization_timestamp", "session_id", "client_launch_id"]
            .iter()
            .map(|k| text.find(k).expect("anahtar"))
            .collect();
        assert!(pos.windows(2).all(|w| w[0] < w[1]), "{text}");
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

    #[test]
    fn voice_events_and_op4() {
        let st = r#"{"t":"VOICE_STATE_UPDATE","s":3,"op":0,"d":{"guild_id":"1","channel_id":"2","user_id":"3","session_id":"abc","self_mute":true,"self_deaf":false,"mute":false,"deaf":false,"member":{}}}"#;
        let Event::Dispatch {
            dispatch: Dispatch::Event(GatewayEvent::VoiceStateUpdate(vs)),
            ..
        } = parse(st).expect("ses durumu")
        else {
            panic!("VOICE_STATE_UPDATE bekleniyordu");
        };
        assert_eq!(
            (vs.channel_id, vs.session_id.as_str(), vs.self_mute),
            (Some(Snowflake(2)), "abc", true)
        );
        let sv = r#"{"t":"VOICE_SERVER_UPDATE","s":4,"op":0,"d":{"token":"gizli","guild_id":null,"channel_id":"9","endpoint":"c-ist01.discord.media:443"}}"#;
        let Event::Dispatch {
            dispatch: Dispatch::Event(GatewayEvent::VoiceServerUpdate(server)),
            ..
        } = parse(sv).expect("ses sunucusu")
        else {
            panic!("VOICE_SERVER_UPDATE bekleniyordu");
        };
        assert_eq!(
            server.endpoint.as_deref(),
            Some("c-ist01.discord.media:443")
        );
        assert!(
            !format!("{server:?}").contains("gizli"),
            "token Debug'da görünmez"
        );

        let v: serde_json::Value = serde_json::from_str(&voice_state_update(
            Some(Snowflake(1)),
            Some(Snowflake(2)),
            false,
            true,
        ))
        .expect("json");
        assert_eq!(
            v,
            serde_json::json!({"op": 4, "d": {"guild_id": "1", "channel_id": "2",
                "self_mute": false, "self_deaf": true, "self_video": false, "flags": 2}})
        );
        let leave: serde_json::Value =
            serde_json::from_str(&voice_state_update(None, None, false, false)).expect("json");
        assert_eq!(leave["d"]["channel_id"], serde_json::Value::Null);
    }
}
