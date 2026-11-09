//! Çizim: sunucu şeridi, kanal paneli, mesaj görünümü, durum çubuğu, giriş penceresi.

use std::collections::VecDeque;

use digcord_model::{Message, Snowflake, User};
use digcord_store::State;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::layout::{Constraint, Flex, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::render::{self, Resolver};
use crate::{App, EntryKind, Focus, Screen, Status};

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
}

impl<'a> Res<'a> {
    pub(crate) fn new(st: &'a State, guild: Option<Snowflake>, tz: &'a TimeZone) -> Self {
        Self {
            st,
            guild,
            tz,
            me: st.me().map(|u| u.id),
            today: Timestamp::now().to_zoned(tz.clone()).date(),
        }
    }

    fn zoned(&self, ms: i64) -> Option<jiff::Zoned> {
        Timestamp::from_millisecond(ms)
            .ok()
            .map(|t| t.to_zoned(self.tz.clone()))
    }
}

impl Resolver for Res<'_> {
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

/// `idx` mesajının satırları; en eski yüklü mesajın üst boşluğu atılır.
pub(crate) fn lines(
    msgs: &VecDeque<Message>,
    idx: usize,
    res: &Res<'_>,
    width: u16,
) -> Vec<Line<'static>> {
    let prev = idx.checked_sub(1).and_then(|i| msgs.get(i));
    let Some(msg) = msgs.get(idx) else {
        return Vec::new();
    };
    let grouped = render::is_grouped(prev, msg);
    let mut out = render::message_lines(msg, grouped, res, width.max(1));
    if idx == 0 && !grouped && out.first().is_some_and(|l| l.width() == 0) {
        out.remove(0);
    }
    out
}

pub(crate) fn height(msgs: &VecDeque<Message>, idx: usize, res: &Res<'_>, width: u16) -> usize {
    lines(msgs, idx, res, width).len()
}

pub(crate) fn draw(app: &App, frame: &mut Frame) {
    let [main, status] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
    let show_members = main.width >= MEMBERS_MIN_WIDTH;
    let [strip, channels, messages, members] = Layout::horizontal([
        Constraint::Length(STRIP_WIDTH),
        Constraint::Length(CHANNELS_WIDTH),
        Constraint::Fill(1),
        Constraint::Length(if show_members { 20 } else { 0 }),
    ])
    .areas(main);

    let st = app.shared.read();
    draw_strip(app, &st, frame, strip);
    draw_channels(app, &st, frame, channels);
    draw_messages(app, &st, frame, messages);
    if show_members {
        frame.render_widget(Block::bordered().title(" Üyeler "), members);
    }
    drop(st);
    frame.render_widget(Paragraph::new(status_line(app)), status);

    if let Screen::Login { input, error } = &app.screen {
        draw_login(frame, input, error.as_deref());
    }
}

fn panel(title: String, focused: bool) -> Block<'static> {
    let b = Block::bordered().title(title);
    if focused {
        b.border_style(Style::new().cyan())
    } else {
        b
    }
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
    let label = |i: usize, text: String, guild: Option<Snowflake>| {
        let mut style = Style::new();
        if open_guild == Some(guild) {
            style = style.green();
        }
        if i == app.guild_sel {
            style = style.patch(selected_style(focused));
        }
        Line::styled(text, style)
    };
    lines.push(label(0, "DM".to_owned(), None));
    for (i, g) in st.guilds().iter().enumerate() {
        lines.push(label(i + 1, initials(&g.name), Some(g.id)));
    }
    let inner_h = usize::from(area.height.saturating_sub(2));
    let skip = app.guild_sel.saturating_sub(inner_h.saturating_sub(1));
    frame.render_widget(
        Paragraph::new(lines.split_off(skip.min(lines.len()))).block(panel(String::new(), focused)),
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
    let entries = app.entries();
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
                EntryKind::Category { .. } => Style::new().dim().bold(),
                EntryKind::Readable => Style::new(),
                EntryKind::Other => Style::new().dim(),
            };
            if open == Some(e.id) {
                style = style.green();
            }
            if i == app.chan_sel {
                style = style.patch(selected_style(focused));
            }
            let indent = "  ".repeat(usize::from(e.depth));
            Line::styled(format!("{indent}{}", e.label), style)
        })
        .collect();
    let body = if lines.is_empty() {
        Paragraph::new(Line::from("(kanal yok)").dim())
    } else {
        Paragraph::new(lines)
    };
    frame.render_widget(body.block(panel(title, focused)), area);
}

fn draw_messages(app: &App, st: &State, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::Messages && matches!(app.screen, Screen::Main);
    let Some(open) = app.open else {
        app.msg_area
            .set((area.width.saturating_sub(2), area.height.saturating_sub(2)));
        let hint =
            Paragraph::new(Line::from("Bir kanal seç (Tab: panel, j/k: gez, Enter: aç)").dim());
        frame.render_widget(hint.block(panel(" Mesajlar ".to_owned(), focused)), area);
        return;
    };
    let mut title = match open.guild {
        Some(_) => format!(" #{} ", st.channel_title(open.channel)),
        None => format!(" @{} ", st.channel_title(open.channel)),
    };
    if let Some(topic) = st
        .channel(open.channel)
        .and_then(|c| c.topic.as_deref())
        .filter(|t| !t.is_empty())
    {
        title.push_str("— ");
        title.push_str(topic.lines().next().unwrap_or_default());
        title.push(' ');
    }
    if app.scroll.anchor.is_some() {
        title.push_str("[geçmiş] ");
    }
    let block = panel(title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    app.msg_area.set((inner.width, inner.height));

    let Some(msgs) = st.messages(open.channel) else {
        frame.render_widget(Paragraph::new(Line::from("yükleniyor…").dim()), inner);
        return;
    };
    let res = Res::new(st, open.guild, &app.tz);
    let h = usize::from(inner.height);
    let mut out: VecDeque<Line<'static>> = VecDeque::with_capacity(h + 1);
    let mut reached_oldest = msgs.is_empty();
    if !msgs.is_empty() {
        let mut idx = anchor_index(msgs, app.scroll.anchor);
        let mut skip_bottom = app.scroll.offset;
        loop {
            let mut ls = lines(msgs, idx, &res, inner.width);
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
        out.push_front(Line::from(marker).dim().centered());
        // Kısa içerik alta yaslanır (sohbet düzeni).
        while out.len() < h {
            out.push_front(Line::default());
        }
    } else if reached_oldest && !st.history_complete(open.channel) {
        app.want_older.set(true);
    }
    frame.render_widget(Paragraph::new(Vec::from(out)), inner);
}

fn status_line(app: &App) -> Line<'static> {
    let mode = if matches!(app.screen, Screen::Login { .. }) {
        " [GİRİŞ] "
    } else {
        " [NORMAL] "
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
        Status::Online { .. } => Style::new().green(),
        Status::Reconnecting { .. } => Style::new().yellow(),
        Status::Failed(_) => Style::new().red(),
        _ => Style::new(),
    };
    Line::from(vec![mode.bold(), Span::styled(text, style)])
}

fn draw_login(frame: &mut Frame, input: &str, error: Option<&str>) {
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
        Line::from(format!("> {masked}")).bold(),
        Line::from(""),
        match error {
            Some(e) => Line::from(e.to_owned()).red(),
            None => Line::from(""),
        },
        Line::from("Enter: giriş · Esc: çıkış").dim(),
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
