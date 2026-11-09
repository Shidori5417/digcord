//! Gateway bağlantı döngüsü: bağlan → Hello → Identify/Resume → olaylar; koparsa
//! close kodu tablosuna göre resume / yeniden identify / dur.

use std::time::Duration;

use digcord_model::{ClientProperties, Ready, Token};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tokio::time::{Instant, sleep, sleep_until, timeout};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, header};
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::protocol::{
    self, Backoff, Beat, CloseAction, Dispatch, Event, GATEWAY_QUERY, GATEWAY_URL, Heartbeat,
};
use crate::zlib::{ZlibError, ZlibStream};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const HELLO_TIMEOUT: Duration = Duration::from_secs(20);

pub struct Config {
    pub token: Token,
    pub properties: ClientProperties,
    /// İlk bağlantı adresi; normalde [`GATEWAY_URL`]. Testler yerel sunucu verir.
    pub gateway_url: String,
}

impl Config {
    pub fn new(token: Token, properties: ClientProperties) -> Self {
        Self {
            token,
            properties,
            gateway_url: GATEWAY_URL.to_owned(),
        }
    }
}

/// Gateway'den uygulamaya giden durum bildirimleri.
#[derive(Debug)]
pub enum Update {
    Connecting,
    Ready(Box<Ready>),
    Resumed,
    /// Bağlantı koptu; `after` sonra yeniden denenecek.
    Reconnecting {
        after: Duration,
        reason: String,
    },
    /// Token geçersiz (4004). Döngü durdu.
    InvalidToken,
    /// Yeniden denenmeyecek hata. Döngü durdu.
    Fatal(String),
}

#[derive(Debug, thiserror::Error)]
enum ConnError {
    #[error("WebSocket: {0}")]
    Ws(#[from] tungstenite::Error),
    #[error("zlib: {0}")]
    Zlib(#[from] ZlibError),
    #[error("protokol: {0}")]
    Protocol(#[from] protocol::ProtocolError),
    #[error("sıkıştırılmış mesaj UTF-8 değil")]
    Utf8,
    #[error("zaman aşımı: {0}")]
    Timeout(&'static str),
    #[error("Hello yerine başka mesaj geldi")]
    NoHello,
    #[error("bağlantı close çerçevesi olmadan kapandı")]
    Eof,
    #[error("sunucu kapattı ({0}): {1}")]
    Closed(u16, String),
}

/// Bir bağlantının nasıl bittiği.
enum Exit {
    Shutdown,
    Closed { action: CloseAction, reason: String },
}

struct Session {
    id: String,
    resume_url: String,
}

/// Oturum durumu bağlantılar arasında korunur.
#[derive(Default)]
struct State {
    session: Option<Session>,
    seq: Option<u64>,
}

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// `shutdown` `true` olana veya kurtarılamaz bir hata olana kadar bağlı kalır.
pub async fn run(
    config: Config,
    updates: mpsc::Sender<Update>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut state = State::default();
    let mut backoff = Backoff::default();
    loop {
        let _ = updates.send(Update::Connecting).await;
        let exit = tokio::select! {
            exit = connect_once(&config, &mut state, &mut backoff, &updates) => exit,
            _ = shutdown.wait_for(|stop| *stop) => Exit::Shutdown,
        };
        let reason = match exit {
            Exit::Shutdown => return,
            Exit::Closed { action, reason } => match action {
                CloseAction::Resume => reason,
                CloseAction::Reidentify => {
                    state = State::default();
                    reason
                }
                CloseAction::InvalidToken => {
                    tracing::warn!("gateway: token geçersiz (4004)");
                    let _ = updates.send(Update::InvalidToken).await;
                    return;
                }
                CloseAction::Fatal => {
                    tracing::error!(%reason, "gateway: kurtarılamaz hata");
                    let _ = updates.send(Update::Fatal(reason)).await;
                    return;
                }
            },
        };
        let after = backoff.next_delay();
        tracing::info!(%reason, ?after, resume = state.session.is_some(), "gateway: yeniden bağlanılacak");
        let _ = updates.send(Update::Reconnecting { after, reason }).await;
        tokio::select! {
            _ = sleep(after) => {}
            _ = shutdown.wait_for(|stop| *stop) => return,
        }
    }
}

async fn connect_once(
    config: &Config,
    state: &mut State,
    backoff: &mut Backoff,
    updates: &mpsc::Sender<Update>,
) -> Exit {
    match session_loop(config, state, backoff, updates).await {
        Ok(exit) => exit,
        Err(e) => {
            let code = match &e {
                ConnError::Closed(code, _) => Some(*code),
                _ => None,
            };
            Exit::Closed {
                action: protocol::classify_close(code),
                reason: e.to_string(),
            }
        }
    }
}

async fn session_loop(
    config: &Config,
    state: &mut State,
    backoff: &mut Backoff,
    updates: &mpsc::Sender<Update>,
) -> Result<Exit, ConnError> {
    let url = match &state.session {
        Some(s) => format!("{}{GATEWAY_QUERY}", s.resume_url.trim_end_matches('/')),
        None => config.gateway_url.clone(),
    };
    let mut ws = connect(&url, &config.properties).await?;
    let mut zlib = ZlibStream::new();

    let hello = timeout(HELLO_TIMEOUT, next_event(&mut ws, &mut zlib))
        .await
        .map_err(|_| ConnError::Timeout("Hello"))??;
    let Some(Event::Hello { heartbeat_interval }) = hello else {
        return Err(ConnError::NoHello);
    };
    let mut hb = Heartbeat::new(heartbeat_interval);

    let first = match (&state.session, state.seq) {
        (Some(s), seq) => {
            tracing::info!("gateway: resume gönderiliyor");
            protocol::resume(&config.token, &s.id, seq)
        }
        (None, _) => {
            tracing::info!("gateway: identify gönderiliyor");
            protocol::identify(&config.token, &config.properties)
        }
    };
    ws.send(Message::text(first)).await?;

    let mut next_beat = Instant::now() + hb.first_delay();
    loop {
        let event = tokio::select! {
            _ = sleep_until(next_beat) => {
                next_beat += hb.interval();
                match hb.on_tick() {
                    Beat::Send => ws.send(Message::text(protocol::heartbeat(state.seq))).await?,
                    Beat::Zombie => {
                        return Ok(Exit::Closed { action: CloseAction::Resume, reason: "heartbeat ACK gelmedi (zombi bağlantı)".into() });
                    }
                }
                continue;
            }
            event = next_event(&mut ws, &mut zlib) => event,
        };
        let Some(event) = event? else { continue };
        match event {
            Event::Dispatch { seq, dispatch } => {
                if seq.is_some() {
                    state.seq = seq;
                }
                match dispatch {
                    Dispatch::Ready(ready) => {
                        tracing::info!(guilds = ready.guilds.len(), "gateway: READY");
                        state.session = Some(Session {
                            id: ready.session_id.clone(),
                            resume_url: ready.resume_gateway_url.clone(),
                        });
                        backoff.reset();
                        // Web istemcisi READY'den hemen sonra bir op 40 gönderiyor (PROTOCOL_NOTES §3.1).
                        ws.send(Message::text(protocol::heartbeat(state.seq)))
                            .await?;
                        hb.on_extra_sent();
                        let _ = updates.send(Update::Ready(ready)).await;
                    }
                    Dispatch::Resumed => {
                        tracing::info!("gateway: RESUMED");
                        backoff.reset();
                        let _ = updates.send(Update::Resumed).await;
                    }
                    Dispatch::Other => {}
                }
            }
            Event::HeartbeatRequest => {
                ws.send(Message::text(protocol::heartbeat(state.seq)))
                    .await?;
                hb.on_extra_sent();
            }
            Event::HeartbeatAck => hb.on_ack(),
            Event::Reconnect => {
                return Ok(Exit::Closed {
                    action: CloseAction::Resume,
                    reason: "sunucu yeniden bağlanma istedi (op 7)".into(),
                });
            }
            Event::InvalidSession { resumable } => {
                let action = if resumable {
                    CloseAction::Resume
                } else {
                    CloseAction::Reidentify
                };
                return Ok(Exit::Closed {
                    action,
                    reason: "oturum geçersiz (op 9)".into(),
                });
            }
            Event::Hello { .. } | Event::Unknown { .. } => {}
        }
    }
}

/// Bir sonraki tam gateway mesajı. `Ok(None)`: mesaj tamamlanmadı veya kontrol çerçevesi.
/// Close çerçevesi kodu ve nedeniyle `Err(ConnError::Closed)` olarak döner.
async fn next_event(ws: &mut Ws, zlib: &mut ZlibStream) -> Result<Option<Event>, ConnError> {
    let Some(msg) = ws.next().await else {
        return Err(ConnError::Eof);
    };
    match msg? {
        Message::Binary(bytes) => match zlib.push(&bytes)? {
            Some(text) => {
                let text = std::str::from_utf8(text).map_err(|_| ConnError::Utf8)?;
                Ok(Some(protocol::parse(text)?))
            }
            None => Ok(None),
        },
        Message::Text(text) => Ok(Some(protocol::parse(text.as_str())?)),
        Message::Close(Some(f)) => Err(ConnError::Closed(u16::from(f.code), f.reason.to_string())),
        Message::Close(None) => Err(ConnError::Eof),
        _ => Ok(None),
    }
}

async fn connect(url: &str, props: &ClientProperties) -> Result<Ws, ConnError> {
    let mut req = url.into_client_request()?;
    let h = req.headers_mut();
    // Resmi web istemcisinin el sıkışma başlıkları (PROTOCOL_NOTES §3.1).
    h.insert(
        header::ORIGIN,
        HeaderValue::from_static("https://discord.com"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    h.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    h.insert(
        header::ACCEPT_ENCODING,
        HeaderValue::from_static("gzip, deflate, br, zstd"),
    );
    if let Ok(v) = HeaderValue::from_str(&props.accept_language()) {
        h.insert(header::ACCEPT_LANGUAGE, v);
    }
    if let Ok(v) = HeaderValue::from_str(props.user_agent()) {
        h.insert(header::USER_AGENT, v);
    }
    let (ws, _) = timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(req))
        .await
        .map_err(|_| ConnError::Timeout("bağlantı"))??;
    Ok(ws)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression, FlushCompress};
    use serde_json::Value;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::protocol::CloseFrame;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

    /// Sahte gateway sunucusunun tek bağlantısı: zlib-stream ile gönderir.
    struct Peer {
        ws: WebSocketStream<TcpStream>,
        z: Compress,
    }

    impl Peer {
        async fn accept(listener: &TcpListener) -> Self {
            let (tcp, _) = listener.accept().await.expect("accept");
            let ws = tokio_tungstenite::accept_async(tcp)
                .await
                .expect("el sıkışma");
            Self {
                ws,
                z: Compress::new(Compression::default(), true),
            }
        }

        async fn send(&mut self, json: &str) {
            let mut out = Vec::with_capacity(json.len() + 1024);
            self.z
                .compress_vec(json.as_bytes(), &mut out, FlushCompress::Sync)
                .expect("sıkıştırma");
            self.ws.send(Message::binary(out)).await.expect("gönder");
        }

        async fn recv(&mut self) -> Value {
            loop {
                match self.ws.next().await.expect("mesaj").expect("ws") {
                    Message::Text(t) => return serde_json::from_str(t.as_str()).expect("json"),
                    _ => continue,
                }
            }
        }

        async fn close(mut self, code: u16) {
            let frame = CloseFrame {
                code: CloseCode::from(code),
                reason: "test".into(),
            };
            let _ = self.ws.close(Some(frame)).await;
        }
    }

    fn ready_json(resume_url: &str) -> String {
        let mut ready: Value = serde_json::from_str(include_str!(
            "../../digcord-model/tests/fixtures/ready.json"
        ))
        .expect("fixture");
        ready["resume_gateway_url"] = Value::from(resume_url);
        serde_json::json!({"t":"READY","s":1,"op":0,"d":ready}).to_string()
    }

    const HELLO: &str = r#"{"t":null,"s":null,"op":10,"d":{"heartbeat_interval":41250}}"#;

    async fn expect_update(
        rx: &mut mpsc::Receiver<Update>,
        pred: impl Fn(&Update) -> bool,
    ) -> Update {
        loop {
            let u = timeout(Duration::from_secs(600), rx.recv())
                .await
                .expect("güncelleme zaman aşımı")
                .expect("kanal");
            if pred(&u) {
                return u;
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn identify_ready_resume_then_invalid_token() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!("ws://{}", listener.local_addr().expect("adres"));
        let token = Token::new("sahte-token").expect("token");
        let mut config = Config::new(token, ClientProperties::chrome_linux("tr-TR", 155, 1));
        config.gateway_url = format!("{url}/?encoding=json&v=9&compress=zlib-stream");
        let (tx, mut rx) = mpsc::channel(16);
        let (_stop_tx, stop_rx) = watch::channel(false);
        let client = tokio::spawn(run(config, tx, stop_rx));

        // 1. bağlantı: Hello → IDENTIFY → READY → READY sonrası op 40 → 4000 ile kapat.
        let mut peer = Peer::accept(&listener).await;
        peer.send(HELLO).await;
        let identify = peer.recv().await;
        assert_eq!(identify["op"], 2);
        assert_eq!(identify["d"]["token"], "sahte-token");
        peer.send(&ready_json(&url)).await;
        let beat = peer.recv().await;
        assert_eq!(beat["op"], 40);
        assert_eq!(beat["d"]["seq"], 1);
        assert!(
            matches!(expect_update(&mut rx, |u| matches!(u, Update::Ready(_))).await, Update::Ready(r) if r.guilds.len() == 3)
        );
        peer.send(r#"{"t":"MESSAGE_CREATE","s":2,"op":0,"d":{}}"#)
            .await;
        peer.close(4000).await;

        // 2. bağlantı resume_gateway_url'ye: RESUME (op 6) son seq ile.
        expect_update(&mut rx, |u| matches!(u, Update::Reconnecting { .. })).await;
        let mut peer = Peer::accept(&listener).await;
        peer.send(HELLO).await;
        let resume = peer.recv().await;
        assert_eq!(resume["op"], 6);
        assert_eq!(
            resume["d"]["session_id"],
            "0123456789abcdef0123456789abcdef"
        );
        assert_eq!(resume["d"]["seq"], 2);
        peer.send(r#"{"t":"RESUMED","s":3,"op":0,"d":{}}"#).await;
        expect_update(&mut rx, |u| matches!(u, Update::Resumed)).await;

        // 4004 → InvalidToken ve döngü biter.
        peer.close(4004).await;
        expect_update(&mut rx, |u| matches!(u, Update::InvalidToken)).await;
        timeout(Duration::from_secs(600), client)
            .await
            .expect("döngü bitmeli")
            .expect("panik yok");
    }

    #[tokio::test(start_paused = true)]
    async fn missing_ack_is_zombie_and_reidentify_after_4009() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!("ws://{}", listener.local_addr().expect("adres"));
        let mut config = Config::new(
            Token::new("sahte-token").expect("token"),
            ClientProperties::chrome_linux("tr-TR", 155, 1),
        );
        config.gateway_url = url.clone();
        let (tx, mut rx) = mpsc::channel(16);
        let (stop_tx, stop_rx) = watch::channel(false);
        let client = tokio::spawn(run(config, tx, stop_rx));

        let mut peer = Peer::accept(&listener).await;
        peer.send(HELLO).await;
        assert_eq!(peer.recv().await["op"], 2);
        peer.send(&ready_json(&url)).await;
        assert_eq!(peer.recv().await["op"], 40);
        // ACK hiç gönderilmez → ilk zamanlı vuruşta zombi.
        let u = expect_update(&mut rx, |u| matches!(u, Update::Reconnecting { .. })).await;
        assert!(matches!(u, Update::Reconnecting { reason, .. } if reason.contains("zombi")));
        drop(peer);

        // Resume denemesi 4009 ile reddedilir → oturum atılır, yeni IDENTIFY.
        let mut peer = Peer::accept(&listener).await;
        peer.send(HELLO).await;
        assert_eq!(peer.recv().await["op"], 6);
        peer.close(4009).await;
        let mut peer = Peer::accept(&listener).await;
        peer.send(HELLO).await;
        assert_eq!(peer.recv().await["op"], 2);

        stop_tx.send(true).expect("durdur");
        timeout(Duration::from_secs(600), client)
            .await
            .expect("kapanmalı")
            .expect("panik yok");
    }
}
