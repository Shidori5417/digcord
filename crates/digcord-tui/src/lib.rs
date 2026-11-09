//! ratatui arayüzü, girdi ve kısayollar.
//!
//! TUI ağ katmanını bilmez: dışarıdan [`UiEvent`] alır, dışarıya [`Action`] verir ve
//! paylaşılan store'u ([`Shared`]) sadece okur.

mod markdown;
mod render;
mod view;

use std::cell::Cell;
use std::collections::HashSet;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, RwLock, RwLockReadGuard};

use digcord_model::{Snowflake, channel_kind};
use digcord_store::{State, TreeKind};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{DefaultTerminal, Frame};

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
#[derive(Debug, PartialEq, Eq)]
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
    mut actions: impl FnMut(Action),
) -> io::Result<()> {
    std::thread::Builder::new()
        .name("digcord-input".into())
        .spawn(move || {
            while let Ok(ev) = event::read() {
                if tx.send(UiEvent::Input(ev)).is_err() {
                    break;
                }
            }
        })?;
    ratatui::run(|terminal| App::new(shared).run(terminal, &events, &mut actions))
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    Category {
        collapsed: bool,
    },
    /// Mesajları okunabilen kanal (metin, duyuru, DM).
    Readable,
    /// Bu aşamada açılmayan kanal (ses, forum...).
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
    open: Option<Open>,
    scroll: Scroll,
    /// Son çizimde mesaj alanının boyutu (kaydırma hesapları için).
    msg_area: Cell<(u16, u16)>,
    chan_offset: Cell<usize>,
    want_older: Cell<bool>,
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
            open: None,
            scroll: Scroll::default(),
            msg_area: Cell::new((0, 0)),
            chan_offset: Cell::new(0),
            want_older: Cell::new(false),
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
            let Ok(ev) = events.recv() else { break };
            if let Some(action) = self.handle(ev) {
                actions(action);
            }
        }
        actions(Action::Quit);
        Ok(())
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
                if let Screen::Login { input, .. } = &mut self.screen {
                    input.extend(text.chars().filter(|c| !c.is_control()));
                }
                None
            }
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
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.kind != KeyEventKind::Press {
            return None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
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

        let half_page = usize::from(self.msg_area.get().1 / 2).max(1);
        match (key.code, ctrl) {
            (KeyCode::Char('q'), false) => self.should_quit = true,
            (KeyCode::Tab, _) => self.focus = self.next_focus(true),
            (KeyCode::BackTab, _) => self.focus = self.next_focus(false),
            (KeyCode::Char('j') | KeyCode::Down, false) => self.move_down(1),
            (KeyCode::Char('k') | KeyCode::Up, false) => self.move_up(1),
            (KeyCode::PageDown, _) | (KeyCode::Char('d'), true) => self.scroll_down(half_page),
            (KeyCode::PageUp, _) | (KeyCode::Char('u'), true) => self.scroll_up(half_page),
            (KeyCode::Char('G') | KeyCode::End, _) => self.scroll = Scroll::default(),
            (KeyCode::Enter | KeyCode::Char('l'), false) => return self.activate(),
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
                        let previous = self.open.map(|o| o.channel).filter(|c| *c != entry.id);
                        self.open = Some(Open {
                            guild,
                            channel: entry.id,
                        });
                        self.scroll = Scroll::default();
                        self.focus = Focus::Messages;
                        Some(Action::OpenChannel {
                            guild,
                            channel: entry.id,
                            previous,
                        })
                    }
                    EntryKind::Other => None,
                }
            }
            Focus::Messages => None,
        }
    }

    /// Seçili sunucunun (veya DM'lerin) kanal paneli satırları.
    fn entries(&self) -> Vec<Entry> {
        let st = self.shared.read();
        let Some(gid) = self.selected_guild(&st) else {
            return st
                .dm_channels()
                .iter()
                .map(|id| Entry {
                    id: *id,
                    label: format!("@ {}", st.channel_title(*id)),
                    depth: 0,
                    kind: EntryKind::Readable,
                })
                .collect();
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
                        channel_kind::VOICE | channel_kind::STAGE => ("🔊", EntryKind::Other),
                        channel_kind::FORUM => ("≡", EntryKind::Other),
                        _ => ("·", EntryKind::Other),
                    };
                    out.push(Entry {
                        id: item.id,
                        label: format!("{icon} {}", item.name),
                        depth: item.depth,
                        kind,
                    });
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
        let res = view::Res::new(&st, open.guild, &self.tz);
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
        let res = view::Res::new(&st, open.guild, &self.tz);
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
mod tests;
