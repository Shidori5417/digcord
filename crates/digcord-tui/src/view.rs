//! Çizim: sunucu şeridi, kanal paneli, mesaj görünümü, durum çubuğu, giriş penceresi.

use std::collections::VecDeque;

use digcord_model::{Message, Snowflake, User};
use digcord_store::State;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::compose::{self, Mode, Target};
use crate::keymap::{Act, Scope};
use crate::media::{Images, SlotState};
use crate::members::fit;
use crate::render::ImageFit;
use crate::render::{self, Resolver};
use crate::theme::Theme;
use crate::{App, EntryKind, Focus, Open, Screen, Status};

/// Mesaj panelinin iç yüksekliği bunun altındaysa başlık satırı çizilmez.
const MIN_HEADER_PANEL_HEIGHT: u16 = 5;

/// Giriş kutusunun en fazla satır sayısı (fazlası kaydırılır).
const MAX_INPUT_ROWS: u16 = 8;

/// Üye paneli bu genişliğin altında gizlenir (TUI_DESIGN "duyarlı düzen").
const MEMBERS_MIN_WIDTH: u16 = 100;
const STRIP_WIDTH: u16 = 6;
const CHANNELS_WIDTH: u16 = 24;

/// Render katmanının store'a bakan çözümleyicisi.
pub(crate) struct Res<'a> {
    st: &'a State,
    guild: Option<Snowflake>,
    tz: &'a TimeZone,
    me: Option<Snowflake>,
    today: jiff::civil::Date,
    images: Option<&'a Images>,
    theme: &'a Theme,
    /// "Yeni mesajlar" çizgisi: kanal açılırken okunmuş son mesaj; ondan yeni ilk mesajın üstüne.
    unread_after: Option<Snowflake>,
    /// Kod bloğu renklendirme teması (`None`: kapalı).
    code_theme: Option<&'a str>,
}

impl<'a> Res<'a> {
    pub(crate) fn new(st: &'a State, guild: Option<Snowflake>, tz: &'a TimeZone) -> Self {
        Self {
            st,
            guild,
            tz,
            me: st.me().map(|u| u.id),
            today: Timestamp::now().to_zoned(tz.clone()).date(),
            images: None,
            theme: &Theme::DEFAULT,
            unread_after: None,
            code_theme: None,
        }
    }

    pub(crate) fn with_unread_after(mut self, id: Option<Snowflake>) -> Self {
        self.unread_after = id;
        self
    }

    /// Renklendirme satır sayısını değiştirmez; bu yüzden yükseklik hesapları bunu atlar.
    pub(crate) fn with_code_theme(mut self, theme: Option<&'a str>) -> Self {
        self.code_theme = theme;
        self
    }

    /// Resim boyutları için (kaydırma hesapları da aynı çözücüyü kullanmalı).
    pub(crate) fn with_images(mut self, images: &'a Images) -> Self {
        self.images = Some(images);
        self
    }

    /// Renkler için (yükseklik hesapları temadan bağımsızdır; onlar varsayılanı kullanır).
    pub(crate) fn with_theme(mut self, theme: &'a Theme) -> Self {
        self.theme = theme;
        self
    }

    fn zoned(&self, ms: i64) -> Option<jiff::Zoned> {
        Timestamp::from_millisecond(ms)
            .ok()
            .map(|t| t.to_zoned(self.tz.clone()))
    }
}

impl Resolver for Res<'_> {
    fn theme(&self) -> &Theme {
        self.theme
    }

    fn code_theme(&self) -> Option<&str> {
        self.code_theme
    }

    fn day(&self, unix_ms: u64) -> Option<jiff::civil::Date> {
        i64::try_from(unix_ms)
            .ok()
            .and_then(|ms| self.zoned(ms))
            .map(|z| z.date())
    }

    fn author_color(&self, user: &User) -> Option<u32> {
        self.st.author_color(self.guild, user.id)
    }

    fn image_fit(&self, w: u32, h: u32, max_cols: u16) -> Option<ImageFit> {
        self.images?.fit(w, h, max_cols)
    }

    fn user_name(&self, id: u64) -> Option<String> {
        self.st
            .user(Snowflake(id))
            .map(|u| self.st.author_name(self.guild, u))
    }

    fn channel_name(&self, id: u64) -> Option<String> {
        self.st.channel(Snowflake(id)).and_then(|c| c.name.clone())
    }

    fn role_name(&self, id: u64) -> Option<String> {
        self.st
            .role(self.guild?, Snowflake(id))
            .map(|r| r.name.clone())
    }

    fn is_me(&self, id: u64) -> bool {
        self.me == Some(Snowflake(id))
    }

    fn author_name(&self, user: &User) -> String {
        self.st.author_name(self.guild, user)
    }

    fn clock(&self, unix_ms: u64) -> String {
        let Some(z) = i64::try_from(unix_ms).ok().and_then(|ms| self.zoned(ms)) else {
            return String::new();
        };
        if z.date() == self.today {
            z.strftime("%H:%M").to_string()
        } else {
            z.strftime("%d.%m.%Y %H:%M").to_string()
        }
    }

    fn is_pending(&self, id: u64) -> bool {
        self.st.is_pending(Snowflake(id))
    }

    fn is_failed(&self, id: u64) -> bool {
        self.st.is_failed(Snowflake(id))
    }

    fn timestamp(&self, unix_secs: i64, format: Option<char>) -> String {
        let Some(z) = unix_secs.checked_mul(1000).and_then(|ms| self.zoned(ms)) else {
            return "?".to_owned();
        };
        let pattern = match format {
            Some('t') => "%H:%M",
            Some('T') => "%H:%M:%S",
            Some('d') => "%d.%m.%Y",
            Some('R') => return relative(unix_secs, Timestamp::now().as_second()),
            _ => "%d.%m.%Y %H:%M",
        };
        z.strftime(pattern).to_string()
    }
}

/// `<t:..:R>` için göreli zaman.
fn relative(then: i64, now: i64) -> String {
    let d = then - now;
    let a = d.unsigned_abs();
    let (n, unit) = match a {
        0..60 => (a, "saniye"),
        60..3_600 => (a / 60, "dakika"),
        3_600..86_400 => (a / 3_600, "saat"),
        86_400..2_592_000 => (a / 86_400, "gün"),
        2_592_000..31_536_000 => (a / 2_592_000, "ay"),
        _ => (a / 31_536_000, "yıl"),
    };
    if d < 0 {
        format!("{n} {unit} önce")
    } else {
        format!("{n} {unit} sonra")
    }
}

/// `anchor` mesajının dizini; yoksa (silinmiş) ondan eski en yakın mesaj; `None` → en yeni.
pub(crate) fn anchor_index(msgs: &VecDeque<Message>, anchor: Option<Snowflake>) -> usize {
    let last = msgs.len().saturating_sub(1);
    match anchor {
        None => last,
        Some(id) => match msgs.binary_search_by_key(&id, |m| m.id) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1).min(last),
        },
    }
}

/// `idx` mesajının satırları ve resim yerleri; en eski yüklü mesajın üst boşluğu atılır.
pub(crate) fn lines(
    msgs: &VecDeque<Message>,
    idx: usize,
    res: &Res<'_>,
    width: u16,
) -> render::Layout {
    let prev = idx.checked_sub(1).and_then(|i| msgs.get(i));
    let Some(msg) = msgs.get(idx) else {
        return render::Layout::default();
    };
    let date = render::date_separator(prev, msg, res, width);
    let new_line = res
        .unread_after
        .is_some_and(|mark| msg.id > mark && prev.is_some_and(|p| p.id <= mark));
    // Ayırıcıdan sonra mesaj başlığı yeniden yazılır (gruplama kesilir).
    let grouped = date.is_none() && !new_line && render::is_grouped(prev, msg);
    let mut out = render::message_layout(msg, grouped, res, width.max(1));
    if date.is_some() || new_line {
        // Ayırıcılar baştaki boşluk satırının hemen altına girer (ayırıcı ile başlık bitişik).
        let at = usize::from(out.lines.first().is_some_and(|l| l.width() == 0));
        let seps: Vec<_> = date
            .into_iter()
            .chain(new_line.then(|| render::new_messages_line(res, width.max(1))))
            .collect();
        for slot in &mut out.images {
            slot.line += seps.len();
        }
        out.lines.splice(at..at, seps);
    }
    if idx == 0 && out.lines.first().is_some_and(|l| l.width() == 0) {
        out.lines.remove(0);
        for slot in &mut out.images {
            slot.line = slot.line.saturating_sub(1);
        }
    }
    out
}

pub(crate) fn height(msgs: &VecDeque<Message>, idx: usize, res: &Res<'_>, width: u16) -> usize {
    lines(msgs, idx, res, width).lines.len()
}

pub(crate) fn draw(app: &App, frame: &mut Frame) {
    let [main, status] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
    let show_members = app.show_members && main.width >= MEMBERS_MIN_WIDTH;
    let [strip, channels, messages, members] = Layout::horizontal([
        Constraint::Length(STRIP_WIDTH),
        Constraint::Length(CHANNELS_WIDTH),
        Constraint::Fill(1),
        Constraint::Length(if show_members { 20 } else { 0 }),
    ])
    .areas(main);

    let full = frame.area();
    frame.buffer_mut().set_style(full, app.theme.base());
    let st = app.shared.read();
    draw_strip(app, &st, frame, strip);
    draw_channels(app, &st, frame, channels);
    draw_messages(app, &st, frame, messages);
    if show_members {
        let block = Block::bordered()
            .title(" Üyeler ")
            .border_style(app.theme.border_style(false));
        let inner = block.inner(members);
        frame.render_widget(block, members);
        let lines = crate::members::rows(
            &app.theme,
            &st,
            app.open.map(|o| (o.guild, o.channel)),
            usize::from(inner.width),
            usize::from(inner.height),
        );
        frame.render_widget(Paragraph::new(lines), inner);
    }
    frame.render_widget(
        Paragraph::new(status_line(app, &st, status.width)).style(app.theme.status()),
        status,
    );
    drop(st);
    // Bildirimler mesaj alanının sağ üstünde, başlık satırının altında.
    let toast_area = messages.inner(Margin::new(1, 1));
    crate::toast::draw(
        &app.toasts,
        &app.theme,
        frame,
        Rect::new(
            toast_area.x,
            toast_area.y.saturating_add(1),
            toast_area.width,
            toast_area.height.saturating_sub(1),
        ),
    );

    if let Screen::Login { input, error } = &app.screen {
        draw_login(frame, &app.theme, input, error.as_deref());
    } else if app.mode == Mode::Settings {
        crate::settings::draw(app, frame);
    } else if app.mode == Mode::Search {
        crate::search::draw(app, frame);
    } else if app.mode == Mode::Help {
        crate::help::draw(app, frame);
    } else if app.mode == Mode::Switcher {
        draw_switcher(app, frame);
    } else if let (Mode::Viewer, Some(v)) = (app.mode, &app.viewer) {
        crate::viewer::draw(v, &app.images, &app.theme, frame);
    }
}

/// `Ctrl+K` hızlı geçiş penceresi.
fn draw_switcher(app: &App, frame: &mut Frame) {
    let area = centered(
        frame.area(),
        64.min(frame.area().width),
        16.min(frame.area().height),
    );
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .title(" Hızlı geçiş (Ctrl+K) ")
        .border_style(app.theme.border_style(true));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [input, list] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    let (rows, cur) = app.command.layout(input.width.saturating_sub(2).max(1));
    let mut line = Line::from(Span::styled("> ", app.theme.dim()));
    line.spans
        .extend(rows.into_iter().next().unwrap_or_default().spans);
    frame.render_widget(Paragraph::new(line), input);
    frame.set_cursor_position((input.x + 2 + cur.1, input.y));

    let matches = app.switcher_matches();
    let sel = app.comp_sel.min(matches.len().saturating_sub(1));
    let lines: Vec<Line> = if matches.is_empty() {
        vec![Line::styled("eşleşme yok", app.theme.dim())]
    } else {
        matches
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let style = if i == sel {
                    Style::new().add_modifier(Modifier::REVERSED)
                } else {
                    Style::new()
                };
                Line::styled(m.label.clone(), style)
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(lines), list);
}

fn panel(title: String, focused: bool, theme: &Theme) -> Block<'static> {
    Block::bordered()
        .title(title)
        .border_style(theme.border_style(focused))
}

fn selected_style(focused: bool) -> Style {
    if focused {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new().add_modifier(Modifier::BOLD)
    }
}

/// Sunucu adından en fazla 3 harflik kısaltma ("Rust Türkiye" → "RT").
fn initials(name: &str) -> String {
    let s: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().find(|c| c.is_alphanumeric()))
        .take(3)
        .flat_map(char::to_uppercase)
        .collect();
    if s.is_empty() { "?".to_owned() } else { s }
}

fn draw_strip(app: &App, st: &State, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::Guilds && matches!(app.screen, Screen::Main);
    let open_guild = app.open.map(|o| o.guild);
    let mut lines = Vec::with_capacity(st.guilds().len() + 1);
    let now = crate::unix_now_i64();
    let inner_w = usize::from(area.width.saturating_sub(2));
    let label =
        |i: usize, text: String, guild: Option<Snowflake>, (unread, mentions): (bool, u32)| {
            let mut style = if unread || mentions > 0 {
                app.theme.unread()
            } else {
                app.theme.dim()
            };
            if open_guild == Some(guild) {
                style = style.patch(app.theme.current());
            }
            if i == app.guild_sel {
                style = style.patch(selected_style(focused));
            }
            let mut spans = vec![Span::styled(text.clone(), style)];
            if mentions > 0 {
                let room = inner_w.saturating_sub(text.chars().count());
                let count = if mentions > 99 {
                    "+".to_owned()
                } else {
                    mentions.to_string()
                };
                if room > count.len() {
                    spans.push(Span::raw(" ".repeat(room - count.len())));
                    spans.push(Span::styled(count, app.theme.badge()));
                }
            }
            Line::from(spans)
        };
    lines.push(label(0, "DM".to_owned(), None, st.dm_unread()));
    for (i, g) in st.guilds().iter().enumerate() {
        lines.push(label(
            i + 1,
            initials(&g.name),
            Some(g.id),
            st.guild_unread(g.id, now),
        ));
    }
    let inner_h = usize::from(area.height.saturating_sub(2));
    let skip = app.guild_sel.saturating_sub(inner_h.saturating_sub(1));
    frame.render_widget(
        Paragraph::new(lines.split_off(skip.min(lines.len()))).block(panel(
            String::new(),
            focused,
            &app.theme,
        )),
        area,
    );
}

fn draw_channels(app: &App, st: &State, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::Channels && matches!(app.screen, Screen::Main);
    let title = match app.selected_guild(st) {
        Some(gid) => st
            .guilds()
            .iter()
            .find(|g| g.id == gid)
            .map_or_else(|| " ? ".to_owned(), |g| format!(" {} ", g.name)),
        None => " Direkt Mesajlar ".to_owned(),
    };
    let entries = app.entries_in(st);
    let inner_h = usize::from(area.height.saturating_sub(2)).max(1);
    // Seçim görünür kalsın.
    let mut offset = app.chan_offset.get();
    if app.chan_sel < offset {
        offset = app.chan_sel;
    } else if app.chan_sel >= offset + inner_h {
        offset = app.chan_sel + 1 - inner_h;
    }
    app.chan_offset.set(offset);

    let open = app.open.map(|o| o.channel);
    let lines: Vec<Line> = entries
        .iter()
        .enumerate()
        .skip(offset)
        .take(inner_h)
        .map(|(i, e)| {
            let mut style = match e.kind {
                EntryKind::Category { .. } => app.theme.dim().add_modifier(Modifier::BOLD),
                _ if e.muted || e.dim => app.theme.dim(),
                EntryKind::Readable if e.unread || e.mentions > 0 => app.theme.unread(),
                EntryKind::Readable => app.theme.channel(),
                EntryKind::Forum { .. } if e.unread || e.mentions > 0 => app.theme.unread(),
                EntryKind::Forum { .. } => app.theme.channel(),
                EntryKind::Voice => app.theme.channel(),
                EntryKind::VoiceMember { speaking: true } => app.theme.speaking(),
                EntryKind::VoiceMember { speaking: false } => app.theme.dim(),
                EntryKind::Other => app.theme.dim(),
            };
            if open == Some(e.id) {
                style = style.patch(app.theme.current());
            }
            if i == app.chan_sel {
                style = style.patch(selected_style(focused));
            }
            let indent = "  ".repeat(usize::from(e.depth));
            let mut spans = vec![Span::styled(format!("{indent}{}", e.label), style)];
            if e.mentions > 0 {
                spans.push(Span::styled(
                    format!(" ({})", e.mentions),
                    app.theme.badge(),
                ));
            }
            Line::from(spans)
        })
        .collect();
    let body = if lines.is_empty() {
        Paragraph::new(Line::styled("(kanal yok)", app.theme.dim()))
    } else {
        Paragraph::new(lines)
    };
    frame.render_widget(body.block(panel(title, focused, &app.theme)), area);
}

fn draw_messages(app: &App, st: &State, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::Messages && matches!(app.screen, Screen::Main);
    let Some(open) = app.open else {
        app.msg_area
            .set((area.width.saturating_sub(2), area.height.saturating_sub(2)));
        let hint = Paragraph::new(Line::styled(
            "Bir kanal seç (Tab: panel, j/k: gez, Enter: aç)",
            app.theme.dim(),
        ));
        frame.render_widget(
            hint.block(panel(" Mesajlar ".to_owned(), focused, &app.theme)),
            area,
        );
        return;
    };
    let block = panel(String::new(), focused, &app.theme);
    let full = block.inner(area);
    frame.render_widget(block, area);
    // Başlık satırı: kanal, konu ve sağda bilgiler. Çok kısa panelde mesajlara yer kalsın diye yok.
    let full = if full.height >= MIN_HEADER_PANEL_HEIGHT {
        let [head, rest] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(full);
        frame.render_widget(Paragraph::new(header_line(app, st, open, head.width)), head);
        rest
    } else {
        full
    };
    let input_h = input_height(app, st, open, full.width);
    let [inner, input] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(input_h)]).areas(full);
    app.msg_area.set((inner.width, inner.height));

    let Some(msgs) = st.messages(open.channel) else {
        frame.render_widget(
            Paragraph::new(Line::styled("yükleniyor…", app.theme.dim())),
            inner,
        );
        draw_input(app, st, open, frame, input);
        return;
    };
    let res = app
        .res(st, open)
        .with_theme(&app.theme)
        .with_code_theme(app.code_theme.as_deref());
    let h = usize::from(inner.height);
    // Görünen resim yerleri: (yer, ilk satırın alttan sırası, kaynak mesaj).
    let mut slots: Vec<(render::ImageSlot, usize, (Snowflake, Snowflake))> = Vec::new();
    let mut out: VecDeque<Line<'static>> = VecDeque::with_capacity(h + 1);
    let mut reached_oldest = msgs.is_empty();
    let selected =
        if app.mode == Mode::Select || matches!(app.mode, Mode::React | Mode::ConfirmDelete(_)) {
            app.selected
        } else {
            None
        };
    app.visible.set(None);
    if !msgs.is_empty() {
        let mut idx = anchor_index(msgs, app.scroll.anchor);
        let bottom_id = msgs[idx].id;
        let mut skip_bottom = app.scroll.offset;
        loop {
            let layout = lines(msgs, idx, &res, inner.width);
            let mut ls = layout.lines;
            let n = ls.len().saturating_sub(skip_bottom);
            for slot in layout.images {
                // Alttan kesilen (görünümün altında kalan) resim çizilmez.
                if slot.line + usize::from(slot.rows) <= n {
                    let first = out.len() + n - 1 - slot.line;
                    slots.push((slot, first, (msgs[idx].channel_id, msgs[idx].id)));
                }
            }
            if selected == Some(msgs[idx].id) {
                for l in &mut ls {
                    l.style = l.style.patch(app.theme.selection_bg());
                }
            }
            app.visible.set(Some((msgs[idx].id, bottom_id)));
            ls.truncate(ls.len().saturating_sub(skip_bottom));
            skip_bottom = 0;
            for l in ls.into_iter().rev() {
                if out.len() == h {
                    break;
                }
                out.push_front(l);
            }
            if out.len() == h {
                break;
            }
            if idx == 0 {
                reached_oldest = true;
                break;
            }
            idx -= 1;
        }
    }
    if reached_oldest && out.len() < h {
        let marker = if st.history_complete(open.channel) {
            "— kanalın başlangıcı —"
        } else {
            app.want_older.set(true);
            "yükleniyor…"
        };
        out.push_front(Line::styled(marker, app.theme.dim()).centered());
        // Kısa içerik alta yaslanır (sohbet düzeni).
        while out.len() < h {
            out.push_front(Line::default());
        }
    } else if reached_oldest && !st.history_complete(open.channel) {
        app.want_older.set(true);
    }
    frame.render_widget(Paragraph::new(Vec::from(out)), inner);
    draw_slots(app, frame, inner, &slots);
    // Tamamlama penceresi mesaj listesinin üstüne çizildiği için en son.
    draw_input(app, st, open, frame, input);
}

/// Resimleri yerlerine çizer. Sadece tamamen görünen resim çizilir (kısmen görünen resim
/// her kaydırmada yeniden kodlanmasın diye); diğerlerinin görünen ilk satırına not düşülür.
fn draw_slots(
    app: &App,
    frame: &mut Frame,
    inner: Rect,
    slots: &[(render::ImageSlot, usize, (Snowflake, Snowflake))],
) {
    let h = usize::from(inner.height);
    for (slot, first, origin) in slots {
        let rows = usize::from(slot.rows);
        // Alttan sıra → ekran satırı: alt satır h-1.
        let last = first + 1 - rows;
        if last >= h {
            continue;
        }
        let col = inner.x + slot.col.min(inner.width);
        let cols = slot.cols.min(inner.width.saturating_sub(slot.col));
        let note = if *first < h {
            let y = inner.y + u16::try_from(h - 1 - first).unwrap_or(0);
            let area = Rect::new(col, y, cols, slot.rows);
            match app.images.draw(frame, area, &slot.url, Some(*origin)) {
                SlotState::Drawn => continue,
                SlotState::Loading => (y, "yükleniyor…"),
                SlotState::Failed => (y, "[resim yüklenemedi]"),
            }
        } else {
            (inner.y, "[resim]")
        };
        let area = Rect::new(col, note.0, cols, 1);
        frame.render_widget(Paragraph::new(Line::styled(note.1, app.theme.dim())), area);
    }
}

fn input_height(app: &App, st: &State, open: Open, width: u16) -> u16 {
    if !st.can_send(open.channel) {
        return 1;
    }
    match app.mode {
        Mode::Command | Mode::React => 2,
        _ => {
            1 + app
                .editor
                .height(width.saturating_sub(2))
                .min(MAX_INPUT_ROWS)
        }
    }
}

fn no_send_text(st: &State, channel: Snowflake) -> &'static str {
    if st.is_thread_locked(channel) {
        "(bu thread kilitli; yazamazsın)"
    } else {
        "(bu kanala yazma iznin yok)"
    }
}

/// Mesaj panelinin altı: ayraç/etiket satırı + giriş; üstünde tamamlama penceresi.
fn draw_input(app: &App, st: &State, open: Open, frame: &mut Frame, area: Rect) {
    if area.height == 0 {
        return;
    }
    if !st.can_send(open.channel) {
        frame.render_widget(
            Paragraph::new(Line::styled(
                no_send_text(st, open.channel),
                app.theme.dim(),
            )),
            area,
        );
        return;
    }
    let [label, body] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    let name = |id| {
        st.find_message(open.channel, id)
            .map(|m| st.author_name(open.guild, &m.author))
            .unwrap_or_else(|| "?".to_owned())
    };
    let label_text = match (app.mode, app.target) {
        (Mode::Command, _) => "komut (Esc: iptal)".to_owned(),
        (Mode::React, _) => "tepki: emoji veya :kısa_ad: (Tab/Enter: seç, Esc: iptal)".to_owned(),
        (_, Target::Reply(id)) => format!("↳ yanıtlanıyor: {} (Esc: iptal)", name(id)),
        (_, Target::Edit(_)) => "✎ düzenleniyor (Enter: kaydet, Esc: iptal)".to_owned(),
        (Mode::Insert, Target::New) => {
            "Enter: gönder · Alt+Enter: yeni satır · Esc: çık".to_owned()
        }
        (Mode::Select, _) => select_hint(&app.keys),
        _ => normal_hint(&app.keys),
    };
    frame.render_widget(
        Paragraph::new(Line::styled(label_text, app.theme.dim())),
        label,
    );

    let (cursor, top) = if matches!(app.mode, Mode::Command | Mode::React) {
        let (rows, cur) = app.command.layout(body.width.saturating_sub(2).max(1));
        let first = rows.into_iter().next().unwrap_or_default();
        let mut l = Line::from(Span::raw(if app.mode == Mode::React { "+ " } else { ": " }));
        l.spans.extend(first.spans);
        frame.render_widget(Paragraph::new(l), body);
        (Some(cur), body.y)
    } else {
        let (rows, cur) = app.editor.layout(body.width.saturating_sub(2).max(1));
        let skip = cur.0.saturating_sub(body.height.saturating_sub(1));
        let lines: Vec<Line> = rows
            .into_iter()
            .enumerate()
            .skip(usize::from(skip))
            .map(|(i, mut l)| {
                let prefix = if i == 0 { "> " } else { "  " };
                l.spans.insert(0, Span::styled(prefix, app.theme.dim()));
                l
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), body);
        let cur = (cur.0.saturating_sub(skip), cur.1);
        (app.mode == Mode::Insert)
            .then_some(cur)
            .map_or((None, body.y), |c| (Some(c), body.y))
    };
    if let Some((row, col)) = cursor {
        let x = (body.x + 2 + col).min(body.right().saturating_sub(1));
        frame.set_cursor_position((x, top + row));
    }

    let candidates = app.candidates_in(st);
    if !candidates.is_empty() {
        let n = u16::try_from(candidates.len()).unwrap_or(u16::MAX);
        let h = n.min(area.y.saturating_sub(1));
        let w = area.width.min(48);
        let popup = Rect::new(area.x, area.y.saturating_sub(h), w, h);
        frame.render_widget(Clear, popup);
        let sel = app.comp_sel.min(candidates.len() - 1);
        let lines: Vec<Line> = candidates
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                let style = if i == sel {
                    Style::new().add_modifier(Modifier::REVERSED)
                } else {
                    app.theme.popup_bg()
                };
                Line::styled(format!(" {} ", c.label), style)
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), popup);
    }
}

/// NORMAL modun alt ipucu; tuşlar canlı kısayol tablosundan gelir.
pub(crate) fn normal_hint(keys: &crate::Keymap) -> String {
    format!(
        "{}: yaz · {}: seç · {}: yardım · :upload <yol>",
        keys.first_key(Scope::Normal, Act::InsertMode),
        keys.first_key(Scope::Normal, Act::SelectMode),
        keys.first_key(Scope::Normal, Act::Help),
    )
}

/// SEÇ modunun alt ipucu.
pub(crate) fn select_hint(keys: &crate::Keymap) -> String {
    let k = |act| keys.first_key(Scope::Select, act);
    let (down, up, del) = (k(Act::MoveDown), k(Act::MoveUp), k(Act::Delete));
    format!(
        "{down}/{up} seç · {} yanıt · {} düzenle · {del}{del} sil · {} tepki · {} kopyala · {} yardım · {}",
        k(Act::Reply),
        k(Act::Edit),
        k(Act::React),
        k(Act::Copy),
        k(Act::Help),
        k(Act::Back),
    )
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// Span'ları `max` sütuna kısaltır; kesilirse sona `…` konur.
fn clip_spans(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
    use unicode_width::UnicodeWidthChar;
    if spans_width(&spans) <= max {
        return spans;
    }
    let mut out = Vec::with_capacity(spans.len());
    let mut left = max.saturating_sub(1);
    for span in spans {
        let mut text = String::new();
        let mut done = false;
        for c in span.content.chars() {
            let w = c.width().unwrap_or(0);
            if w > left {
                done = true;
                break;
            }
            left -= w;
            text.push(c);
        }
        if done {
            text.push('…');
            out.push(Span::styled(text, span.style));
            return out;
        }
        out.push(Span::styled(text, span.style));
    }
    out
}

/// Gecikme göstergesi: eşiklere göre renk.
fn latency_span(theme: &Theme, d: std::time::Duration) -> Span<'static> {
    let ms = d.as_millis();
    let style = match ms {
        0..150 => theme.success(),
        150..400 => theme.warning(),
        _ => theme.error(),
    };
    Span::styled(format!("{ms} ms"), style)
}

/// `HH:MM` (yerel saat dilimi).
fn clock_text(app: &App) -> String {
    app.now()
        .to_zoned(app.tz.clone())
        .strftime("%H:%M")
        .to_string()
}

fn status_line(app: &App, st: &State, width: u16) -> Line<'static> {
    let width = usize::from(width);
    let mode = if matches!(app.screen, Screen::Login { .. }) {
        " [GİRİŞ] "
    } else {
        compose::mode_label(app.mode)
    };
    let text = match &app.status {
        Status::Starting => "Digcord · başlatılıyor…".to_owned(),
        Status::Connecting => "Digcord · bağlanıyor…".to_owned(),
        Status::Online {
            user,
            guilds,
            channels,
        } => format!("Digcord · {user} · bağlı · {guilds} sunucu · {channels} kanal"),
        Status::Reconnecting { secs, reason } => {
            format!("Digcord · yeniden bağlanılıyor ({secs} sn): {reason}")
        }
        Status::Failed(reason) => format!("Digcord · hata: {reason}"),
    };
    let style = match app.status {
        Status::Online { .. } => app.theme.success(),
        Status::Reconnecting { .. } => app.theme.warning(),
        Status::Failed(_) => app.theme.error(),
        _ => Style::new(),
    };
    // Çekirdek (kip + bağlantı durumu) her zaman önceliklidir; ses, bildirim ve "yazıyor" sığdığı
    // kadar eklenir; sağdaki gecikme ve saat çekirdekle birlikte sığmazsa önce gecikme düşer.
    let core = vec![
        Span::styled(mode, Style::new().add_modifier(Modifier::BOLD)),
        Span::styled(text, style),
    ];
    let mut extra = Vec::new();
    if let Some(voice) = voice_status(app, st) {
        extra.push(voice);
    }
    if let Mode::ConfirmDelete(_) = app.mode {
        extra.push(Span::styled(
            "  Mesaj silinsin mi? (e/h)",
            app.theme.warning().add_modifier(Modifier::BOLD),
        ));
    } else if let Some(n) = &app.notice {
        extra.push(Span::styled(format!("  {n}"), app.theme.warning()));
    } else if let Some(open) = app.open {
        let names = st.typing_names(open.channel, crate::unix_now());
        if !names.is_empty() {
            let who = match names.len() {
                1 => format!("{} yazıyor…", names[0]),
                2 => format!("{} ve {} yazıyor…", names[0], names[1]),
                n => format!("{n} kişi yazıyor…"),
            };
            extra.push(Span::styled(format!("  {who}"), app.theme.dim()));
        }
    }

    let clock = Span::styled(clock_text(app), app.theme.dim());
    let latency = app
        .latency
        .filter(|_| matches!(app.status, Status::Online { .. }))
        .map(|d| latency_span(&app.theme, d));
    let core_w = spans_width(&core);
    // Sağ taraf adayları, en zengin olandan en yalına; çekirdek + 1 boşluk ile sığan ilki.
    let mut candidates: Vec<Vec<Span<'static>>> = Vec::new();
    if let Some(l) = latency {
        candidates.push(vec![l, Span::raw(" · "), clock.clone(), Span::raw(" ")]);
    }
    candidates.push(vec![clock, Span::raw(" ")]);
    let right = candidates
        .into_iter()
        .find(|r| core_w + 1 + spans_width(r) <= width)
        .unwrap_or_default();
    let right_w = spans_width(&right);

    let mut left = core;
    left.extend(extra);
    let avail = width.saturating_sub(right_w + usize::from(right_w > 0));
    let mut left = clip_spans(left, avail);
    let pad = width.saturating_sub(spans_width(&left) + right_w);
    left.push(Span::raw(" ".repeat(pad)));
    left.extend(right);
    Line::from(left)
}

/// Mesaj paneli başlık satırı: `# kanal — konu` solda; sağda (öncelik sırasıyla) `[geçmiş]`,
/// ses simgesi, çevrimiçi/toplam üye, sabitlenen sayısı. Dar genişlikte en düşük öncelikli
/// parçalar önce düşer; sonra konu, en son kanal adı kısalır.
pub(crate) fn header_line(app: &App, st: &State, open: Open, width: u16) -> Line<'static> {
    use digcord_model::channel_kind;
    let width = usize::from(width);
    let ch = st.channel(open.channel);
    let icon = match ch.map(|c| c.kind) {
        _ if open.guild.is_none() => "@",
        Some(k) if channel_kind::is_thread(k) => "└",
        Some(channel_kind::VOICE | channel_kind::STAGE) => "🔊",
        Some(channel_kind::FORUM | channel_kind::MEDIA) => "≡",
        _ => "#",
    };
    let name = format!("{icon} {}", st.channel_title(open.channel));
    let topic = ch
        .and_then(|c| c.topic.as_deref())
        .and_then(|t| t.lines().next())
        .map(str::trim)
        .filter(|t| !t.is_empty());

    // (öncelik, metin, stil): küçük sayı önce düşer.
    let theme = &app.theme;
    let mut right: Vec<(u8, String, Style)> = Vec::new();
    if let Some(n) = app.pin_counts.get(&open.channel) {
        right.push((1, format!("📌 {n}"), theme.dim()));
    }
    if let Some(list) = open.guild.and_then(|g| st.member_list(g))
        && list.member_count > 0
    {
        right.push((
            2,
            format!("● {}/{}", list.online_count, list.member_count),
            theme.presence(digcord_model::Presence::Online),
        ));
    }
    if st.voice_call().is_some_and(|c| {
        c.guild == open.guild && (open.guild.is_some() || c.channel == open.channel)
    }) {
        right.push((3, "🔊".to_owned(), theme.success()));
    }
    if app.scroll.anchor.is_some() {
        right.push((4, "[geçmiş]".to_owned(), theme.warning()));
    }

    let total = |r: &[(u8, String, Style)]| {
        r.iter().map(|(_, t, _)| t.width()).sum::<usize>() + 2 * r.len().saturating_sub(1)
    };
    // Kanal adına en az bu kadar yer kalsın; yoksa en düşük öncelikli parça düşer.
    let min_left = name.width().min(12);
    while !right.is_empty() && min_left + 1 + total(&right) > width {
        if let Some(i) = right
            .iter()
            .enumerate()
            .min_by_key(|(_, (p, _, _))| *p)
            .map(|(i, _)| i)
        {
            right.remove(i);
        }
    }
    let right_w = total(&right);
    let avail = width.saturating_sub(right_w + usize::from(right_w > 0));

    let mut spans = Vec::new();
    let name_fit = fit(&name, avail);
    let mut used = name_fit.width();
    spans.push(Span::styled(
        name_fit,
        Style::new().add_modifier(Modifier::BOLD),
    ));
    if let Some(topic) = topic {
        let room = avail.saturating_sub(used);
        // " — " + en az birkaç harf yoksa konu hiç gösterilmez.
        if room >= 3 + 4 {
            let t = format!(" — {}", fit(topic, room - 3));
            used += t.width();
            spans.push(Span::styled(t, theme.dim()));
        }
    }
    if right_w > 0 {
        spans.push(Span::raw(" ".repeat(width.saturating_sub(used + right_w))));
        for (i, (_, text, style)) in right.into_iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(text, style));
        }
    }
    Line::from(spans)
}

/// `  🔊 Ses · bağlı (uçtan uca) 🎤✕` — ses kanalındaysak.
fn voice_status(app: &App, st: &State) -> Option<Span<'static>> {
    use digcord_store::VoicePhase;
    let call = st.voice_call()?;
    let (phase, style) = match call.phase {
        VoicePhase::Joining => ("katılınıyor…", app.theme.warning()),
        VoicePhase::Connecting => ("bağlanıyor…", app.theme.warning()),
        VoicePhase::Connected { e2ee: true } => ("bağlı (uçtan uca)", app.theme.success()),
        VoicePhase::Connected { e2ee: false } => ("bağlı", app.theme.success()),
    };
    let flags = match (call.self_deaf, call.self_mute) {
        (true, _) => " 🎧✕",
        (false, true) => " 🎤✕",
        _ => "",
    };
    Some(Span::styled(
        format!("  🔊 {} · {phase}{flags}", st.channel_title(call.channel)),
        style,
    ))
}

fn draw_login(frame: &mut Frame, theme: &Theme, input: &str, error: Option<&str>) {
    let area = centered(frame.area(), 60, 9);
    frame.render_widget(Clear, area);
    let block = Block::bordered().title(" Digcord — Giriş ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let field_width = usize::from(inner.width.saturating_sub(4));
    let count = input.chars().count();
    // Token asla gösterilmez; sadece uzunluğu kadar nokta.
    let masked: String = "•".repeat(count.min(field_width));
    let lines = vec![
        Line::from("Discord token'ını yapıştır (yan hesap önerilir):"),
        Line::from(""),
        Line::styled(
            format!("> {masked}"),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Line::from(""),
        match error {
            Some(e) => Line::styled(e.to_owned(), theme.error()),
            None => Line::from(""),
        },
        Line::styled("Enter: giriş · Esc: çıkış", theme.dim()),
    ];
    frame.render_widget(Paragraph::new(lines), inner.inner(Margin::new(1, 0)));
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [v] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [h] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(v);
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_and_relative() {
        assert_eq!(initials("Rust Türkiye"), "RT");
        assert_eq!(initials("çok güzel bir sunucu"), "ÇGB");
        assert_eq!(initials("  "), "?");
        assert_eq!(relative(0, 3_700), "1 saat önce");
        assert_eq!(relative(120, 0), "2 dakika sonra");
    }
}
