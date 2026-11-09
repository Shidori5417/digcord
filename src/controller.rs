//! TUI ile ağ/anahtarlık katmanları arasındaki köprü ve store'un **tek yazarı**.
//! tokio üzerinde çalışır; TUI'ye sadece [`UiEvent`] gönderir, TUI'den [`Action`] alır.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Duration;

use digcord_gateway::{Command, Config, GatewayEvent, Update};
use digcord_http::{Http, HttpError, NewMessage, Page};
use digcord_media::{Fetcher, MediaError};
use digcord_model::{
    ClientProperties, DEFAULT_BUILD_NUMBER, DEFAULT_CHROME_MAJOR, Message, MessageDelete,
    ReactionEmoji, Ready, Snowflake, Token,
};
use digcord_store::{NotifyFilter, State, VoiceCall, VoicePhase};
use digcord_tui::{Action, ImageJob, Shared, Status, UiEvent};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::config::{Config as AppConfig, NotifyOn, VoiceMode};
use crate::keystore;

/// Aynı kanala okundu isteği en fazla bu aralıkla (fazlası birleştirilir).
const ACK_EVERY: Duration = Duration::from_secs(3);

/// Kanal açılışında ve yukarı kaydırınca istenen mesaj sayıları (PROTOCOL_NOTES §4.1).
const FIRST_PAGE: u8 = 10;
const OLDER_PAGE: u8 = 20;
/// op 8 isteği başına en fazla kullanıcı.
const MEMBERS_PER_REQUEST: usize = 100;
/// Medya disk önbelleğinin üst sınırı; açılışta en eskiler silinir.
const MEDIA_CACHE_BYTES: u64 = 512 * 1024 * 1024;

struct Session {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
    commands: mpsc::Sender<Command>,
    http: Arc<Http>,
}

impl Session {
    fn start(token: Token, env: &Env, updates: mpsc::Sender<Update>) -> Result<Self, HttpError> {
        let properties = ClientProperties::chrome_linux(
            &env.locale,
            env.settings.chrome_major,
            env.settings.build_number,
        );
        let http = Arc::new(Http::new(&token, &properties, &env.timezone)?);
        let (stop, stop_rx) = watch::channel(false);
        let (commands, commands_rx) = mpsc::channel(64);
        let task = tokio::spawn(digcord_gateway::run(
            Config::new(token, properties),
            updates,
            commands_rx,
            stop_rx,
        ));
        Ok(Self {
            stop,
            task,
            commands,
            http,
        })
    }

    async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = tokio::time::timeout(Duration::from_secs(2), self.task).await;
    }
}

/// Sistemden okunan, istemci kimliğine giren değerler.
pub struct Env {
    pub locale: String,
    pub timezone: String,
    pub settings: Settings,
}

/// config.toml'dan gelen davranış ayarları.
#[derive(Debug, Clone)]
pub struct Settings {
    pub send_typing: bool,
    pub send_read_receipts: bool,
    pub notify: Option<NotifyFilter>,
    pub build_number: u32,
    pub chrome_major: u32,
    pub push_to_talk: bool,
    /// Ses çalmadan önce biriktirilen kare sayısı (20 ms).
    pub jitter_frames: usize,
}

impl Settings {
    pub fn from_config(c: &AppConfig) -> Self {
        let on = &c.notifications.on;
        Self {
            send_typing: c.behavior.send_typing,
            send_read_receipts: c.behavior.send_read_receipts,
            notify: c.notifications.enabled.then(|| NotifyFilter {
                mention: on.contains(&NotifyOn::Mention),
                dm: on.contains(&NotifyOn::Dm),
                all: on.contains(&NotifyOn::All),
            }),
            build_number: match c.client.build_number {
                0 => DEFAULT_BUILD_NUMBER,
                n => n,
            },
            chrome_major: match c.client.chrome_major {
                0 => DEFAULT_CHROME_MAJOR,
                n => n,
            },
            push_to_talk: c.voice.mode == VoiceMode::Ptt,
            jitter_frames: usize::try_from(c.voice.jitter_ms.div_ceil(20)).unwrap_or(3),
        }
    }
}

impl Env {
    pub fn detect(settings: Settings) -> Self {
        Self {
            locale: system_locale(),
            timezone: jiff::tz::TimeZone::system()
                .iana_name()
                .unwrap_or("UTC")
                .to_owned(),
            settings,
        }
    }
}

/// Bir REST geçmiş isteğinin sonucu (ayrı task'tan döner).
struct History {
    channel: Snowflake,
    before: Option<Snowflake>,
    limit: u8,
    result: Result<Vec<Message>, HttpError>,
}

/// Ağ işinin sonucunu denetleyiciye (store'un tek yazarına) geri getiren iş.
type Job = Box<dyn FnOnce(&mut Controller) + Send>;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Open {
    guild: Option<Snowflake>,
    channel: Snowflake,
}

struct Controller {
    ui: Sender<UiEvent>,
    shared: Arc<Shared>,
    env: Env,
    updates_tx: mpsc::Sender<Update>,
    history_tx: mpsc::Sender<History>,
    jobs_tx: mpsc::UnboundedSender<Job>,
    /// Aynı milisaniyede üretilen nonce'lar çakışmasın diye.
    nonce_seq: u64,
    session: Option<Session>,
    /// Girişten gelen token; READY gelince (geçerli olduğu kanıtlanınca) kaydedilir.
    unsaved: Option<Token>,
    last_online: Option<Status>,
    open: Option<Open>,
    /// Bu oturumda op 37 gönderilen guild'ler.
    subscribed: HashSet<Snowflake>,
    /// Geçmiş isteği sürmekte olan kanallar.
    loading: HashSet<Snowflake>,
    /// op 8 ile zaten istenmiş (guild, kullanıcı) çiftleri.
    requested_members: HashSet<(Snowflake, Snowflake)>,
    /// Terminal odakta mı (odak olayları desteklenmiyorsa hep `true`).
    focused: bool,
    last_ack: std::collections::HashMap<Snowflake, std::time::Instant>,
    /// Kısıtlama yüzünden ertelenmiş okundu: kanal → mesaj.
    ack_due: std::collections::HashMap<Snowflake, Snowflake>,
    /// CDN istemcisi; kurulamazsa resimler yüklenmez.
    media: Option<Arc<Fetcher>>,
    /// Resim adresinin süresi dolduğu için bir kez yeniden çekilmiş mesajlar.
    refetched: HashSet<Snowflake>,
    /// Açık ses oturumu.
    voice: Option<crate::voice::VoiceSession>,
    /// Ses oturumu kimliği beklenirken gelen sunucu bilgisi.
    voice_server: Option<digcord_model::VoiceServer>,
    /// Bu katılımda "yeniden katıl" denendi mi (sonsuz döngü olmasın).
    voice_rejoined: bool,
}

impl Controller {
    fn send(&self, ev: UiEvent) {
        let _ = self.ui.send(ev);
    }

    fn notice(&self, text: impl Into<String>) {
        self.send(UiEvent::Notice(text.into()));
    }

    /// Ağ işini ayrı task'ta çalıştırır; sonucu `done` ile denetleyicide işlenir.
    fn spawn<T: Send + 'static>(
        &self,
        work: impl std::future::Future<Output = T> + Send + 'static,
        done: impl FnOnce(&mut Controller, T) + Send + 'static,
    ) {
        let tx = self.jobs_tx.clone();
        tokio::spawn(async move {
            let out = work.await;
            let _ = tx.send(Box::new(move |c: &mut Controller| done(c, out)));
        });
    }

    fn http(&self) -> Option<Arc<Http>> {
        let http = self.session.as_ref().map(|s| s.http.clone());
        if http.is_none() {
            self.notice("Bağlı değil");
        }
        http
    }

    /// Şimdiki zamandan nonce snowflake'i (alt bitlerde sayaç).
    fn next_nonce(&mut self) -> Snowflake {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        self.nonce_seq = (self.nonce_seq + 1) & 0x3F_FFFF;
        Snowflake(Snowflake::from_timestamp_ms(ms).0 | self.nonce_seq)
    }

    fn send_message(&mut self, page: Page, content: String, reply_to: Option<Snowflake>) {
        let Some(http) = self.http() else { return };
        let nonce = self.next_nonce();
        let pending = self.shared.read_state(|st| {
            let me = st.me()?.clone();
            let reply = reply_to.and_then(|id| st.find_message(page.channel, id).cloned());
            Some(Message::pending(
                nonce,
                page.channel,
                page.guild,
                me,
                content.clone(),
                reply,
            ))
        });
        if let Some(p) = pending {
            self.write(|st| st.add_pending(p));
        }
        let key = nonce.to_string();
        self.spawn(
            async move {
                let msg = NewMessage {
                    content: &content,
                    nonce,
                    reply_to,
                    attachments: Vec::new(),
                };
                http.create_message(page, &msg).await
            },
            move |c, result| match result {
                Ok(m) => c.write(|st| st.message_create(m)),
                Err(e) => {
                    tracing::warn!("mesaj gönderilemedi: {e}");
                    c.write(|st| st.mark_failed(&key));
                    c.notice(format!("Gönderilemedi: {e}"));
                }
            },
        );
    }

    fn retry(&mut self, page: Page, message: Snowflake) {
        let found = self.shared.read_state(|st| {
            let m = st.find_message(page.channel, message)?;
            st.is_failed(message).then(|| {
                (
                    m.nonce.clone(),
                    m.content.clone(),
                    m.referenced_message.as_ref().map(|r| r.id),
                )
            })
        });
        let Some((nonce, content, reply_to)) = found else {
            return;
        };
        if let Some(n) = nonce {
            self.write(|st| st.discard_pending(&n));
        }
        self.send_message(page, content, reply_to);
    }

    fn edit_message(&mut self, page: Page, message: Snowflake, content: String) {
        let Some(http) = self.http() else { return };
        self.spawn(
            async move { http.edit_message(page, message, &content).await },
            |c, result| match result {
                Ok(m) => c.write(|st| st.message_update(m)),
                Err(e) => c.notice(format!("Düzenlenemedi: {e}")),
            },
        );
    }

    fn delete_message(&mut self, page: Page, message: Snowflake) {
        // Hiç gönderilememiş iyimser mesaj: sadece yerelden kaldır.
        let local = self.shared.read_state(|st| {
            st.is_failed(message)
                .then(|| {
                    st.find_message(page.channel, message)
                        .and_then(|m| m.nonce.clone())
                })
                .flatten()
        });
        if let Some(n) = local {
            self.write(|st| st.discard_pending(&n));
            return;
        }
        let Some(http) = self.http() else { return };
        self.spawn(
            async move { http.delete_message(page, message).await },
            move |c, result| match result {
                Ok(()) => c.write(|st| {
                    st.message_delete(&MessageDelete {
                        id: message,
                        channel_id: page.channel,
                        guild_id: page.guild,
                    })
                }),
                Err(e) => c.notice(format!("Silinemedi: {e}")),
            },
        );
    }

    fn react(&mut self, page: Page, message: Snowflake, emoji: ReactionEmoji, add: bool) {
        let Some(http) = self.http() else { return };
        self.spawn(
            async move {
                if add {
                    http.add_reaction(page, message, &emoji).await
                } else {
                    http.remove_reaction(page, message, &emoji).await
                }
            },
            // Sayaç gateway'in MESSAGE_REACTION_* yankısıyla güncellenir.
            |c, result| {
                if let Err(e) = result {
                    c.notice(format!("Tepki eklenemedi: {e}"));
                }
            },
        );
    }

    fn typing(&self, page: Page) {
        let Some(http) = self.session.as_ref().map(|s| s.http.clone()) else {
            return;
        };
        tokio::spawn(async move {
            if let Err(e) = http.typing(page).await {
                tracing::debug!("yazıyor bildirimi gönderilemedi: {e}");
            }
        });
    }

    fn upload(&mut self, page: Page, path: std::path::PathBuf, content: String, max: u64) {
        let Some(http) = self.http() else { return };
        let nonce = self.next_nonce();
        self.notice(format!("Yükleniyor: {}", path.display()));
        self.spawn(
            async move {
                let filename = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .ok_or_else(|| "geçersiz dosya yolu".to_owned())?;
                let read_path = path.clone();
                let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
                    let meta = std::fs::metadata(&read_path).map_err(|e| e.to_string())?;
                    if meta.len() > max {
                        return Err(format!("dosya çok büyük ({} MB sınırı)", max / 1024 / 1024));
                    }
                    std::fs::read(&read_path).map_err(|e| e.to_string())
                })
                .await
                .map_err(|e| e.to_string())??;
                let uploaded = http
                    .upload_attachment(page, &filename, bytes)
                    .await
                    .map_err(|e| e.to_string())?;
                let msg = NewMessage {
                    content: &content,
                    nonce,
                    reply_to: None,
                    attachments: vec![uploaded],
                };
                http.create_message(page, &msg)
                    .await
                    .map_err(|e| e.to_string())
            },
            |c, result: Result<Message, String>| match result {
                Ok(m) => {
                    c.write(|st| st.message_create(m));
                    c.notice("Dosya gönderildi");
                }
                Err(e) => c.notice(format!("Yüklenemedi: {e}")),
            },
        );
    }

    /// Resmi indirir; çözme ve kodlama engelleyici havuzda. Sonuç doğrudan TUI'ye gider.
    fn load_image(&mut self, job: ImageJob) {
        let Some(fetcher) = self.media.clone() else {
            self.send(job.failed("medya istemcisi yok".into()));
            return;
        };
        let ui = self.ui.clone();
        self.spawn(
            async move {
                match fetcher.get(job.url()).await {
                    Ok(bytes) => {
                        match tokio::task::spawn_blocking(move || job.encode(&bytes)).await {
                            Ok(ev) => {
                                let _ = ui.send(ev);
                            }
                            Err(e) => tracing::warn!("resim kodlanamadı: {e}"),
                        }
                        None
                    }
                    Err(e) => Some((job, e)),
                }
            },
            |c, failed| {
                if let Some((job, e)) = failed {
                    c.image_failed(job, e);
                }
            },
        );
    }

    /// İmzalı ek adresinin süresi dolmuşsa mesaj bir kez yeniden çekilir: gelen yeni adres
    /// TUI'de yeni bir istek doğurur.
    fn image_failed(&mut self, job: ImageJob, e: MediaError) {
        if e.maybe_expired()
            && job.url().contains("ex=")
            && let Some((channel, message)) = job.origin()
            && self.refetched.insert(message)
            && let Some(http) = self.session.as_ref().map(|s| s.http.clone())
        {
            let guild = self
                .shared
                .read_state(|st| st.channel(channel).and_then(|c| c.guild_id));
            let after = Snowflake(message.0.saturating_add(1));
            self.spawn(
                async move {
                    http.get_messages(Page { guild, channel }, Some(after), 1)
                        .await
                },
                move |c, result| match result {
                    Ok(msgs) => {
                        if let Some(m) = msgs.into_iter().find(|m| m.id == message) {
                            c.write(|st| st.message_update(m));
                        }
                    }
                    Err(e) => tracing::debug!("mesaj yenilenemedi: {e}"),
                },
            );
        }
        self.send(job.failed(e.to_string()));
    }

    fn voice_call(&self) -> Option<VoiceCall> {
        self.shared.read_state(|st| st.voice_call().cloned())
    }

    fn stop_voice(&mut self) {
        if let Some(v) = self.voice.take() {
            v.stop();
        }
        self.voice_server = None;
    }

    /// op 4 ile katılır; oturum ve sunucu bilgisi gelince bağlantı başlar.
    fn join_voice(&mut self, guild: Option<Snowflake>, channel: Snowflake) {
        if self.session.is_none() {
            self.notice("Bağlı değil");
            return;
        }
        let (self_mute, self_deaf) = self
            .voice_call()
            .map_or((false, false), |c| (c.self_mute, c.self_deaf));
        self.stop_voice();
        self.voice_rejoined = false;
        self.write(|st| {
            st.set_voice_call(Some(VoiceCall {
                guild,
                channel,
                phase: VoicePhase::Joining,
                self_mute,
                self_deaf,
            }))
        });
        self.command(Command::VoiceState {
            guild,
            channel: Some(channel),
            self_mute,
            self_deaf,
        });
    }

    fn leave_voice(&mut self) {
        let Some(call) = self.voice_call() else {
            return;
        };
        self.stop_voice();
        self.command(Command::VoiceState {
            guild: call.guild,
            channel: None,
            self_mute: call.self_mute,
            self_deaf: call.self_deaf,
        });
        self.write(|st| st.set_voice_call(None));
    }

    /// Mikrofon (`deaf: false`) ya da kulaklık susturma; sunucuya da bildirilir.
    fn toggle_voice_flag(&mut self, deaf: bool) {
        let Some(mut call) = self.voice_call() else {
            self.notice("Ses kanalında değilsin");
            return;
        };
        if deaf {
            call.self_deaf = !call.self_deaf;
        } else {
            call.self_mute = !call.self_mute;
        }
        if let Some(v) = &self.voice {
            // Kulaklık kapalıyken mikrofon da kapalı (resmi istemci davranışı).
            v.set_muted(call.self_mute || call.self_deaf);
            v.set_deafened(call.self_deaf);
        }
        self.command(Command::VoiceState {
            guild: call.guild,
            channel: Some(call.channel),
            self_mute: call.self_mute,
            self_deaf: call.self_deaf,
        });
        self.write(|st| st.set_voice_call(Some(call)));
    }

    fn on_voice_state(&mut self, vs: &digcord_model::VoiceState) {
        self.write(|st| st.voice_state_update(vs));
        let me = self.shared.read_state(|st| st.me().map(|u| u.id));
        if Some(vs.user_id) != me {
            return;
        }
        let Some(mut call) = self.voice_call() else {
            return;
        };
        if vs.guild_id != call.guild {
            return;
        }
        match vs.channel_id {
            // Başka yerden çıkarıldık ya da ayrıldık.
            None if call.phase != VoicePhase::Joining => {
                self.stop_voice();
                self.write(|st| st.set_voice_call(None));
                self.notice("Ses kanalından ayrıldın");
            }
            None => {}
            Some(ch) if ch != call.channel => {
                // Taşındık: sunucu yeni bilgi gönderecek.
                self.stop_voice();
                call.channel = ch;
                call.phase = VoicePhase::Joining;
                self.write(|st| st.set_voice_call(Some(call)));
            }
            Some(_) => self.try_start_voice(),
        }
    }

    fn on_voice_server(&mut self, server: digcord_model::VoiceServer) {
        let Some(call) = self.voice_call() else {
            return;
        };
        let ours = match call.guild {
            Some(g) => server.guild_id == Some(g),
            None => {
                server.guild_id.is_none() && server.channel_id.is_none_or(|c| c == call.channel)
            }
        };
        if !ours {
            return;
        }
        // Yeni sunucu/token: eski bağlantı geçersiz.
        if let Some(v) = self.voice.take() {
            v.stop();
        }
        if server.endpoint.is_none() {
            // Sunucu değişiyor; yenisi gelecek.
            self.write(|st| {
                if let Some(mut c) = st.voice_call().cloned() {
                    c.phase = VoicePhase::Joining;
                    st.set_voice_call(Some(c));
                }
            });
            return;
        }
        self.voice_server = Some(server);
        self.try_start_voice();
    }

    /// Oturum kimliği ve sunucu bilgisi ikisi de varsa ses bağlantısını başlatır.
    fn try_start_voice(&mut self) {
        if self.voice.is_some() {
            return;
        }
        let Some(mut call) = self.voice_call() else {
            return;
        };
        let (session_id, me) = self.shared.read_state(|st| {
            (
                st.my_voice_session(call.guild).map(str::to_owned),
                st.me().map(|u| u.id),
            )
        });
        let (Some(session_id), Some(me)) = (session_id, me) else {
            return;
        };
        let Some(server) = self.voice_server.take() else {
            return;
        };
        let Some(endpoint) = server.endpoint.clone() else {
            return;
        };
        let info = digcord_voice::protocol::SessionInfo {
            // DM aramasında kanal id'si (doğrulanmadı, PROTOCOL_NOTES §7).
            server_id: call.guild.unwrap_or(call.channel).0,
            user_id: me.0,
            session_id,
            token: server.token,
            endpoint,
        };
        let props = ClientProperties::chrome_linux(
            &self.env.locale,
            self.env.settings.chrome_major,
            self.env.settings.build_number,
        );
        let jobs = self.jobs_tx.clone();
        self.voice = Some(crate::voice::VoiceSession::start(
            info,
            call.channel.0,
            props.user_agent().to_owned(),
            crate::voice::Options {
                muted: call.self_mute || call.self_deaf,
                deafened: call.self_deaf,
                push_to_talk: self.env.settings.push_to_talk,
                jitter_frames: self.env.settings.jitter_frames,
            },
            move |ev| {
                let _ = jobs.send(Box::new(move |c: &mut Controller| c.on_voice_event(ev)));
            },
        ));
        call.phase = VoicePhase::Connecting;
        self.write(|st| st.set_voice_call(Some(call)));
    }

    fn on_voice_event(&mut self, ev: digcord_voice::connection::Event) {
        use digcord_voice::connection::Event as V;
        match ev {
            V::Connected { dave_version } => self.write(|st| {
                if let Some(mut c) = st.voice_call().cloned() {
                    c.phase = VoicePhase::Connected {
                        e2ee: dave_version > 0,
                    };
                    st.set_voice_call(Some(c));
                }
            }),
            V::Speaking { user_id, speaking } => {
                self.write(|st| st.set_speaking(Snowflake(user_id), speaking));
            }
            V::UserLeft(user) => self.write(|st| st.set_speaking(Snowflake(user), false)),
            V::Audio { .. } => {}
            V::Closed { reason, rejoin } => {
                tracing::info!(rejoin, "ses bağlantısı kapandı: {reason}");
                self.voice = None;
                match self.voice_call() {
                    Some(call) if rejoin && !self.voice_rejoined => {
                        self.voice_rejoined = true;
                        let rejoined = self.voice_rejoined;
                        self.join_voice(call.guild, call.channel);
                        self.voice_rejoined = rejoined;
                    }
                    Some(_) => {
                        self.leave_voice();
                        self.notice(format!("Ses bağlantısı kapandı: {reason}"));
                    }
                    None => {}
                }
            }
        }
    }

    /// Adresi `xdg-open` ile açar (sadece http/https; kabuk yok, tek argüman).
    fn open_external(&self, url: String) {
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            self.notice("Sadece http/https bağlantıları açılır");
            return;
        }
        self.spawn(
            async move {
                tokio::task::spawn_blocking(move || {
                    std::process::Command::new("xdg-open")
                        .arg(&url)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()
                })
                .await
            },
            |c, result| match result {
                Ok(Ok(status)) if status.success() => {}
                Ok(Ok(status)) => c.notice(format!("Açılamadı: xdg-open {status}")),
                Ok(Err(e)) => c.notice(format!("Açılamadı: {e}")),
                Err(e) => c.notice(format!("Açılamadı: {e}")),
            },
        );
    }

    fn copy(&self, text: String) {
        self.spawn(
            async move { tokio::task::spawn_blocking(move || copy_to_clipboard(&text)).await },
            |c, result| match result {
                Ok(Ok(())) => c.notice("Kopyalandı"),
                Ok(Err(e)) => c.notice(format!("Kopyalanamadı: {e}")),
                Err(e) => c.notice(format!("Kopyalanamadı: {e}")),
            },
        );
    }

    /// Store'a yazar; TUI'ye bekleyen bildirim yoksa "değişti" gönderir.
    fn write(&self, f: impl FnOnce(&mut State)) {
        {
            let mut st = self.shared.state.write().unwrap_or_else(|e| e.into_inner());
            f(&mut st);
        }
        if self.shared.mark_dirty() {
            self.send(UiEvent::StoreChanged);
        }
    }

    async fn start(&mut self, token: Token) {
        if let Some(old) = self.session.take() {
            old.stop().await;
        }
        match Session::start(token, &self.env, self.updates_tx.clone()) {
            Ok(s) => self.session = Some(s),
            Err(e) => self.send(UiEvent::Status(Status::Failed(e.to_string()))),
        }
    }

    fn command(&self, cmd: Command) {
        if let Some(s) = &self.session
            && s.commands.try_send(cmd).is_err()
        {
            tracing::warn!("gateway komut kuyruğu dolu; komut atlandı");
        }
    }

    fn fetch(&mut self, open: Open, before: Option<Snowflake>, limit: u8) {
        let Some(http) = self.session.as_ref().map(|s| s.http.clone()) else {
            return;
        };
        if !self.loading.insert(open.channel) {
            return;
        }
        let tx = self.history_tx.clone();
        tokio::spawn(async move {
            let page = Page {
                guild: open.guild,
                channel: open.channel,
            };
            let result = http.get_messages(page, before, limit).await;
            let _ = tx
                .send(History {
                    channel: open.channel,
                    before,
                    limit,
                    result,
                })
                .await;
        });
    }

    /// Kanal açılınca: guild aboneliği (bir kez) ve ilk sayfa.
    fn open_channel(&mut self, open: Open) {
        self.open = Some(open);
        if let Some(gid) = open.guild
            && self.subscribed.insert(gid)
        {
            let large = self
                .shared
                .read_state(|st| st.guilds().iter().any(|g| g.id == gid && g.large));
            self.command(Command::SubscribeGuild { guild: gid, large });
        }
        if self.shared.read_state(|st| st.is_loaded(open.channel)) {
            self.request_members(open);
        } else {
            self.fetch(open, None, FIRST_PAGE);
        }
        self.mark_seen(open.channel);
    }

    /// Açık ve görülen kanal: yerelde okundu, ayar açıksa sunucuya okundu (kısıtlamalı).
    fn mark_seen(&mut self, channel: Snowflake) {
        if !self.focused || self.open.is_none_or(|o| o.channel != channel) {
            return;
        }
        let latest = self.shared.read_state(|st| {
            let pending = st.is_unread(channel) || st.mention_count(channel) > 0;
            let last = st.channel(channel).and_then(|c| c.last_message_id);
            pending.then_some(last).flatten()
        });
        let Some(message) = latest else { return };
        self.write(|st| st.mark_read(channel, message));
        if !self.env.settings.send_read_receipts {
            return;
        }
        let now = std::time::Instant::now();
        match self.last_ack.get(&channel) {
            Some(t) if now.duration_since(*t) < ACK_EVERY => {
                if self.ack_due.insert(channel, message).is_none() {
                    let wait = ACK_EVERY - now.duration_since(*t);
                    self.spawn(tokio::time::sleep(wait), move |c, ()| c.flush_ack(channel));
                }
            }
            _ => self.send_ack(channel, message),
        }
    }

    fn flush_ack(&mut self, channel: Snowflake) {
        if let Some(message) = self.ack_due.remove(&channel) {
            self.send_ack(channel, message);
        }
    }

    fn send_ack(&mut self, channel: Snowflake, message: Snowflake) {
        let Some(http) = self.session.as_ref().map(|s| s.http.clone()) else {
            return;
        };
        self.last_ack.insert(channel, std::time::Instant::now());
        let guild = self
            .shared
            .read_state(|st| st.channel(channel).and_then(|c| c.guild_id));
        tokio::spawn(async move {
            if let Err(e) = http.ack(Page { guild, channel }, message).await {
                tracing::debug!("okundu gönderilemedi: {e}");
            }
        });
    }

    /// Yeni mesaj için masaüstü bildirimi (açık+odaktaki kanal hariç).
    fn maybe_notify(&self, msg: &Message) {
        let Some(filter) = self.env.settings.notify else {
            return;
        };
        if self.focused && self.open.is_some_and(|o| o.channel == msg.channel_id) {
            return;
        }
        let now = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        )
        .unwrap_or(i64::MAX);
        let content = self.shared.read_state(|st| {
            if !st.should_notify(msg, filter, now) {
                return None;
            }
            let author = st.author_name(msg.guild_id, &msg.author);
            let place = match msg.guild_id {
                Some(g) => {
                    let guild = st
                        .guilds()
                        .iter()
                        .find(|x| x.id == g)
                        .map_or("?", |x| x.name.as_str());
                    format!("{author} · #{} ({guild})", st.channel_title(msg.channel_id))
                }
                None => author,
            };
            Some((place, msg.content.chars().take(200).collect::<String>()))
        });
        let Some((summary, body)) = content else {
            return;
        };
        tokio::task::spawn_blocking(move || {
            let body = if body.is_empty() {
                "(ek veya embed)".to_owned()
            } else {
                body
            };
            if let Err(e) = notify_rust::Notification::new()
                .appname("Digcord")
                .summary(&summary)
                .body(&body)
                .show()
            {
                tracing::debug!("bildirim gösterilemedi: {e}");
            }
        });
    }

    fn load_older(&mut self, channel: Snowflake) {
        let Some(open) = self.open.filter(|o| o.channel == channel) else {
            return;
        };
        let oldest = self.shared.read_state(|st| {
            if st.history_complete(channel) {
                None
            } else {
                st.messages(channel).and_then(|m| m.front()).map(|m| m.id)
            }
        });
        if let Some(before) = oldest {
            self.fetch(open, Some(before), OLDER_PAGE);
        }
    }

    /// Görünen yazarlardan üye bilgisi bilinmeyenler için op 8.
    fn request_members(&mut self, open: Open) {
        let Some(gid) = open.guild else { return };
        let unknown = self
            .shared
            .read_state(|st| st.unknown_members(open.channel, MEMBERS_PER_REQUEST * 2));
        let ids: Vec<Snowflake> = unknown
            .into_iter()
            .filter(|u| self.requested_members.insert((gid, *u)))
            .take(MEMBERS_PER_REQUEST)
            .collect();
        if !ids.is_empty() {
            self.command(Command::RequestMembers {
                guild: gid,
                user_ids: ids,
            });
        }
    }

    fn on_history(&mut self, h: History) {
        self.loading.remove(&h.channel);
        match h.result {
            Ok(messages) => {
                self.write(|st| {
                    st.history_loaded(h.channel, h.before, usize::from(h.limit), messages)
                });
                if let Some(open) = self.open.filter(|o| o.channel == h.channel) {
                    self.request_members(open);
                    self.mark_seen(open.channel);
                }
            }
            Err(e) => {
                tracing::warn!("geçmiş alınamadı: {e}");
                self.notice(format!("Geçmiş alınamadı: {e}"));
            }
        }
    }

    async fn on_update(&mut self, update: Update) {
        match update {
            Update::Connecting => self.send(UiEvent::Status(Status::Connecting)),
            Update::Ready(ready) => {
                let status = online(&ready);
                self.write(|st| st.load_ready(&ready));
                // Yeni oturum: abonelikler ve istekler sıfırlanır; açık kanal yeniden yüklenir.
                self.subscribed.clear();
                self.requested_members.clear();
                self.loading.clear();
                if let Some(open) = self.open {
                    self.open_channel(open);
                }
                self.last_online = Some(status.clone());
                self.send(UiEvent::Status(status));
                // Kayıt başarısızsa bağlantı sürer; sadece bir sonraki açılışta token tekrar sorulur.
                if let Some(token) = self.unsaved.take()
                    && let Err(e) = blocking(move || keystore::save(&token)).await
                {
                    tracing::warn!("token kaydedilemedi: {e:#}");
                }
            }
            Update::Resumed => {
                let status = self.last_online.clone().unwrap_or(Status::Connecting);
                self.send(UiEvent::Status(status));
            }
            Update::Reconnecting { after, reason } => {
                self.send(UiEvent::Status(Status::Reconnecting {
                    secs: after.as_secs().max(1),
                    reason,
                }));
            }
            Update::InvalidToken => {
                self.session = None;
                self.unsaved = None;
                if let Err(e) = blocking(keystore::delete).await {
                    tracing::warn!("{e:#}");
                }
                self.send(UiEvent::ShowLogin {
                    error: Some("Token geçersiz (4004), tekrar dene".into()),
                });
            }
            Update::Fatal(reason) => {
                self.session = None;
                self.send(UiEvent::Status(Status::Failed(reason)));
            }
            Update::Event(event) => self.on_event(event),
        }
    }

    fn on_event(&mut self, event: GatewayEvent) {
        use GatewayEvent as E;
        match event {
            E::MessageCreate(m) => {
                self.maybe_notify(&m);
                let channel = m.channel_id;
                self.write(|st| st.message_create(*m));
                self.mark_seen(channel);
            }
            E::MessageUpdate(m) => self.write(|st| st.message_update(*m)),
            E::MessageDelete(d) => self.write(|st| st.message_delete(&d)),
            E::ChannelCreate(c) | E::ChannelUpdate(c) => self.write(|st| st.upsert_channel(&c)),
            E::ChannelDelete(c) => self.write(|st| st.remove_channel(c.id)),
            E::GuildCreate(g) => self.write(|st| st.upsert_guild(&g)),
            E::GuildMembersChunk(c) => self.write(|st| st.add_members(&c)),
            E::ReactionAdd(r) => self.write(|st| st.reaction_add(&r)),
            E::ReactionRemove(r) => self.write(|st| st.reaction_remove(&r)),
            E::TypingStart(t) => self.write(|st| st.typing_start(&t)),
            E::MessageAck(a) => self.write(|st| st.apply_ack(&a)),
            E::VoiceStateUpdate(vs) => self.on_voice_state(&vs),
            E::VoiceServerUpdate(server) => self.on_voice_server(server),
        }
    }

    async fn on_action(&mut self, action: Action) -> bool {
        match action {
            Action::SubmitToken(raw) => {
                let Some(token) = Token::new(&raw) else {
                    self.send(UiEvent::ShowLogin {
                        error: Some("Token boş olamaz".into()),
                    });
                    return true;
                };
                self.unsaved = Some(token.clone());
                self.start(token).await;
            }
            Action::OpenChannel {
                guild,
                channel,
                previous,
            } => {
                if let Some(prev) = previous {
                    self.write(|st| st.trim_channel(prev));
                }
                self.open_channel(Open { guild, channel });
            }
            Action::LoadOlder { channel } => self.load_older(channel),
            Action::SendMessage {
                guild,
                channel,
                content,
                reply_to,
            } => self.send_message(Page { guild, channel }, content, reply_to),
            Action::EditMessage {
                guild,
                channel,
                message,
                content,
            } => self.edit_message(Page { guild, channel }, message, content),
            Action::DeleteMessage {
                guild,
                channel,
                message,
            } => self.delete_message(Page { guild, channel }, message),
            Action::Retry {
                guild,
                channel,
                message,
            } => self.retry(Page { guild, channel }, message),
            Action::React {
                guild,
                channel,
                message,
                emoji,
                add,
            } => self.react(Page { guild, channel }, message, emoji, add),
            Action::Typing { guild, channel } => {
                if self.env.settings.send_typing {
                    self.typing(Page { guild, channel });
                }
            }
            Action::Focus(focused) => {
                self.focused = focused;
                if let Some(open) = self.open {
                    self.mark_seen(open.channel);
                }
            }
            Action::Upload {
                guild,
                channel,
                path,
                content,
                max_bytes,
            } => self.upload(Page { guild, channel }, path, content, max_bytes),
            Action::Copy(text) => self.copy(text),
            Action::LoadImage(job) => self.load_image(job),
            Action::JoinVoice { guild, channel } => self.join_voice(guild, channel),
            Action::LeaveVoice => self.leave_voice(),
            Action::ToggleMute => self.toggle_voice_flag(false),
            Action::ToggleDeaf => self.toggle_voice_flag(true),
            Action::PushToTalk(pressed) => {
                if let Some(v) = &self.voice {
                    v.set_push_to_talk(pressed);
                }
            }
            Action::PushToTalkUnavailable => {
                self.env.settings.push_to_talk = false;
                self.notice(
                    "Bas-konuş için kitty klavye protokolü gerekli; ses algılama kullanılıyor",
                );
            }
            Action::SetVolume { user, volume } => {
                if let Some(v) = &self.voice {
                    v.set_volume(user.0, volume);
                }
            }
            Action::OpenExternal(url) => self.open_external(url),
            Action::Quit => return false,
        }
        true
    }
}

trait ReadState {
    fn read_state<T>(&self, f: impl FnOnce(&State) -> T) -> T;
}

impl ReadState for Shared {
    fn read_state<T>(&self, f: impl FnOnce(&State) -> T) -> T {
        f(&self.state.read().unwrap_or_else(|e| e.into_inner()))
    }
}

fn online(ready: &Ready) -> Status {
    Status::Online {
        user: ready.user.display_name().to_owned(),
        guilds: ready.guilds.len(),
        channels: ready.channel_count(),
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(f).await?
}

/// CDN istemcisi: tarayıcıyla aynı User-Agent, `~/.cache/digcord/media` önbelleği.
fn media_fetcher(env: &Env) -> Option<Arc<Fetcher>> {
    let props = ClientProperties::chrome_linux(
        &env.locale,
        env.settings.chrome_major,
        env.settings.build_number,
    );
    let dir = dirs::cache_dir().map(|d| d.join("digcord").join("media"));
    let fetcher = match Fetcher::new(props.user_agent(), dir.clone()) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("medya istemcisi kurulamadı: {e}");
            return None;
        }
    };
    if let Some(dir) = dir {
        tokio::task::spawn_blocking(move || {
            if let Err(e) = digcord_media::prune(&dir, MEDIA_CACHE_BYTES) {
                tracing::debug!("medya önbelleği temizlenemedi: {e}");
            }
        });
    }
    Some(Arc::new(fetcher))
}

/// `actions` kapanana veya `Action::Quit` gelene kadar çalışır.
pub async fn run(
    mut actions: mpsc::UnboundedReceiver<Action>,
    ui: Sender<UiEvent>,
    shared: Arc<Shared>,
    env: Env,
) {
    let (updates_tx, mut updates) = mpsc::channel(256);
    let (history_tx, mut history) = mpsc::channel(16);
    let (jobs_tx, mut jobs) = mpsc::unbounded_channel::<Job>();
    let mut c = Controller {
        ui,
        shared,
        env,
        updates_tx,
        history_tx,
        jobs_tx,
        nonce_seq: 0,
        session: None,
        unsaved: None,
        last_online: None,
        open: None,
        subscribed: HashSet::new(),
        loading: HashSet::new(),
        requested_members: HashSet::new(),
        focused: true,
        last_ack: std::collections::HashMap::new(),
        ack_due: std::collections::HashMap::new(),
        media: None,
        refetched: HashSet::new(),
        voice: None,
        voice_server: None,
        voice_rejoined: false,
    };
    c.media = media_fetcher(&c.env);

    let stored = blocking(|| {
        keystore::init()?;
        keystore::load()
    })
    .await;
    match stored {
        Ok(Some(token)) => {
            tracing::info!("kayıtlı token bulundu");
            c.start(token).await;
        }
        Ok(None) => c.send(UiEvent::ShowLogin { error: None }),
        Err(e) => {
            tracing::warn!("{e:#}");
            c.send(UiEvent::ShowLogin {
                error: Some(format!("{e:#} — token kaydedilemeyecek")),
            });
        }
    }

    loop {
        tokio::select! {
            action = actions.recv() => match action {
                Some(a) => if !c.on_action(a).await { break },
                None => break,
            },
            Some(update) = updates.recv() => c.on_update(update).await,
            Some(h) = history.recv() => c.on_history(h),
            Some(job) = jobs.recv() => job(&mut c),
        }
    }

    if let Some(s) = c.session.take() {
        s.stop().await;
    }
}

/// Panoya kopyalar: Wayland'de `wl-copy`, yoksa `xclip`. Ek bağımlılık yerine sistem araçları.
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let candidates: [(&str, &[&str]); 2] =
        [("wl-copy", &[]), ("xclip", &["-selection", "clipboard"])];
    for (bin, args) in candidates {
        let Ok(mut child) = Command::new(bin)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .map_err(|e| e.to_string())?;
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        return if status.success() {
            Ok(())
        } else {
            Err(format!("{bin} başarısız"))
        };
    }
    Err("wl-copy veya xclip bulunamadı".to_owned())
}

/// `LC_ALL` / `LC_MESSAGES` / `LANG` değerini BCP-47'ye çevirir (`tr_TR.UTF-8` → `tr-TR`).
fn system_locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find_map(|v| locale_from_posix(&v))
        .unwrap_or_else(|| "en-US".to_owned())
}

fn locale_from_posix(value: &str) -> Option<String> {
    let base = value.split(['.', '@']).next()?;
    let (lang, region) = base.split_once('_')?;
    let valid = |s: &str, n: usize| s.len() == n && s.chars().all(|c| c.is_ascii_alphabetic());
    (valid(lang, 2) && valid(region, 2)).then(|| {
        format!(
            "{}-{}",
            lang.to_ascii_lowercase(),
            region.to_ascii_uppercase()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_locale_to_bcp47() {
        assert_eq!(locale_from_posix("tr_TR.UTF-8").as_deref(), Some("tr-TR"));
        assert_eq!(locale_from_posix("en_US").as_deref(), Some("en-US"));
        assert_eq!(locale_from_posix("de_DE@euro").as_deref(), Some("de-DE"));
        assert_eq!(locale_from_posix("C.UTF-8"), None);
        assert_eq!(locale_from_posix("POSIX"), None);
        assert_eq!(locale_from_posix(""), None);
    }
}
