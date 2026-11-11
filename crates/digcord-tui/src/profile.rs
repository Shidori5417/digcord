//! Kullanıcı profili penceresi (`Mode::Profile`): üye paneline tıklama, SEÇ modunda `p` ya da
//! `:profile <ad>` ile açılır. Ağ yok: istek [`Action::LoadProfile`] olarak denetleyiciye gider,
//! yanıt [`UiEvent::Profile`] ile döner (denetleyici oturum boyunca önbellekler; her kullanıcı
//! eylemi en çok bir istek doğurur, önceden getirme yapılmaz).

use std::cell::Cell;

use digcord_model::{ListItem, Snowflake, UserProfile, parse_user_ref};
use digcord_store::State;
use jiff::tz::TimeZone;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};
use unicode_width::UnicodeWidthChar;

use crate::compose::Mode;
use crate::theme::Theme;
use crate::{Action, App};

/// Pencerenin en geniş hali (sütun).
const MAX_WIDTH: u16 = 64;
const MONTHS: [&str; 12] = [
    "Ocak", "Şubat", "Mart", "Nisan", "Mayıs", "Haziran", "Temmuz", "Ağustos", "Eylül", "Ekim",
    "Kasım", "Aralık",
];

#[derive(Debug)]
pub(crate) enum Load {
    Loading,
    Ready(Box<UserProfile>),
    Failed(String),
}

/// Açık profil penceresi.
#[derive(Debug)]
pub(crate) struct View {
    pub user: Snowflake,
    /// İsteğin sunucusu (`None`: DM). Yanıtla eşleşme ve önbellek anahtarı için.
    pub guild: Option<Snowflake>,
    pub load: Load,
    pub scroll: usize,
    /// Kapanınca dönülecek mod.
    pub from: Mode,
    /// Son çizimde kaydırmanın üst sınırı.
    pub max_scroll: Cell<usize>,
}

impl View {
    pub(crate) fn new(user: Snowflake, guild: Option<Snowflake>, from: Mode) -> Self {
        Self {
            user,
            guild,
            load: Load::Loading,
            scroll: 0,
            from,
            max_scroll: Cell::new(0),
        }
    }
}

impl App {
    /// Profil penceresini "yükleniyor" ile açar ve isteği döner.
    pub(crate) fn open_profile(&mut self, user: Snowflake) -> Option<Action> {
        let guild = self.open.and_then(|o| o.guild);
        let from = match self.mode {
            Mode::Profile => self.profile.as_ref().map_or(Mode::Normal, |v| v.from),
            m => m,
        };
        self.profile = Some(View::new(user, guild, from));
        self.mode = Mode::Profile;
        Some(Action::LoadProfile { user, guild })
    }

    /// `p` (SEÇ): seçili mesajın yazarının profili.
    pub(crate) fn profile_of_selected(&mut self) -> Option<Action> {
        let (open, sel) = (self.open?, self.selected?);
        let author = self
            .shared
            .read()
            .find_message(open.channel, sel)
            .map(|m| m.author.id)?;
        self.open_profile(author)
    }

    /// `:profile <ad>`: kimlik/anma, bilinen kullanıcı adı ya da sunucu takma adı.
    pub(crate) fn profile_command(&mut self, rest: &str) -> Option<Action> {
        let name = rest.trim();
        if name.is_empty() {
            self.notice = Some("Kullanım: :profile <ad | kimlik>".into());
            return None;
        }
        let found = {
            let st = self.shared.read();
            let guild = self.open.and_then(|o| o.guild);
            resolve_user(&st, guild, name)
        };
        match found {
            Some(id) => self.open_profile(id),
            None => {
                self.notice = Some(format!("Kullanıcı bulunamadı: {name}"));
                None
            }
        }
    }

    /// Denetleyiciden gelen yanıt; açık pencerenin (kullanıcı, sunucu) çiftiyle eşleşmezse yok sayılır.
    pub(crate) fn profile_loaded(
        &mut self,
        user: Snowflake,
        guild: Option<Snowflake>,
        result: Result<Box<UserProfile>, String>,
    ) {
        if let Some(v) = self
            .profile
            .as_mut()
            .filter(|v| v.user == user && v.guild == guild)
        {
            v.load = match result {
                Ok(p) => Load::Ready(p),
                Err(e) => Load::Failed(e),
            };
        }
    }

    pub(crate) fn close_profile(&mut self) {
        let back = self.profile.take().map_or(Mode::Normal, |v| v.from);
        self.mode = back;
    }

    pub(crate) fn profile_key(&mut self, key: KeyEvent) -> Option<Action> {
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) || self.profile.is_none() {
            self.close_profile();
            return None;
        }
        let v = self.profile.as_mut()?;
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => v.scroll = v.scroll.saturating_add(1),
            KeyCode::Char('k') | KeyCode::Up => v.scroll = v.scroll.saturating_sub(1),
            KeyCode::PageDown | KeyCode::Char(' ') => v.scroll = v.scroll.saturating_add(8),
            KeyCode::PageUp => v.scroll = v.scroll.saturating_sub(8),
            KeyCode::Char('r') if matches!(v.load, Load::Failed(_)) => {
                v.load = Load::Loading;
                return Some(Action::LoadProfile {
                    user: v.user,
                    guild: v.guild,
                });
            }
            _ => {}
        }
        v.scroll = v.scroll.min(v.max_scroll.get());
        None
    }

    /// Fare tekerleği: pencereyi `delta` satır kaydırır.
    pub(crate) fn profile_scroll(&mut self, delta: isize) {
        if let Some(v) = self.profile.as_mut() {
            v.scroll = v
                .scroll
                .saturating_add_signed(delta)
                .min(v.max_scroll.get());
        }
    }
}

/// Ad çözümleme: kimlik/anma → bilinen kullanıcı (kullanıcı adı ya da görünen ad) → sunucu
/// üye listesindeki takma ad.
pub(crate) fn resolve_user(st: &State, guild: Option<Snowflake>, name: &str) -> Option<Snowflake> {
    if let Some(id) = parse_user_ref(name) {
        return Some(id);
    }
    if let Some(u) = st.find_user_by_name(name) {
        return Some(u.id);
    }
    let want = name.trim_start_matches('@').to_lowercase();
    let list = st.member_list(guild?)?;
    list.items().iter().find_map(|item| match item {
        Some(ListItem::Member(m))
            if m.nick.as_deref().is_some_and(|n| n.to_lowercase() == want) =>
        {
            Some(m.user.id)
        }
        _ => None,
    })
}

/// `2023-05-06T07:08:09+00:00` → `6 Mayıs 2023` (yerel saat dilimiyle).
pub(crate) fn format_date(iso: &str, tz: &TimeZone) -> Option<String> {
    let ts: jiff::Timestamp = iso.parse().ok()?;
    let d = ts.to_zoned(tz.clone()).date();
    let month = MONTHS.get(usize::try_from(d.month() - 1).ok()?)?;
    Some(format!("{} {month} {}", d.day(), d.year()))
}

fn role_style(color: u32) -> Style {
    if color == 0 {
        Style::new()
    } else {
        Style::new().fg(Color::Rgb(
            (color >> 16) as u8,
            (color >> 8) as u8,
            color as u8,
        ))
    }
}

/// Düz metni `width` sütuna sarar (satır sonları korunur; sığmayan uzun sözcük bölünür).
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let w = |c: char| c.width().unwrap_or(0);
    let mut out = Vec::new();
    for para in text.lines() {
        let mut cur = String::new();
        let mut used = 0;
        for word in para.split_whitespace() {
            let word_w: usize = word.chars().map(w).sum();
            if used > 0 && used + 1 + word_w <= width {
                cur.push(' ');
                cur.push_str(word);
                used += 1 + word_w;
                continue;
            }
            if used > 0 {
                out.push(std::mem::take(&mut cur));
                used = 0;
            }
            if word_w <= width {
                cur.push_str(word);
                used = word_w;
                continue;
            }
            // Tek başına sığmayan sözcük: karakter karakter böl.
            for c in word.chars() {
                if used + w(c) > width {
                    out.push(std::mem::take(&mut cur));
                    used = 0;
                }
                cur.push(c);
                used += w(c);
            }
        }
        out.push(cur);
    }
    out
}

fn heading(theme: &Theme, text: &str) -> Line<'static> {
    Line::styled(
        text.to_owned(),
        theme.author().add_modifier(Modifier::UNDERLINED),
    )
}

/// Pencere gövdesinin satırları (kaydırma uygulanmadan). `st` tek okuma korumasıdır.
pub(crate) fn lines(
    v: &View,
    st: &State,
    theme: &Theme,
    tz: &TimeZone,
    width: usize,
) -> Vec<Line<'static>> {
    let known = st.user(v.user);
    let fallback_name = known.map_or_else(|| v.user.to_string(), |u| u.display_name().to_owned());
    let profile = match &v.load {
        Load::Loading => {
            return vec![
                Line::styled(fallback_name, theme.author().add_modifier(Modifier::BOLD)),
                Line::default(),
                Line::styled("Yükleniyor…", theme.dim()),
            ];
        }
        Load::Failed(e) => {
            let mut out = vec![
                Line::styled(fallback_name, theme.author().add_modifier(Modifier::BOLD)),
                Line::default(),
            ];
            out.extend(
                wrap(e, width)
                    .into_iter()
                    .map(|l| Line::styled(l, theme.error())),
            );
            out.push(Line::default());
            out.push(Line::styled("r: yeniden dene", theme.dim()));
            return out;
        }
        Load::Ready(p) => p,
    };

    let username = profile
        .username
        .clone()
        .or_else(|| known.map(|u| u.username.clone()));
    let global = profile
        .global_name
        .clone()
        .or_else(|| known.and_then(|u| u.global_name.clone()));
    let shown = profile
        .guild_nick
        .clone()
        .or_else(|| global.clone())
        .or_else(|| username.clone())
        .unwrap_or(fallback_name);
    let color = v
        .guild
        .and_then(|g| st.member_color(g, &profile.guild_roles));
    let mut head = vec![Span::styled(
        shown,
        color
            .map_or_else(|| theme.author(), role_style)
            .add_modifier(Modifier::BOLD),
    )];
    if profile.bot || known.is_some_and(|u| u.bot) {
        head.push(Span::raw(" "));
        head.push(Span::styled("BOT", theme.bot_badge()));
    }
    let mut out = vec![Line::from(head)];
    let mut sub = Vec::new();
    if let Some(u) = &username {
        sub.push(format!("@{u}"));
    }
    if let Some(p) = &profile.pronouns {
        sub.push(p.clone());
    }
    if !sub.is_empty() {
        out.push(Line::styled(sub.join(" · "), theme.dim()));
    }

    if let Some(bio) = &profile.bio {
        out.push(Line::default());
        out.push(heading(theme, "Hakkında"));
        out.extend(wrap(bio, width).into_iter().map(Line::from));
    }

    out.push(Line::default());
    if let Some(d) = profile
        .joined_at
        .as_deref()
        .and_then(|s| format_date(s, tz))
    {
        out.push(Line::from(vec![
            Span::styled("Sunucuya katılma: ", theme.dim()),
            Span::raw(d),
        ]));
    }
    if let Some(d) = profile
        .premium_since
        .as_deref()
        .and_then(|s| format_date(s, tz))
    {
        out.push(Line::from(vec![
            Span::styled("Nitro: ", theme.dim()),
            Span::raw(format!("{d} tarihinden beri")),
        ]));
    }

    if let Some(g) = v.guild
        && !profile.guild_roles.is_empty()
    {
        let mut roles: Vec<_> = profile
            .guild_roles
            .iter()
            .filter_map(|r| st.role(g, *r))
            .collect();
        roles.sort_by_key(|r| std::cmp::Reverse(r.position));
        if !roles.is_empty() {
            out.push(Line::default());
            out.push(heading(theme, "Roller"));
            // Roller yan yana, genişliğe sarılarak.
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut used = 0;
            for r in roles {
                let w: usize = r.name.chars().map(|c| c.width().unwrap_or(0)).sum();
                if used > 0 && used + 2 + w > width {
                    out.push(Line::from(std::mem::take(&mut spans)));
                    used = 0;
                }
                if used > 0 {
                    spans.push(Span::raw("  "));
                    used += 2;
                }
                spans.push(Span::styled(r.name.clone(), role_style(r.color)));
                used += w;
            }
            out.push(Line::from(spans));
        }
    }

    if !profile.badges.is_empty() {
        out.push(Line::default());
        out.push(heading(theme, "Rozetler"));
        for b in &profile.badges {
            out.push(Line::from(format!("• {}", b.description)));
        }
    }

    if !profile.mutual_guilds.is_empty() {
        out.push(Line::default());
        out.push(heading(
            theme,
            &format!("Ortak sunucular ({})", profile.mutual_guilds.len()),
        ));
        let names: Vec<String> = profile
            .mutual_guilds
            .iter()
            .filter_map(|m| {
                st.guilds()
                    .iter()
                    .find(|g| g.id == m.id)
                    .map(|g| g.name.clone())
            })
            .collect();
        if names.is_empty() {
            // Adlar store'da yok: sadece sayı gösterilir.
            out.push(Line::styled(
                format!("{} sunucu", profile.mutual_guilds.len()),
                theme.dim(),
            ));
        } else {
            out.extend(wrap(&names.join(", "), width).into_iter().map(Line::from));
            let hidden = profile.mutual_guilds.len() - names.len();
            if hidden > 0 {
                out.push(Line::styled(format!("+{hidden} bilinmeyen"), theme.dim()));
            }
        }
    }

    if profile.mutual_friends_count > 0 {
        out.push(Line::default());
        out.push(Line::from(vec![
            Span::styled("Ortak arkadaş: ", theme.dim()),
            Span::raw(profile.mutual_friends_count.to_string()),
        ]));
    }

    if !profile.connected.is_empty() {
        out.push(Line::default());
        out.push(heading(theme, "Bağlı hesaplar"));
        for c in &profile.connected {
            let mut spans = vec![Span::raw(format!("{}: {}", c.kind, c.name))];
            if c.verified {
                spans.push(Span::styled(" ✓", theme.success()));
            }
            out.push(Line::from(spans));
        }
    }
    out
}

pub(crate) fn draw(app: &App, frame: &mut Frame) {
    let Some(v) = &app.profile else { return };
    let t = &app.theme;
    let area = frame.area();
    let st = app.shared.read();
    let width = MAX_WIDTH.min(area.width);
    let inner_w = usize::from(width.saturating_sub(4)).max(1);
    let all = lines(v, &st, t, &app.tz, inner_w);
    drop(st);
    // Kenarlık (2) + altbilgi (1).
    let want = u16::try_from(all.len())
        .unwrap_or(u16::MAX)
        .saturating_add(3);
    let height = want.min(area.height).max(5.min(area.height));
    let popup = centered(area, width, height);
    app.hits.popup.set(popup);
    frame.render_widget(Clear, popup);
    frame.buffer_mut().set_style(popup, t.base());
    let block = Block::bordered()
        .title(" Profil ")
        .border_style(t.border_style(true));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let [body, footer] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(inner);
    let body = Rect {
        x: body.x.saturating_add(1),
        width: body.width.saturating_sub(2),
        ..body
    };
    let max = all.len().saturating_sub(usize::from(body.height));
    v.max_scroll.set(max);
    let scroll = v.scroll.min(max);
    let visible: Vec<Line> = all
        .into_iter()
        .skip(scroll)
        .take(usize::from(body.height))
        .collect();
    frame.render_widget(Paragraph::new(visible), body);
    let hint = if max > 0 {
        "j/k kaydır · Esc/q kapat"
    } else {
        "Esc/q kapat"
    };
    frame.render_widget(Paragraph::new(Line::styled(hint, t.dim())), footer);
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_words_and_newlines() {
        assert_eq!(wrap("bir iki üç dört", 7), vec!["bir iki", "üç dört"]);
        assert_eq!(wrap("a\n\nb", 5), vec!["a", "", "b"]);
        assert_eq!(wrap("abcdefgh", 3), vec!["abc", "def", "gh"]);
        assert_eq!(wrap("", 5), Vec::<String>::new());
    }

    #[test]
    fn formats_turkish_dates_in_given_zone() {
        let utc = TimeZone::UTC;
        assert_eq!(
            format_date("2023-05-06T07:08:09.123000+00:00", &utc).as_deref(),
            Some("6 Mayıs 2023")
        );
        // Yerel güne kayma: UTC 23:30, Istanbul (+3) ertesi gün.
        let ist = TimeZone::get("Europe/Istanbul").expect("dilim");
        assert_eq!(
            format_date("2023-12-31T23:30:00+00:00", &ist).as_deref(),
            Some("1 Ocak 2024")
        );
        assert_eq!(format_date("bozuk", &utc), None);
    }
}
