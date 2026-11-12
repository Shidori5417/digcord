//! Fare: sol tıklama ve tekerlek. Tıklama hedefleri son çizimde kaydedilen panel dikdörtgenlerinden
//! ([`Hits`]) bulunur; çizim her karede bunları yeniden yazar. Sürükleme ve diğer düğmeler yok
//! sayılır; klavye davranışı değişmez. Ağ/disk yok.

use std::cell::{Cell, RefCell};

use digcord_model::Snowflake;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};

use crate::compose::Mode;
use crate::{Action, App, Focus, Screen};

/// Tekerlek bir çentiğinde mesaj alanının kaydığı satır sayısı.
const WHEEL_LINES: usize = 3;

/// Son çizimin tıklanabilir bölgeleri (iç alanlar: kenarlıksız). Çizim `&App` ile çalıştığı için
/// `Cell`/`RefCell`; her karede [`Hits::reset`] ile sıfırlanır.
#[derive(Debug, Default)]
pub(crate) struct Hits {
    /// Sunucu şeridi iç alanı ve en üstte gizli kalan satır sayısı.
    pub strip: Cell<Rect>,
    pub strip_skip: Cell<usize>,
    /// Şeritte bir sunucunun kapladığı satır sayısı (simgeli şeritte 2; 0 = ayarlanmadı).
    pub strip_row_h: Cell<u16>,
    /// Kanal paneli iç alanı (kaydırma `App::chan_offset`'te).
    pub channels: Cell<Rect>,
    /// Mesaj panelinin tamamı (kenarlık dahil).
    pub messages_panel: Cell<Rect>,
    /// Mesaj listesi alanı ve her satırın mesajı (`None`: boş satır/işaret).
    pub messages: Cell<Rect>,
    pub message_rows: RefCell<Vec<Option<Snowflake>>>,
    /// Üye paneli iç alanı ve satırların kullanıcıları (`None`: grup başlığı).
    pub members: Cell<Rect>,
    pub member_rows: RefCell<Vec<Option<Snowflake>>>,
    /// Tam ekran/ortadaki pencerenin seçilebilir liste alanı ve ilk satırın dizini.
    pub overlay_list: Cell<Rect>,
    pub overlay_offset: Cell<usize>,
    /// Profil penceresinin dikdörtgeni.
    pub popup: Cell<Rect>,
}

impl Hits {
    pub(crate) fn reset(&self) {
        self.strip.set(Rect::default());
        self.strip_skip.set(0);
        self.strip_row_h.set(1);
        self.channels.set(Rect::default());
        self.messages_panel.set(Rect::default());
        self.messages.set(Rect::default());
        self.message_rows.borrow_mut().clear();
        self.members.set(Rect::default());
        self.member_rows.borrow_mut().clear();
        self.overlay_list.set(Rect::default());
        self.overlay_offset.set(0);
        self.popup.set(Rect::default());
    }
}

fn contains(r: Rect, x: u16, y: u16) -> bool {
    r.contains(Position::new(x, y))
}

/// `r` içinde `y` satırının sırası.
fn row(r: Rect, y: u16) -> usize {
    usize::from(y - r.y)
}

fn enter() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
}

impl App {
    pub(crate) fn handle_mouse(&mut self, ev: MouseEvent) -> Option<Action> {
        if !self.mouse || !matches!(self.screen, Screen::Main) {
            return None;
        }
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => self.click(ev.column, ev.row),
            MouseEventKind::ScrollUp => {
                self.wheel(ev.column, ev.row, false);
                None
            }
            MouseEventKind::ScrollDown => {
                self.wheel(ev.column, ev.row, true);
                None
            }
            // Sürükleme, bırakma, orta/sağ düğme, hareket: yok sayılır.
            _ => None,
        }
    }

    fn click(&mut self, x: u16, y: u16) -> Option<Action> {
        let previous = self.click_row.take();
        match self.mode {
            Mode::Normal | Mode::Select => self.click_main(x, y),
            Mode::Switcher => {
                let idx = self.overlay_row(x, y)?;
                let count = self.switcher_matches().len();
                if idx >= count {
                    return None;
                }
                self.comp_sel = idx;
                if previous == Some(idx) {
                    return self.switcher_key(enter());
                }
                self.click_row = Some(idx);
                None
            }
            Mode::Settings => {
                let idx = self.overlay_row(x, y)?;
                if idx >= self.settings.items.len() {
                    return None;
                }
                self.settings.sel = idx;
                if previous == Some(idx) {
                    return self.settings_key(enter());
                }
                self.click_row = Some(idx);
                None
            }
            Mode::Search => {
                let idx = self.overlay_row(x, y)?;
                let view = self.search.as_mut()?;
                if idx >= view.hits.len() {
                    return None;
                }
                view.sel = idx;
                if previous == Some(idx) {
                    return self.search_key(enter());
                }
                self.click_row = Some(idx);
                None
            }
            Mode::Profile => {
                // Pencere dışına tıklamak kapatır.
                if !contains(self.hits.popup.get(), x, y) {
                    self.close_profile();
                }
                None
            }
            // Yazarken/komut girerken tıklama bir şey bozmasın (taslak kaybı).
            Mode::Insert
            | Mode::Command
            | Mode::React
            | Mode::ConfirmDelete(_)
            | Mode::Viewer
            | Mode::Help
            | Mode::Diag => None,
        }
    }

    /// Katman listesinde tıklanan satırın dizini (kaydırma dahil).
    fn overlay_row(&self, x: u16, y: u16) -> Option<usize> {
        let list = self.hits.overlay_list.get();
        contains(list, x, y).then(|| self.hits.overlay_offset.get() + row(list, y))
    }

    fn click_main(&mut self, x: u16, y: u16) -> Option<Action> {
        let strip = self.hits.strip.get();
        if contains(strip, x, y) {
            let idx = self.hits.strip_skip.get()
                + row(strip, y) / usize::from(self.hits.strip_row_h.get().max(1));
            // 0 = DM, i = i. sunucu.
            if idx <= self.shared.read().guilds().len() {
                self.guild_sel = idx;
                self.chan_sel = 0;
                self.focus = Focus::Channels;
            }
            return None;
        }
        let channels = self.hits.channels.get();
        if contains(channels, x, y) {
            let idx = self.chan_offset.get() + row(channels, y);
            if idx >= self.entries().len() {
                return None;
            }
            self.focus = Focus::Channels;
            self.chan_sel = idx;
            return self.activate();
        }
        let members = self.hits.members.get();
        if contains(members, x, y) {
            let user = self
                .hits
                .member_rows
                .borrow()
                .get(row(members, y))
                .copied()
                .flatten()?;
            return self.open_profile(user);
        }
        if contains(self.hits.messages_panel.get(), x, y) {
            self.focus = Focus::Messages;
            let list = self.hits.messages.get();
            if contains(list, x, y)
                && let Some(id) = self
                    .hits
                    .message_rows
                    .borrow()
                    .get(row(list, y))
                    .copied()
                    .flatten()
            {
                self.selected = Some(id);
                self.mode = Mode::Select;
                self.pending_d = false;
            }
        }
        None
    }

    /// Tekerlek: imlecin altındaki paneli kaydırır (`down`: aşağı).
    fn wheel(&mut self, x: u16, y: u16, down: bool) {
        self.click_row = None;
        let sign = if down { 1 } else { -1 };
        match self.mode {
            Mode::Switcher => {
                let count = self.switcher_matches().len();
                self.comp_sel = self
                    .comp_sel
                    .saturating_add_signed(sign)
                    .min(count.saturating_sub(1));
            }
            Mode::Settings => {
                let last = self.settings.items.len().saturating_sub(1);
                self.settings.sel = self.settings.sel.saturating_add_signed(sign).min(last);
            }
            Mode::Search => {
                if let Some(v) = self.search.as_mut() {
                    let last = v.hits.len().saturating_sub(1);
                    v.sel = v.sel.saturating_add_signed(sign).min(last);
                }
            }
            Mode::Help => {
                let step = WHEEL_LINES.cast_signed() * sign;
                self.help_scroll = self
                    .help_scroll
                    .saturating_add_signed(step)
                    .min(self.help_max_scroll());
            }
            Mode::Diag => {
                let step = WHEEL_LINES.cast_signed() * sign;
                self.diag_scroll = self
                    .diag_scroll
                    .saturating_add_signed(step)
                    .min(self.diag_max_scroll());
            }
            Mode::Profile => self.profile_scroll(WHEEL_LINES.cast_signed() * sign),
            Mode::Normal | Mode::Select | Mode::Insert => self.wheel_main(x, y, down),
            Mode::Command | Mode::React | Mode::ConfirmDelete(_) | Mode::Viewer => {}
        }
    }

    fn wheel_main(&mut self, x: u16, y: u16, down: bool) {
        let sign: isize = if down { 1 } else { -1 };
        if contains(self.hits.strip.get(), x, y) {
            let count = self.shared.read().guilds().len() + 1;
            self.guild_sel = self.guild_sel.saturating_add_signed(sign).min(count - 1);
            self.chan_sel = 0;
        } else if contains(self.hits.channels.get(), x, y) {
            let count = self.entries().len();
            self.chan_sel = self
                .chan_sel
                .saturating_add_signed(sign)
                .min(count.saturating_sub(1));
        } else if contains(self.hits.members.get(), x, y) {
            let off = self.members_offset.get();
            self.members_offset.set(off.saturating_add_signed(sign));
        } else if contains(self.hits.messages_panel.get(), x, y) {
            if down {
                self.scroll_down(WHEEL_LINES);
            } else {
                self.scroll_up(WHEEL_LINES);
            }
        }
    }
}
