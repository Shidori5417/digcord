//! TUI ile ağ/anahtarlık katmanları arasındaki köprü ve store'un **tek yazarı**.
//! tokio üzerinde çalışır; TUI'ye sadece [`UiEvent`] gönderir, TUI'den [`Action`] alır.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Duration;

use digcord_gateway::{Command, Config, GatewayEvent, Update};
use digcord_http::{Http, HttpError, Page};
use digcord_model::{
    ClientProperties, DEFAULT_BUILD_NUMBER, DEFAULT_CHROME_MAJOR, Message, Ready, Snowflake, Token,
};
use digcord_store::State;
use digcord_tui::{Action, Shared, Status, UiEvent};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::keystore;

/// Kanal açılışında ve yukarı kaydırınca istenen mesaj sayıları (PROTOCOL_NOTES §4.1).
const FIRST_PAGE: u8 = 10;
const OLDER_PAGE: u8 = 20;
/// op 8 isteği başına en fazla kullanıcı.
const MEMBERS_PER_REQUEST: usize = 100;

struct Session {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
    commands: mpsc::Sender<Command>,
    http: Arc<Http>,
}

impl Session {
    fn start(token: Token, env: &Env, updates: mpsc::Sender<Update>) -> Result<Self, HttpError> {
        let properties =
            ClientProperties::chrome_linux(&env.locale, DEFAULT_CHROME_MAJOR, DEFAULT_BUILD_NUMBER);
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
}

impl Env {
    pub fn detect() -> Self {
        Self {
            locale: system_locale(),
            timezone: jiff::tz::TimeZone::system()
                .iana_name()
                .unwrap_or("UTC")
                .to_owned(),
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
}

impl Controller {
    fn send(&self, ev: UiEvent) {
        let _ = self.ui.send(ev);
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
                }
            }
            Err(e) => {
                tracing::warn!("geçmiş alınamadı: {e}");
                self.send(UiEvent::Status(Status::Failed(format!(
                    "geçmiş alınamadı: {e}"
                ))));
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
            E::MessageCreate(m) => self.write(|st| st.message_create(*m)),
            E::MessageUpdate(m) => self.write(|st| st.message_update(*m)),
            E::MessageDelete(d) => self.write(|st| st.message_delete(&d)),
            E::ChannelCreate(c) | E::ChannelUpdate(c) => self.write(|st| st.upsert_channel(&c)),
            E::ChannelDelete(c) => self.write(|st| st.remove_channel(c.id)),
            E::GuildCreate(g) => self.write(|st| st.upsert_guild(&g)),
            E::GuildMembersChunk(c) => self.write(|st| st.add_members(&c)),
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

/// `actions` kapanana veya `Action::Quit` gelene kadar çalışır.
pub async fn run(
    mut actions: mpsc::UnboundedReceiver<Action>,
    ui: Sender<UiEvent>,
    shared: Arc<Shared>,
    env: Env,
) {
    let (updates_tx, mut updates) = mpsc::channel(256);
    let (history_tx, mut history) = mpsc::channel(16);
    let mut c = Controller {
        ui,
        shared,
        env,
        updates_tx,
        history_tx,
        session: None,
        unsaved: None,
        last_online: None,
        open: None,
        subscribed: HashSet::new(),
        loading: HashSet::new(),
        requested_members: HashSet::new(),
    };

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
        }
    }

    if let Some(s) = c.session.take() {
        s.stop().await;
    }
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
