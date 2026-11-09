//! ratatui arayüzü, girdi ve kısayollar.
//!
//! TUI ağ katmanını bilmez: dışarıdan [`UiEvent`] alır, dışarıya [`Action`] verir ve
//! paylaşılan store'u ([`Shared`]) sadece okur.

mod compose;
mod editor;
mod fuzzy;
mod keymap;
mod markdown;
mod media;
mod render;
mod settings;
mod theme;
mod view;
mod viewer;

pub use keymap::Keymap;
pub use media::{ImageDone, ImageJob, ImageMode, Options};
/// Tema yuvalarının türü.
pub use ratatui::style::Color;
pub use settings::{SettingItem, SettingKind, SettingValue};
pub use theme::{Theme, ThemeError};

use std::cell::Cell;
use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock, RwLockReadGuard};

use digcord_model::{ReactionEmoji, Snowflake, channel_kind};
use digcord_store::{State, TreeKind};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{DefaultTerminal, Frame};

use crate::keymap::{Act, Scope};

/// Store + "çizilmesi gereken değişiklik var" bayrağı. Tek yazar denetleyicidir; TUI okur.
#[derive(Default)]
pub struct Shared {
    pub state: RwLock<State>,
    dirty: AtomicBool,
}

impl Shared {
    /// Yazar her değişiklikten sonra çağırır; TUI'ye sadece bekleyen bildirim yoksa `true` döner
    /// (bildirimler birikmez).
    pub fn mark_dirty(&self) -> bool {
        !self.dirty.swap(true, Ordering::AcqRel)
    }

    fn read(&self) -> RwLockReadGuard<'_, State> {
        // Zehirlenmiş kilit: yazar panikledi; son tutarlı durumu okumaya devam et.
        self.state.read().unwrap_or_else(|e| e.into_inner())
    }
}

/// TUI'ye gelen olaylar.
#[derive(Debug)]
pub enum UiEvent {
    Input(Event),
    Status(Status),
    /// Token iste; `error` bir önceki denemenin neden başarısız olduğu.
    ShowLogin {
        error: Option<String>,
    },
    /// Store değişti, yeniden çiz.
    StoreChanged,
    /// Durum çubuğunda bir sonraki tuşa kadar gösterilecek kısa bilgi/hata.
    Notice(String),
    /// Bir resim işi bitti.
    Image(ImageDone),
    /// `:theme <ad>` ile istenen tema yüklendi.
    Theme {
        name: String,
        theme: Box<Theme>,
    },
}

/// Bağlantı durumu (durum çubuğunda gösterilir).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Starting,
    Connecting,
    Online {
        user: String,
        guilds: usize,
        channels: usize,
    },
    Reconnecting {
        secs: u64,
        reason: String,
    },
    Failed(String),
}

/// TUI'den uygulamaya giden eylemler.
#[derive(Debug, PartialEq)]
pub enum Action {
    SubmitToken(String),
    /// Kanal açıldı. `previous`: kapatılan kanal (penceresi kırpılabilir).
    OpenChannel {
        guild: Option<Snowflake>,
        channel: Snowflake,
        previous: Option<Snowflake>,
    },
    /// Görünüm en eski yüklü mesaja ulaştı; daha eskisi istenir.
    LoadOlder {
        channel: Snowflake,
    },
    SendMessage {
        guild: Option<Snowflake>,
        channel: Snowflake,
        content: String,
        reply_to: Option<Snowflake>,
    },
    EditMessage {
        guild: Option<Snowflake>,
        channel: Snowflake,
        message: Snowflake,
        content: String,
    },
    DeleteMessage {
        guild: Option<Snowflake>,
        channel: Snowflake,
        message: Snowflake,
    },
    /// Başarısız iyimser mesajı yeniden gönder.
    Retry {
        guild: Option<Snowflake>,
        channel: Snowflake,
        message: Snowflake,
    },
    React {
        guild: Option<Snowflake>,
        channel: Snowflake,
        message: Snowflake,
        emoji: ReactionEmoji,
        add: bool,
    },
    Typing {
        guild: Option<Snowflake>,
        channel: Snowflake,
    },
    Upload {
        guild: Option<Snowflake>,
        channel: Snowflake,
        path: std::path::PathBuf,
        content: String,
        max_bytes: u64,
    },
    /// Üst kanalın thread listesini REST'ten iste. `archived == false`: forumun aktif
    /// gönderileri; `true`: arşivli herkese açık thread'ler (`:archived`).
    LoadThreads {
        guild: Option<Snowflake>,
        channel: Snowflake,
        archived: bool,
    },
    /// Panoya kopyala.
    Copy(String),
    /// Terminal penceresi odak kazandı/kaybetti (bildirim ve okundu kararları için).
    Focus(bool),
    /// Resmi indir, çöz ve terminal için kodla; sonuç [`UiEvent::Image`].
    LoadImage(ImageJob),
    /// Adresi sistemin varsayılan uygulamasında aç (`xdg-open`).
    OpenExternal(String),
    /// Ses kanalına (ya da DM aramasına, `guild: None`) katıl.
    JoinVoice {
        guild: Option<Snowflake>,
        channel: Snowflake,
    },
    LeaveVoice,
    ToggleMute,
    ToggleDeaf,
    /// Bas-konuş tuşu basıldı/bırakıldı.
    PushToTalk(bool),
    /// Bas-konuş istendi ama terminal tuş bırakmayı bildiremiyor: ses algılamaya dön.
    PushToTalkUnavailable,
    /// `:devices`: ses giriş/çıkış aygıtlarını listele (sonuç [`UiEvent::Notice`]).
    ListDevices,
    /// Bir kullanıcının ses seviyesi (0.0–2.0).
    SetVolume {
        user: Snowflake,
        volume: f32,
    },
    /// Çalışırken uygulanabilen bir ayar değişti (`bölüm.anahtar`).
    ApplySetting {
        key: String,
        value: SettingValue,
    },
    /// Ayarlar ekranında `s`: değişen ayarlar config.toml'a yazılsın.
    SaveSettings(Vec<(String, SettingValue)>),
    /// `:theme <ad>`: tema dosyasını oku; sonuç [`UiEvent::Theme`].
    LoadTheme(String),
    Quit,
}

/// Terminali hazırlar, `events` kanalı kapanana veya kullanıcı çıkana kadar çizer.
/// Girdi ayrı bir thread'de okunup `tx` ile aynı kanala gönderilir; döngü boşta CPU harcamaz.
///
/// `ratatui::run` panic olduğunda da terminali geri yükleyen bir hook kurar; bu yüzden
/// çağıran kendi panic hook'larını bundan **önce** kurmalı.
pub fn run(
    tx: Sender<UiEvent>,
    events: Receiver<UiEvent>,
    shared: Arc<Shared>,
    options: Options,
    mut actions: impl FnMut(Action),
) -> io::Result<()> {
    ratatui::run(|terminal| {
        use ratatui::crossterm::event::{DisableFocusChange, EnableFocusChange};
        // Grafik yeteneği sorgusu terminale yazıp yanıtını okur: alternatif ekrana geçtikten
        // sonra, girdi thread'i başlamadan önce yapılmalı (yoksa yanıtı girdi thread'i yer).
        let picker = (options.images != ImageMode::Off).then(|| {
            ratatui_image::picker::Picker::from_query_stdio().unwrap_or_else(|e| {
                tracing::warn!("terminal grafik sorgusu başarısız: {e}");
                ratatui_image::picker::Picker::halfblocks()
            })
        });
        // Kitty klavye protokolü (varsa): Ctrl+Shift+M gibi tuşlar Enter'dan ayrılabilsin.
        // Sorgu da terminale yazıp okur: girdi thread'inden önce.
        let enhanced =
            ratatui::crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false);
        if enhanced {
            use ratatui::crossterm::event::{
                KeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
            };
            let _ = ratatui::crossterm::execute!(
                io::stdout(),
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            );
        }
        std::thread::Builder::new()
            .name("digcord-input".into())
            .spawn(move || input_loop(&tx))?;
        // Odak olayları desteklenmiyorsa sorun değil: bildirim kararı "odakta" varsayar.
        let _ = ratatui::crossterm::execute!(io::stdout(), EnableFocusChange);
        let mut app = App::new(shared);
        app.push_to_talk = options.push_to_talk && enhanced;
        app.theme = options.theme;
        app.keys = options.keys.clone();
        app.settings = settings::State::new(options.settings.clone());
        app.images = media::Images::new(picker, options);
        if let Some(name) = app.images.protocol_name() {
            tracing::info!(protocol = name, "resim gösterimi");
        }
        let result = app.run(terminal, &events, &mut actions);
        if enhanced {
            let _ = ratatui::crossterm::execute!(
                io::stdout(),
                ratatui::crossterm::event::PopKeyboardEnhancementFlags
            );
        }
        let _ = ratatui::crossterm::execute!(io::stdout(), DisableFocusChange);
        result
    })
}

/// Terminal girdisini okur. Tek bir okuma hatası (ör. yeniden boyutlandırma sırasında boyut
/// sorgusu başarısız olursa crossterm hata döner) thread'i bitirmemeli: biterse ne tuşlar ne
/// de yeniden boyutlandırma olayları gelir. Üst üste çok hata olursa vazgeçilir.
fn input_loop(tx: &Sender<UiEvent>) {
    const MAX_CONSECUTIVE_ERRORS: u32 = 50;
    let mut errors = 0;
    loop {
        match event::read() {
            Ok(ev) => {
                errors = 0;
                if tx.send(UiEvent::Input(ev)).is_err() {
                    return;
                }
            }
            Err(e) => {
                errors += 1;
                tracing::warn!(errors, "terminal girdisi okunamadı: {e}");
                if errors >= MAX_CONSECUTIVE_ERRORS {
                    tracing::error!("terminal girdisi okunamıyor; girdi thread'i durdu");
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    }
}

#[derive(Debug)]
enum Screen {
    Main,
    Login {
        input: String,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Guilds,
    Channels,
    Messages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Open {
    guild: Option<Snowflake>,
    channel: Snowflake,
}

/// Mesaj görünümünün konumu. `anchor == None`: en alta yapışık (yeni mesajlar görünür).
/// Aksi halde ekranın alt kenarı, `anchor` mesajının son `offset` satırı aşağıda gizli kalacak
/// şekilde hizalıdır. Böylece çizim ve kaydırma sadece görünen mesajlara dokunur.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Scroll {
    anchor: Option<Snowflake>,
    offset: usize,
}

/// Kanal panelindeki bir satır.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    id: Snowflake,
    label: String,
    depth: u8,
    kind: EntryKind,
    unread: bool,
    mentions: u32,
    muted: bool,
    /// Arşivli veya kilitli thread: soluk çizilir.
    dim: bool,
}

/// Bu sayıya kadar thread'i olan metin kanalının thread listesi başta açık gelir.
const AUTO_EXPAND_THREADS: usize = 3;

/// DM panelindeki "ARKADAŞLAR" başlığının sahte id'si (gerçek snowflake'ler hep > 0).
const FRIENDS_HEADER: Snowflake = Snowflake(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Category {
        collapsed: bool,
    },
    /// Mesajları okunabilen kanal (metin, duyuru, DM).
    Readable,
    /// Katılınabilen ses kanalı.
    Voice,
    /// Ses kanalındaki bir kullanıcı (seçilebilir ama eylemi yok).
    VoiceMember {
        speaking: bool,
    },
    /// Forum / medya kanalı: Enter gönderilerini (thread) açıp kapatır; mesajı yoktur.
    Forum {
        expanded: bool,
    },
    /// Açılmayan kanal (sahne...).
    Other,
}

/// Uygulama durumu.
pub struct App {
    screen: Screen,
    status: Status,
    should_quit: bool,
    shared: Arc<Shared>,
    tz: jiff::tz::TimeZone,
    focus: Focus,
    /// 0 = DM'ler, i = `guilds()[i - 1]`.
    guild_sel: usize,
    chan_sel: usize,
    collapsed: HashSet<Snowflake>,
    /// Thread listesinin varsayılan açık/kapalı durumunu tersine çeviren üst kanallar.
    thread_flip: HashSet<Snowflake>,
    open: Option<Open>,
    scroll: Scroll,
    /// Son çizimde mesaj alanının boyutu (kaydırma hesapları için).
    msg_area: Cell<(u16, u16)>,
    chan_offset: Cell<usize>,
    want_older: Cell<bool>,
    mode: compose::Mode,
    editor: editor::Editor,
    /// COMMAND ve tepki istemi girişi.
    command: editor::Editor,
    target: compose::Target,
    selected: Option<Snowflake>,
    /// SELECT'te ilk `d` basıldı (`d d` silme).
    pending_d: bool,
    comp_sel: usize,
    last_typing: Option<(Snowflake, std::time::Instant)>,
    notice: Option<String>,
    /// Son çizimde görünen en eski ve en yeni mesaj.
    visible: Cell<Option<(Snowflake, Snowflake)>>,
    images: media::Images,
    /// Ctrl+Space basılıyken konuş (basma/bırakma olayları geliyorsa).
    push_to_talk: bool,
    /// Tam ekran resim görüntüleyici (`Mode::Viewer`).
    viewer: Option<viewer::Viewer>,
    theme: Theme,
    keys: Keymap,
    /// `:settings` ekranı (`Mode::Settings`).
    settings: settings::State,
}

impl App {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self {
            screen: Screen::Main,
            status: Status::Starting,
            should_quit: false,
            shared,
            tz: jiff::tz::TimeZone::system(),
            focus: Focus::Guilds,
            guild_sel: 0,
            chan_sel: 0,
            collapsed: HashSet::new(),
            thread_flip: HashSet::new(),
            open: None,
            scroll: Scroll::default(),
            msg_area: Cell::new((0, 0)),
            chan_offset: Cell::new(0),
            want_older: Cell::new(false),
            mode: compose::Mode::Normal,
            editor: editor::Editor::new(),
            command: editor::Editor::new(),
            target: compose::Target::New,
            selected: None,
            pending_d: false,
            comp_sel: 0,
            last_typing: None,
            notice: None,
            visible: Cell::new(None),
            images: media::Images::off(),
            push_to_talk: false,
            viewer: None,
            theme: Theme::default(),
            keys: Keymap::default(),
            settings: settings::State::default(),
        }
    }

    fn run(
        &mut self,
        terminal: &mut DefaultTerminal,
        events: &Receiver<UiEvent>,
        actions: &mut impl FnMut(Action),
    ) -> io::Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| self.draw(frame))?;
            if let Some(a) = self.take_older_request() {
                actions(a);
            }
            for job in self.images.take_wanted() {
                actions(Action::LoadImage(job));
            }
            // "Yazıyor" göstergesi süresi dolunca kaybolsun diye: biri yazıyorsa saniyede bir çiz.
            let ev = if self.typing_visible() {
                match events.recv_timeout(std::time::Duration::from_secs(1)) {
                    Ok(ev) => ev,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            } else {
                let Ok(ev) = events.recv() else { break };
                ev
            };
            if let Some(action) = self.handle(ev) {
                actions(action);
            }
        }
        actions(Action::Quit);
        Ok(())
    }

    fn typing_visible(&self) -> bool {
        self.open.is_some_and(|o| {
            !self
                .shared
                .read()
                .typing_names(o.channel, unix_now())
                .is_empty()
        })
    }

    /// Son çizim en eski yüklü mesaja ulaştıysa daha eski sayfa isteği.
    fn take_older_request(&self) -> Option<Action> {
        if !self.want_older.replace(false) {
            return None;
        }
        self.open.map(|o| Action::LoadOlder { channel: o.channel })
    }

    /// Bir olayı işler; gerekiyorsa dışarıya bir eylem döndürür.
    pub fn handle(&mut self, ev: UiEvent) -> Option<Action> {
        match ev {
            UiEvent::Input(Event::Key(key)) => self.handle_key(key),
            UiEvent::Input(Event::Paste(text)) => {
                match (&mut self.screen, self.mode) {
                    (Screen::Login { input, .. }, _) => {
                        input.extend(text.chars().filter(|c| !c.is_control()));
                    }
                    (Screen::Main, compose::Mode::Insert) => self.editor.insert_str(&text),
                    (Screen::Main, compose::Mode::Command | compose::Mode::React) => {
                        self.command.insert_str(&text.replace('\n', " "));
                    }
                    _ => {}
                }
                None
            }
            UiEvent::Input(Event::FocusGained) => Some(Action::Focus(true)),
            UiEvent::Input(Event::FocusLost) => Some(Action::Focus(false)),
            UiEvent::Input(_) => None,
            UiEvent::Status(status) => {
                self.status = status;
                None
            }
            UiEvent::ShowLogin { error } => {
                self.screen = Screen::Login {
                    input: String::new(),
                    error,
                };
                None
            }
            UiEvent::StoreChanged => {
                self.shared.dirty.store(false, Ordering::Release);
                None
            }
            UiEvent::Notice(text) => {
                self.notice = Some(text);
                None
            }
            UiEvent::Image(done) => {
                self.images.done(done);
                None
            }
            UiEvent::Theme { name, theme } => {
                self.theme = *theme;
                self.notice = Some(format!("Tema: {name}"));
                self.settings
                    .set_external("ui.theme", SettingValue::Str(name));
                None
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        if self.push_to_talk && self.keys.lookup(Scope::Global, &key) == Some(Act::PushToTalk) {
            return match key.kind {
                KeyEventKind::Press => Some(Action::PushToTalk(true)),
                KeyEventKind::Release => Some(Action::PushToTalk(false)),
                KeyEventKind::Repeat => None,
            };
        }
        if key.kind != KeyEventKind::Press {
            return None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        // Ctrl+C sabit: yanlış bir kısayol atamasıyla çıkışsız kalınmasın.
        if ctrl && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return None;
        }
        if let Screen::Login { input, error } = &mut self.screen {
            match key.code {
                KeyCode::Esc => self.should_quit = true,
                KeyCode::Enter if !input.trim().is_empty() => {
                    let token = std::mem::take(input);
                    self.screen = Screen::Main;
                    self.status = Status::Connecting;
                    return Some(Action::SubmitToken(token));
                }
                KeyCode::Enter => *error = Some("Token boş olamaz".into()),
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char(c) if !ctrl => input.push(c),
                _ => {}
            }
            return None;
        }

        self.notice = None;
        // Ses: her modda (kitty klavye protokolü gerekir; yoksa :mute / :deaf).
        match self.keys.lookup(Scope::Global, &key) {
            Some(Act::ToggleMute) => return Some(Action::ToggleMute),
            Some(Act::ToggleDeaf) => return Some(Action::ToggleDeaf),
            _ => {}
        }
        match self.mode {
            compose::Mode::Insert => return self.insert_key(key),
            compose::Mode::Select => return self.select_key(key),
            compose::Mode::Command | compose::Mode::React => return self.command_key(key),
            compose::Mode::Switcher => return self.switcher_key(key),
            compose::Mode::ConfirmDelete(id) => return self.confirm_key(key, id),
            compose::Mode::Viewer => return self.viewer_key(key),
            compose::Mode::Settings => return self.settings_key(key),
            compose::Mode::Normal => {}
        }
        let act = self.keys.lookup(Scope::Normal, &key)?;
        if let Some(result) = self.normal_mode_key(act) {
            return result;
        }
        let half_page = usize::from(self.msg_area.get().1 / 2).max(1);
        match act {
            Act::Quit => self.should_quit = true,
            Act::Switcher => {
                self.command.clear();
                self.comp_sel = 0;
                self.mode = compose::Mode::Switcher;
            }
            Act::NextUnread => return self.jump_unread(true),
            Act::PrevUnread => return self.jump_unread(false),
            Act::NextFocus => self.focus = self.next_focus(true),
            Act::PrevFocus => self.focus = self.next_focus(false),
            Act::MoveDown => self.move_down(1),
            Act::MoveUp => self.move_up(1),
            Act::PageDown => self.scroll_down(half_page),
            Act::PageUp => self.scroll_up(half_page),
            Act::ScrollBottom => self.scroll = Scroll::default(),
            Act::Activate => {
                // Mesaj panelinde Enter, SELECT'e girer (`l` girmez).
                if key.code == KeyCode::Enter
                    && self.focus == Focus::Messages
                    && self.open.is_some()
                {
                    self.enter_select();
                } else {
                    return self.activate();
                }
            }
            _ => {}
        }
        None
    }

    fn next_focus(&self, forward: bool) -> Focus {
        use Focus::*;
        let order = if self.open.is_some() {
            &[Guilds, Channels, Messages][..]
        } else {
            &[Guilds, Channels][..]
        };
        let i = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        let n = order.len();
        order[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
    }

    fn move_down(&mut self, n: usize) {
        match self.focus {
            Focus::Guilds => {
                let count = self.shared.read().guilds().len() + 1;
                self.guild_sel = (self.guild_sel + n).min(count - 1);
                self.chan_sel = 0;
            }
            Focus::Channels => {
                let count = self.entries().len();
                self.chan_sel = (self.chan_sel + n).min(count.saturating_sub(1));
            }
            Focus::Messages => self.scroll_down(n),
        }
    }

    fn move_up(&mut self, n: usize) {
        match self.focus {
            Focus::Guilds => {
                self.guild_sel = self.guild_sel.saturating_sub(n);
                self.chan_sel = 0;
            }
            Focus::Channels => self.chan_sel = self.chan_sel.saturating_sub(n),
            Focus::Messages => self.scroll_up(n),
        }
    }

    fn selected_guild(&self, st: &State) -> Option<Snowflake> {
        self.guild_sel
            .checked_sub(1)
            .and_then(|i| st.guilds().get(i))
            .map(|g| g.id)
    }

    fn activate(&mut self) -> Option<Action> {
        match self.focus {
            Focus::Guilds => {
                self.focus = Focus::Channels;
                self.chan_sel = 0;
                None
            }
            Focus::Channels => {
                let entry = self.entries().into_iter().nth(self.chan_sel)?;
                match entry.kind {
                    EntryKind::Category { collapsed } => {
                        if collapsed {
                            self.collapsed.remove(&entry.id);
                        } else {
                            self.collapsed.insert(entry.id);
                        }
                        None
                    }
                    EntryKind::Readable => {
                        let guild = self.selected_guild(&self.shared.read());
                        Some(self.open_channel(guild, entry.id))
                    }
                    EntryKind::Voice => {
                        let st = self.shared.read();
                        if st.voice_call().is_some_and(|c| c.channel == entry.id) {
                            return None;
                        }
                        let guild = self.selected_guild(&st);
                        Some(Action::JoinVoice {
                            guild,
                            channel: entry.id,
                        })
                    }
                    EntryKind::Forum { .. } => self.toggle_threads(entry.id),
                    EntryKind::VoiceMember { .. } | EntryKind::Other => None,
                }
            }
            Focus::Messages => None,
        }
    }

    /// Üst kanalın thread listesi açık mı. Varsayılan: metin/duyuru kanalında en çok 3 aktif thread
    /// varsa açık, daha fazlaysa kapalı (panel şişmesin); forumda kapalı. `thread_flip` ile tersine çevrilir.
    fn threads_expanded(&self, st: &State, parent: Snowflake) -> bool {
        let forum = st
            .channel(parent)
            .is_some_and(|c| channel_kind::is_forum(c.kind));
        let active = st
            .threads_of(parent, None)
            .iter()
            .filter(|t| !t.archived)
            .count();
        let default = !forum && active <= AUTO_EXPAND_THREADS;
        default != self.thread_flip.contains(&parent)
    }

    fn flip_threads(&mut self, parent: Snowflake) {
        if !self.thread_flip.remove(&parent) {
            self.thread_flip.insert(parent);
        }
    }

    /// Thread listesini açıp kapatır. Forum açılırken gönderileri henüz REST'ten alınmadıysa
    /// [`Action::LoadThreads`] döner.
    fn toggle_threads(&mut self, parent: Snowflake) -> Option<Action> {
        let st = self.shared.read();
        let was_expanded = self.threads_expanded(&st, parent);
        let fetch = !was_expanded
            && st
                .channel(parent)
                .is_some_and(|c| channel_kind::is_forum(c.kind))
            && !st.threads_fetched(parent);
        let guild = self.selected_guild(&st);
        drop(st);
        self.flip_threads(parent);
        fetch.then_some(Action::LoadThreads {
            guild,
            channel: parent,
            archived: false,
        })
    }

    /// `:threads` / `:archived` hedefi: seçili kanal satırı (kanal panelindeyken) ya da açık
    /// kanal; thread ise üst kanalı.
    fn thread_parent_target(&self) -> Option<Snowflake> {
        let st = self.shared.read();
        let pick = |id: Snowflake| {
            let ch = st.channel(id)?;
            if ch.is_thread() {
                ch.parent_id
            } else {
                channel_kind::has_threads(ch.kind).then_some(id)
            }
        };
        let from_list = (self.focus == Focus::Channels)
            .then(|| self.entries().get(self.chan_sel).map(|e| e.id))
            .flatten()
            .and_then(pick);
        from_list.or_else(|| self.open.and_then(|o| pick(o.channel)))
    }

    /// Kanalı açar: şerit ve liste seçimini ona taşır, mesaj görünümünü sıfırlar.
    fn open_channel(&mut self, guild: Option<Snowflake>, channel: Snowflake) -> Action {
        let previous = self.open.map(|o| o.channel).filter(|c| *c != channel);
        self.guild_sel = {
            let st = self.shared.read();
            guild
                .and_then(|g| st.guilds().iter().position(|x| x.id == g))
                .map_or(0, |i| i + 1)
        };
        if let Some(cat) = self
            .shared
            .read()
            .channel(channel)
            .and_then(|c| c.parent_id)
        {
            // Thread'de bu, üst kanalın kategorisi değil üst kanalın kendisidir (zararsız).
            self.collapsed.remove(&cat);
        }
        // Açılan thread'in listesi kapalıysa görünür kıl.
        let hidden_parent = {
            let st = self.shared.read();
            st.thread_parent(channel)
                .filter(|p| !self.threads_expanded(&st, *p))
        };
        if let Some(parent) = hidden_parent {
            self.flip_threads(parent);
        }
        self.chan_sel = self
            .entries()
            .iter()
            .position(|e| e.id == channel)
            .unwrap_or(0);
        self.open = Some(Open { guild, channel });
        self.scroll = Scroll::default();
        self.focus = Focus::Messages;
        self.editor.clear();
        self.target = compose::Target::New;
        self.selected = None;
        self.mode = compose::Mode::Normal;
        Action::OpenChannel {
            guild,
            channel,
            previous,
        }
    }

    /// `Alt+↓`/`Alt+↑`: açık kanala göre sonraki/önceki okunmamış kanal.
    fn jump_unread(&mut self, forward: bool) -> Option<Action> {
        let list = self.shared.read().unread_channels(unix_now_i64());
        if list.is_empty() {
            self.notice = Some("Okunmamış kanal yok".into());
            return None;
        }
        let current = self.open.map(|o| o.channel);
        let pos = current.and_then(|c| list.iter().position(|(_, ch)| *ch == c));
        let next = match (pos, forward) {
            (Some(i), true) => (i + 1) % list.len(),
            (Some(i), false) => (i + list.len() - 1) % list.len(),
            (None, true) => 0,
            (None, false) => list.len() - 1,
        };
        let (guild, channel) = list[next];
        Some(self.open_channel(guild, channel))
    }

    /// Seçili sunucunun (veya DM'lerin) kanal paneli satırları.
    fn entries(&self) -> Vec<Entry> {
        let st = self.shared.read();
        let now = unix_now_i64();
        let Some(gid) = self.selected_guild(&st) else {
            let mut out: Vec<Entry> = st
                .dm_channels()
                .iter()
                .map(|id| Entry {
                    id: *id,
                    label: format!("@ {}", st.channel_title(*id)),
                    depth: 0,
                    kind: EntryKind::Readable,
                    unread: st.is_unread(*id),
                    mentions: st.mention_count(*id),
                    muted: st.is_muted(*id, now),
                    dim: false,
                })
                .collect();
            let friends = st.friends();
            if !friends.is_empty() {
                let collapsed = self.collapsed.contains(&FRIENDS_HEADER);
                out.push(Entry {
                    id: FRIENDS_HEADER,
                    label: format!(
                        "{} ARKADAŞLAR ({})",
                        if collapsed { "▸" } else { "▾" },
                        friends.len()
                    ),
                    depth: 0,
                    kind: EntryKind::Category { collapsed },
                    unread: false,
                    mentions: 0,
                    muted: false,
                    dim: false,
                });
                if !collapsed {
                    for (uid, name) in friends {
                        // Arkadaşla DM varsa Enter onu açar.
                        let dm = st.dm_channels().iter().copied().find(|c| {
                            st.channel(*c).is_some_and(|ch| {
                                ch.kind == channel_kind::DM && ch.recipient_ids == [uid]
                            })
                        });
                        out.push(Entry {
                            id: dm.unwrap_or(uid),
                            label: format!("  {name}"),
                            depth: 0,
                            kind: if dm.is_some() {
                                EntryKind::Readable
                            } else {
                                EntryKind::Other
                            },
                            unread: false,
                            mentions: 0,
                            muted: false,
                            dim: false,
                        });
                    }
                }
            }
            return out;
        };
        let mut out = Vec::new();
        let mut hidden_parent = false;
        for item in st.guild_tree(gid) {
            match item.kind {
                TreeKind::Category => {
                    let collapsed = self.collapsed.contains(&item.id);
                    hidden_parent = collapsed;
                    let arrow = if collapsed { "▸" } else { "▾" };
                    out.push(Entry {
                        id: item.id,
                        label: format!("{arrow} {}", item.name.to_uppercase()),
                        depth: 0,
                        kind: EntryKind::Category { collapsed },
                        unread: false,
                        mentions: 0,
                        muted: false,
                        dim: false,
                    });
                }
                TreeKind::Channel => {
                    if item.depth > 0 && hidden_parent {
                        continue;
                    }
                    if item.depth == 0 {
                        hidden_parent = false;
                    }
                    let (icon, kind) = match item.channel_kind {
                        channel_kind::TEXT | channel_kind::ANNOUNCEMENT => {
                            ("#", EntryKind::Readable)
                        }
                        channel_kind::VOICE => ("🔊", EntryKind::Voice),
                        channel_kind::STAGE => ("🔊", EntryKind::Other),
                        channel_kind::FORUM | channel_kind::MEDIA => ("≡", EntryKind::Other),
                        _ => ("·", EntryKind::Other),
                    };
                    let forum = channel_kind::is_forum(item.channel_kind);
                    let threads = if forum || channel_kind::has_threads(item.channel_kind) {
                        st.threads_of(item.id, self.open.map(|o| o.channel))
                    } else {
                        Vec::new()
                    };
                    let expanded =
                        (forum || !threads.is_empty()) && self.threads_expanded(&st, item.id);
                    // Kapalı listede okunmamış/bahsetme üst satırda görünsün.
                    let (mut unread, mut mentions) =
                        (st.is_unread(item.id), st.mention_count(item.id));
                    if !expanded {
                        for t in threads.iter().filter(|t| !t.archived) {
                            unread |= st.is_unread(t.id) && !st.is_muted(t.id, now);
                            mentions += st.mention_count(t.id);
                        }
                    }
                    let hint = match (expanded, threads.len()) {
                        (true, _) => String::new(),
                        (false, 0) if forum => " ▸".to_owned(),
                        (false, 0) => String::new(),
                        (false, n) => format!(" ▸{n}"),
                    };
                    out.push(Entry {
                        id: item.id,
                        label: format!("{icon} {}{hint}", item.name),
                        depth: item.depth,
                        kind: if forum {
                            EntryKind::Forum { expanded }
                        } else {
                            kind
                        },
                        unread,
                        mentions,
                        muted: st.is_muted(item.id, now),
                        dim: false,
                    });
                    if expanded {
                        for t in &threads {
                            out.push(Entry {
                                id: t.id,
                                label: format!("└ {}{}", t.name, if t.locked { " 🔒" } else { "" }),
                                depth: item.depth + 1,
                                kind: EntryKind::Readable,
                                unread: st.is_unread(t.id),
                                mentions: st.mention_count(t.id),
                                muted: st.is_muted(t.id, now),
                                dim: t.archived || t.locked,
                            });
                        }
                    }
                    if kind == EntryKind::Voice {
                        for vs in st.voice_members(item.id) {
                            let name = st
                                .user(vs.user_id)
                                .map_or_else(|| "?".to_owned(), |u| st.author_name(Some(gid), u));
                            let flags = match (vs.self_deaf || vs.deaf, vs.self_mute || vs.mute) {
                                (true, _) => " 🎧✕",
                                (false, true) => " 🎤✕",
                                _ => "",
                            };
                            out.push(Entry {
                                id: vs.user_id,
                                label: format!("  {name}{flags}"),
                                depth: item.depth + 1,
                                kind: EntryKind::VoiceMember {
                                    speaking: st.is_speaking(vs.user_id),
                                },
                                unread: false,
                                mentions: 0,
                                muted: false,
                                dim: false,
                            });
                        }
                    }
                }
            }
        }
        out
    }

    /// Mesaj görünümünü `n` satır yukarı kaydırır (eski mesajlara).
    fn scroll_up(&mut self, n: usize) {
        let Some(open) = self.open else { return };
        let width = self.msg_area.get().0;
        let st = self.shared.read();
        let Some(msgs) = st.messages(open.channel).filter(|m| !m.is_empty()) else {
            return;
        };
        let res = view::Res::new(&st, open.guild, &self.tz).with_images(&self.images);
        let mut idx = view::anchor_index(msgs, self.scroll.anchor);
        let mut offset = self.scroll.offset + n;
        loop {
            let h = view::height(msgs, idx, &res, width);
            if offset < h {
                break;
            }
            if idx == 0 {
                // En eskiyi geçtik: ekranın üstü en eski mesajın başına hizalansın.
                let view_h = usize::from(self.msg_area.get().1);
                self.scroll = top_scroll(msgs, &res, width, view_h);
                return;
            }
            offset -= h;
            idx -= 1;
        }
        self.scroll = Scroll {
            anchor: Some(msgs[idx].id),
            offset,
        };
    }

    /// Mesaj görünümünü `n` satır aşağı kaydırır; en alta ulaşınca yapışık moda döner.
    fn scroll_down(&mut self, mut n: usize) {
        let Some(open) = self.open else { return };
        if self.scroll.anchor.is_none() {
            return;
        }
        let width = self.msg_area.get().0;
        let st = self.shared.read();
        let Some(msgs) = st.messages(open.channel).filter(|m| !m.is_empty()) else {
            self.scroll = Scroll::default();
            return;
        };
        let res = view::Res::new(&st, open.guild, &self.tz).with_images(&self.images);
        let mut idx = view::anchor_index(msgs, self.scroll.anchor);
        let mut offset = self.scroll.offset;
        loop {
            if offset >= n {
                offset -= n;
                break;
            }
            n -= offset;
            if idx + 1 >= msgs.len() {
                offset = 0;
                break;
            }
            idx += 1;
            offset = view::height(msgs, idx, &res, width);
        }
        self.scroll = if idx + 1 >= msgs.len() && offset == 0 {
            Scroll::default()
        } else {
            Scroll {
                anchor: Some(msgs[idx].id),
                offset,
            }
        };
    }

    pub fn draw(&self, frame: &mut Frame) {
        view::draw(self, frame);
    }
}

pub(crate) fn unix_now_i64() -> i64 {
    i64::try_from(unix_now()).unwrap_or(i64::MAX)
}

pub(crate) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// En üst konum: ilk satır başlangıç işareti, ardından en eski mesajdan itibaren ekran dolar.
/// İçerik ekrana sığıyorsa en alta yapışık konum.
fn top_scroll(
    msgs: &std::collections::VecDeque<digcord_model::Message>,
    res: &view::Res<'_>,
    width: u16,
    view_h: usize,
) -> Scroll {
    let need = view_h.saturating_sub(1).max(1);
    let mut sum = 0;
    for (idx, m) in msgs.iter().enumerate() {
        sum += view::height(msgs, idx, res, width);
        if sum >= need {
            return if idx + 1 == msgs.len() && sum == need {
                Scroll::default()
            } else {
                Scroll {
                    anchor: Some(m.id),
                    offset: sum - need,
                }
            };
        }
    }
    Scroll::default()
}

#[cfg(test)]
mod perf_synth;
#[cfg(test)]
mod tests;
