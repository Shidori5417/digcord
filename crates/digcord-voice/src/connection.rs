//! Ses bağlantısı: ses gateway'i (WebSocket) + UDP. Bağlan → Hello → Identify/Resume →
//! Ready → IP keşfi → Select Protocol → Session Description → DAVE → ses.
//!
//! Koparsa kapanış koduna göre resume (DAVE ve taşıma durumu korunur), yeniden katılma
//! isteği ya da dur. Ses verisi Opus kareleri olarak girer/çıkar; çözme/kodlama dışarıda.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::{mpsc, watch};
use tokio::time::{Instant, interval_at, timeout};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, header};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{self, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::dave::manager::{self, Dave, Outgoing};
use crate::protocol::{self, Event as Wire, SessionInfo, close};
use crate::transport::{Mode, Transport};
use crate::{discovery, rtp};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const HELLO_TIMEOUT: Duration = Duration::from_secs(15);
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(2);
const DISCOVERY_TRIES: u32 = 3;
/// NAT eşlemesi açık kalsın diye UDP canlı tutma aralığı (discord.js davranışı; belgelenmemiş).
const UDP_KEEPALIVE: Duration = Duration::from_secs(5);
/// Bu kadar heartbeat ACK'i kaçarsa bağlantı ölü sayılır.
const MAX_MISSED_ACKS: u32 = 3;
const MAX_RESUMES: u32 = 5;
/// 20 ms'lik 48 kHz Opus karesinin RTP zaman damgası artışı.
const FRAME_SAMPLES: u32 = 960;
const UDP_BUFFER: usize = 2048;

pub struct Config {
    pub info: SessionInfo,
    pub channel_id: u64,
    pub user_agent: String,
    /// Testler için `ws://` adresi; normalde `info.url()`.
    pub url_override: Option<String>,
}

/// Bağlantıdan uygulamaya.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Ses gönderilip alınabilir.
    Connected {
        dave_version: u16,
    },
    Speaking {
        user_id: u64,
        speaking: bool,
    },
    UserLeft(u64),
    /// Bir kullanıcıdan çözülmüş Opus karesi.
    Audio {
        user_id: u64,
        opus: Vec<u8>,
    },
    /// Bağlantı bitti. `rejoin`: ana gateway'den yeniden katılma (op 4) denenebilir.
    Closed {
        reason: String,
        rejoin: bool,
    },
}

/// Uygulamadan bağlantıya.
#[derive(Debug)]
pub enum Command {
    /// Gönderilecek Opus karesi (20 ms).
    Opus(Vec<u8>),
    /// Konuşma bitti (öncesinde sessizlik kareleri gönderilmeli).
    StopSpeaking,
}

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Debug, thiserror::Error)]
enum ConnError {
    #[error("WebSocket: {0}")]
    Ws(#[from] tungstenite::Error),
    #[error("kapandı ({0}): {1}")]
    Closed(u16, String),
    #[error("bağlantı kapandı")]
    Eof,
    #[error("zaman aşımı: {0}")]
    Timeout(&'static str),
    #[error("protokol: {0}")]
    Protocol(String),
    #[error("UDP: {0}")]
    Io(#[from] std::io::Error),
    #[error("heartbeat yanıtı yok")]
    Zombie,
}

enum Exit {
    Stop,
}

/// Bağlantılar arası korunan durum (resume için).
struct State {
    seq_ack: Option<u64>,
    /// Identify tamamlandı (Ready geldi): kopunca resume denenir.
    ready: bool,
    ssrc: u32,
    udp: Option<Arc<UdpSocket>>,
    transport: Option<Transport>,
    dave: Dave,
    users: HashMap<u32, u64>,
    speaking: bool,
    rtp_sequence: u16,
    rtp_timestamp: u32,
    keepalive: u32,
}

/// Bağlantı döngüsü; `shutdown` ya da `commands` kapanınca döner.
pub async fn run(
    config: Config,
    events: mpsc::Sender<Event>,
    mut commands: mpsc::Receiver<Command>,
    mut shutdown: watch::Receiver<bool>,
) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let dave = match Dave::new(config.info.user_id, config.channel_id) {
        Ok(d) => d,
        Err(e) => {
            let _ = events
                .send(Event::Closed {
                    reason: format!("DAVE başlatılamadı: {e}"),
                    rejoin: false,
                })
                .await;
            return;
        }
    };
    let mut st = State {
        seq_ack: None,
        ready: false,
        ssrc: 0,
        udp: None,
        transport: None,
        dave,
        users: HashMap::new(),
        speaking: false,
        rtp_sequence: fastrand::u16(..),
        rtp_timestamp: fastrand::u32(..),
        keepalive: 0,
    };
    let mut resumes = 0;
    loop {
        let result = connect_once(&config, &mut st, &events, &mut commands, &mut shutdown).await;
        let err = match result {
            Ok(Exit::Stop) => return,
            Err(e) => e,
        };
        let (resumable, rejoin) = match &err {
            ConnError::Closed(code, _) if *code >= 4000 => (
                st.ready && close::resumable(*code),
                matches!(*code, close::SESSION_INVALID | close::SESSION_TIMEOUT),
            ),
            _ => (st.ready, false),
        };
        tracing::info!(resumable, "ses bağlantısı koptu: {err}");
        if !resumable || resumes >= MAX_RESUMES {
            let _ = events
                .send(Event::Closed {
                    reason: err.to_string(),
                    rejoin,
                })
                .await;
            return;
        }
        resumes += 1;
        tokio::time::sleep(Duration::from_millis(500 * u64::from(resumes))).await;
    }
}

async fn connect(url: &str, user_agent: &str) -> Result<Ws, ConnError> {
    let mut req = url.into_client_request()?;
    let h = req.headers_mut();
    h.insert(
        header::ORIGIN,
        HeaderValue::from_static("https://discord.com"),
    );
    if let Ok(v) = HeaderValue::from_str(user_agent) {
        h.insert(header::USER_AGENT, v);
    }
    let (ws, _) = timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(req))
        .await
        .map_err(|_| ConnError::Timeout("bağlantı"))??;
    Ok(ws)
}

async fn send_all(ws: &mut Ws, out: Vec<Outgoing>) -> Result<(), ConnError> {
    for o in out {
        match o {
            Outgoing::Json(text) => ws.send(Message::text(text)).await?,
            Outgoing::Binary(bytes) => ws.send(Message::binary(bytes)).await?,
        }
    }
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

async fn connect_once(
    config: &Config,
    st: &mut State,
    events: &mpsc::Sender<Event>,
    commands: &mut mpsc::Receiver<Command>,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<Exit, ConnError> {
    let url = config
        .url_override
        .clone()
        .unwrap_or_else(|| config.info.url());
    let mut ws = connect(&url, &config.user_agent).await?;

    // Hello.
    let interval = loop {
        let msg = timeout(HELLO_TIMEOUT, ws.next())
            .await
            .map_err(|_| ConnError::Timeout("Hello"))?
            .ok_or(ConnError::Eof)??;
        match msg {
            Message::Text(text) => {
                let (_, ev) = protocol::parse(text.as_str())
                    .map_err(|e| ConnError::Protocol(e.to_string()))?;
                if let Wire::Hello {
                    heartbeat_interval_ms,
                } = ev
                {
                    break Duration::from_millis(heartbeat_interval_ms.max(1000.0) as u64);
                }
            }
            Message::Close(f) => return Err(closed(f)),
            _ => {}
        }
    };
    let hello = if st.ready {
        protocol::resume(&config.info, st.seq_ack)
    } else {
        protocol::identify(&config.info)
    };
    ws.send(Message::text(hello)).await?;

    let start = Instant::now();
    let mut heartbeat = interval_at(start + interval, interval);
    let mut keepalive = interval_at(start + UDP_KEEPALIVE, UDP_KEEPALIVE);
    let mut missed = 0u32;
    let mut udp_buf = vec![0u8; UDP_BUFFER];
    let mut frame = Vec::new();
    let mut packet = Vec::new();

    loop {
        let udp = st.udp.clone();
        tokio::select! {
            msg = ws.next() => {
                let msg = msg.ok_or(ConnError::Eof)??;
                match msg {
                    Message::Text(text) => {
                        let (seq, ev) = protocol::parse(text.as_str())
                            .map_err(|e| ConnError::Protocol(e.to_string()))?;
                        if seq.is_some() {
                            st.seq_ack = seq;
                        }
                        if matches!(ev, Wire::HeartbeatAck) {
                            missed = 0;
                        }
                        on_wire(st, events, &mut ws, ev).await?;
                    }
                    Message::Binary(bytes) => {
                        if let Some((seq, op, payload)) = manager::split_server_binary(&bytes) {
                            st.seq_ack = Some(u64::from(seq));
                            let out = st.dave.binary(op, payload, std::time::Instant::now());
                            send_all(&mut ws, out).await?;
                        }
                    }
                    Message::Close(f) => return Err(closed(f)),
                    _ => {}
                }
            }
            _ = heartbeat.tick() => {
                missed += 1;
                if missed > MAX_MISSED_ACKS {
                    return Err(ConnError::Zombie);
                }
                ws.send(Message::text(protocol::heartbeat(now_ms(), st.seq_ack))).await?;
            }
            _ = keepalive.tick(), if udp.is_some() => {
                if let Some(u) = &udp {
                    let mut ka = [0u8; 8];
                    ka[..4].copy_from_slice(&st.keepalive.to_le_bytes());
                    st.keepalive = st.keepalive.wrapping_add(1);
                    let _ = u.send(&ka).await;
                }
            }
            n = async { match &udp { Some(u) => u.recv(&mut udp_buf).await, None => std::future::pending().await } } => {
                let n = n?;
                on_udp(st, events, &udp_buf[..n]);
            }
            cmd = commands.recv() => match cmd {
                None => return Ok(close_normally(ws).await),
                Some(Command::Opus(opus)) => {
                    send_opus(st, &mut ws, &opus, &mut frame, &mut packet).await?;
                }
                Some(Command::StopSpeaking) => {
                    if st.speaking {
                        st.speaking = false;
                        ws.send(Message::text(protocol::speaking(st.ssrc, false))).await?;
                    }
                }
            },
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    return Ok(close_normally(ws).await);
                }
            }
        }
    }
}

async fn close_normally(mut ws: Ws) -> Exit {
    let _ = ws
        .close(Some(CloseFrame {
            code: CloseCode::Normal,
            reason: "".into(),
        }))
        .await;
    Exit::Stop
}

fn closed(f: Option<CloseFrame>) -> ConnError {
    match f {
        Some(f) => ConnError::Closed(u16::from(f.code), f.reason.to_string()),
        None => ConnError::Eof,
    }
}

async fn on_wire(
    st: &mut State,
    events: &mpsc::Sender<Event>,
    ws: &mut Ws,
    ev: Wire,
) -> Result<(), ConnError> {
    let now = std::time::Instant::now();
    match ev {
        Wire::Ready {
            ssrc,
            ip,
            port,
            modes,
        } => {
            st.ready = true;
            st.ssrc = ssrc;
            let mode = Mode::choose(&modes)
                .ok_or_else(|| ConnError::Protocol("desteklenen şifreleme kipi yok".into()))?;
            let server: SocketAddr = format!("{ip}:{port}")
                .parse()
                .map_err(|_| ConnError::Protocol("geçersiz ses sunucusu adresi".into()))?;
            let udp = UdpSocket::bind("0.0.0.0:0").await?;
            udp.connect(server).await?;
            let (my_ip, my_port) = discover(&udp, ssrc).await?;
            st.udp = Some(Arc::new(udp));
            ws.send(Message::text(protocol::select_protocol(
                &my_ip,
                my_port,
                mode.name(),
            )))
            .await?;
        }
        Wire::SessionDescription {
            mode,
            secret_key,
            dave_protocol_version,
        } => {
            let mode = [Mode::Aes256Gcm, Mode::XChaCha20Poly1305]
                .into_iter()
                .find(|m| m.name() == mode)
                .ok_or_else(|| ConnError::Protocol("bilinmeyen şifreleme kipi".into()))?;
            st.transport = Some(
                Transport::new(mode, &secret_key)
                    .map_err(|e| ConnError::Protocol(e.to_string()))?,
            );
            let out = st.dave.session_description(dave_protocol_version);
            send_all(ws, out).await?;
            let _ = events
                .send(Event::Connected {
                    dave_version: dave_protocol_version,
                })
                .await;
        }
        Wire::Speaking {
            user_id,
            ssrc,
            speaking,
        } => {
            st.users.insert(ssrc, user_id);
            let _ = events
                .send(Event::Speaking {
                    user_id,
                    speaking: speaking != 0,
                })
                .await;
        }
        Wire::ClientsConnect(ids) => st.dave.clients_connect(&ids),
        Wire::ClientDisconnect(id) => {
            st.dave.client_disconnect(id);
            st.users.retain(|_, u| *u != id);
            let _ = events.send(Event::UserLeft(id)).await;
        }
        Wire::DavePrepareTransition {
            transition_id,
            protocol_version,
        } => {
            let out = st
                .dave
                .prepare_transition(transition_id, protocol_version, now);
            send_all(ws, out).await?;
        }
        Wire::DaveExecuteTransition { transition_id } => {
            st.dave.execute_transition(transition_id, now);
        }
        Wire::DavePrepareEpoch {
            epoch,
            protocol_version,
        } => {
            let out = st.dave.prepare_epoch(epoch, protocol_version);
            send_all(ws, out).await?;
        }
        Wire::Resumed => tracing::info!("ses bağlantısı resume edildi"),
        Wire::Hello { .. } | Wire::HeartbeatAck | Wire::Other(_) => {}
    }
    Ok(())
}

/// IP keşfi: birkaç deneme, her biri kısa zaman aşımıyla.
async fn discover(udp: &UdpSocket, ssrc: u32) -> Result<(String, u16), ConnError> {
    let req = discovery::request(ssrc);
    let mut buf = [0u8; 256];
    for _ in 0..DISCOVERY_TRIES {
        udp.send(&req).await?;
        let Ok(n) = timeout(DISCOVERY_TIMEOUT, udp.recv(&mut buf)).await else {
            continue;
        };
        if let Some(addr) = discovery::parse_response(&buf[..n?], ssrc) {
            return Ok(addr);
        }
    }
    Err(ConnError::Timeout("IP keşfi"))
}

fn on_udp(st: &mut State, events: &mpsc::Sender<Event>, packet: &[u8]) {
    if rtp::is_rtcp(packet) {
        return;
    }
    let Some(transport) = &st.transport else {
        return;
    };
    let Ok(opened) = transport.open(packet) else {
        return;
    };
    if opened.header.payload_type != rtp::OPUS_PAYLOAD_TYPE {
        return;
    }
    let Some(&user_id) = st.users.get(&opened.header.ssrc) else {
        return;
    };
    let Some(opus) = st
        .dave
        .decrypt(user_id, &opened.payload, std::time::Instant::now())
    else {
        return;
    };
    // Ses gecikmeye duyarlı: kuyruk doluysa kareyi at, bekleme.
    let _ = events.try_send(Event::Audio { user_id, opus });
}

async fn send_opus(
    st: &mut State,
    ws: &mut Ws,
    opus: &[u8],
    frame: &mut Vec<u8>,
    packet: &mut Vec<u8>,
) -> Result<(), ConnError> {
    let (Some(udp), Some(transport)) = (&st.udp, &mut st.transport) else {
        return Ok(());
    };
    if !st.dave.encrypt(opus, frame) {
        return Ok(());
    }
    if !st.speaking {
        st.speaking = true;
        ws.send(Message::text(protocol::speaking(st.ssrc, true)))
            .await?;
    }
    let header = rtp::Header {
        sequence: st.rtp_sequence,
        timestamp: st.rtp_timestamp,
        ssrc: st.ssrc,
    };
    st.rtp_sequence = st.rtp_sequence.wrapping_add(1);
    st.rtp_timestamp = st.rtp_timestamp.wrapping_add(FRAME_SAMPLES);
    transport.seal(&header, frame, packet);
    if !packet.is_empty() {
        udp.send(packet).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    const KEY: [u8; 32] = [9; 32];

    /// Sahte ses sunucusu: WS + UDP. DAVE sürüm 0 (şifresiz) yolunu uçtan uca dener.
    #[tokio::test]
    async fn end_to_end_without_dave() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("tcp");
        let ws_addr = listener.local_addr().expect("adres");
        let server_udp = UdpSocket::bind("127.0.0.1:0").await.expect("udp");
        let udp_port = server_udp.local_addr().expect("adres").port();

        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.expect("kabul");
            let mut ws = tokio_tungstenite::accept_async(tcp).await.expect("ws");
            ws.send(Message::text(
                r#"{"op":8,"d":{"v":8,"heartbeat_interval":30000}}"#,
            ))
            .await
            .expect("hello");
            let identify = ws.next().await.expect("mesaj").expect("ws");
            let v: serde_json::Value =
                serde_json::from_str(identify.to_text().expect("metin")).expect("json");
            assert_eq!(v["op"], 0);
            assert_eq!(v["d"]["max_dave_protocol_version"], 1);
            ws.send(Message::text(format!(
                r#"{{"op":2,"seq":1,"d":{{"ssrc":77,"ip":"127.0.0.1","port":{udp_port},"modes":["aead_aes256_gcm_rtpsize"]}}}}"#
            )))
            .await
            .expect("ready");
            // IP keşfi.
            let mut buf = [0u8; 2048];
            let (n, client) = server_udp.recv_from(&mut buf).await.expect("keşif");
            assert_eq!(n, discovery::PACKET_LEN);
            let mut resp = [0u8; discovery::PACKET_LEN];
            resp[0..2].copy_from_slice(&[0, 2]);
            resp[2..4].copy_from_slice(&[0, 70]);
            resp[4..8].copy_from_slice(&77u32.to_be_bytes());
            resp[8..17].copy_from_slice(b"127.0.0.1");
            resp[72..74].copy_from_slice(&client.port().to_be_bytes());
            server_udp.send_to(&resp, client).await.expect("yanıt");
            let select = ws.next().await.expect("mesaj").expect("ws");
            let v: serde_json::Value =
                serde_json::from_str(select.to_text().expect("metin")).expect("json");
            assert_eq!(v["op"], 1);
            assert_eq!(v["d"]["data"]["mode"], "aead_aes256_gcm_rtpsize");
            assert_eq!(v["d"]["data"]["port"], client.port());
            let key: Vec<String> = KEY.iter().map(u8::to_string).collect();
            ws.send(Message::text(format!(
                r#"{{"op":4,"seq":2,"d":{{"mode":"aead_aes256_gcm_rtpsize","secret_key":[{}],"dave_protocol_version":0}}}}"#,
                key.join(",")
            )))
            .await
            .expect("oturum");
            ws.send(Message::text(
                r#"{"op":5,"seq":3,"d":{"user_id":"5","ssrc":99,"speaking":1}}"#,
            ))
            .await
            .expect("konuşma");

            // İstemcinin gönderdiği ses: önce Speaking, sonra UDP paketi.
            let sp = ws.next().await.expect("mesaj").expect("ws");
            assert!(sp.to_text().expect("metin").contains(r#""op":5"#));
            let (n, _) = server_udp.recv_from(&mut buf).await.expect("ses");
            let rx = Transport::new(Mode::Aes256Gcm, &KEY).expect("anahtar");
            let opened = rx.open(&buf[..n]).expect("çöz");
            assert_eq!(opened.payload, b"giden opus");
            assert_eq!(opened.header.ssrc, 77);

            // Kullanıcı 5'ten (ssrc 99) gelen ses.
            let mut tx = Transport::new(Mode::Aes256Gcm, &KEY).expect("anahtar");
            let mut out = Vec::new();
            tx.seal(
                &rtp::Header {
                    sequence: 1,
                    timestamp: 0,
                    ssrc: 99,
                },
                b"gelen opus",
                &mut out,
            );
            server_udp.send_to(&out, client).await.expect("gönder");
            // Kapanana kadar bekle.
            while let Some(Ok(m)) = ws.next().await {
                if m.is_close() {
                    break;
                }
            }
        });

        let (ev_tx, mut ev_rx) = mpsc::channel(16);
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let (stop_tx, stop_rx) = watch::channel(false);
        let client = tokio::spawn(run(
            Config {
                info: SessionInfo {
                    server_id: 1,
                    user_id: 2,
                    session_id: "s".into(),
                    token: "t".into(),
                    endpoint: "unused".into(),
                },
                channel_id: 3,
                user_agent: "test".into(),
                url_override: Some(format!("ws://{ws_addr}")),
            },
            ev_tx,
            cmd_rx,
            stop_rx,
        ));
        let next = async |rx: &mut mpsc::Receiver<Event>| {
            timeout(Duration::from_secs(5), rx.recv())
                .await
                .expect("olay zaman aşımı")
                .expect("olay")
        };
        assert_eq!(next(&mut ev_rx).await, Event::Connected { dave_version: 0 });
        assert_eq!(
            next(&mut ev_rx).await,
            Event::Speaking {
                user_id: 5,
                speaking: true
            }
        );
        cmd_tx
            .send(Command::Opus(b"giden opus".to_vec()))
            .await
            .expect("komut");
        assert_eq!(
            next(&mut ev_rx).await,
            Event::Audio {
                user_id: 5,
                opus: b"gelen opus".to_vec()
            }
        );
        stop_tx.send(true).expect("dur");
        timeout(Duration::from_secs(5), client)
            .await
            .expect("istemci kapanmadı")
            .expect("istemci");
        timeout(Duration::from_secs(5), server)
            .await
            .expect("sunucu kapanmadı")
            .expect("sunucu");
    }
}
