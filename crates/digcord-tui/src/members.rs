//! Sağ taraftaki üye paneli: sunucu kanalında gateway'in üye listesi (grup başlıkları ve
//! üyeler), DM/grup DM'de kanalın alıcıları. Store'a salt okunur bakar; `&State` dışarıdan gelir.

use digcord_model::{GroupId, ListItem, ListMember, Presence, Snowflake, channel_kind};
use digcord_store::State;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

use crate::theme::Theme;

/// Çevrimdışı ve DM satırları için işaret.
const DOT: &str = "●";
const DOT_OFFLINE: &str = "○";

/// Genişliğe sığdırır (sığmazsa `…`); geniş karakterler hesaba katılır.
pub(crate) fn fit(s: &str, width: usize) -> String {
    let width_of = |c: char| c.width().unwrap_or(0);
    if s.chars().map(width_of).sum::<usize>() <= width {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        if used + width_of(c) > width.saturating_sub(1) {
            break;
        }
        out.push(c);
        used += width_of(c);
    }
    out.push('…');
    out
}

fn role_color(color: u32) -> Color {
    Color::Rgb(
        ((color >> 16) & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        (color & 0xff) as u8,
    )
}

fn member_line(
    theme: &Theme,
    name: &str,
    color: Option<u32>,
    presence: Option<Presence>,
    width: usize,
) -> Line<'static> {
    let mut name_style = color.map_or_else(Style::new, |c| Style::new().fg(role_color(c)));
    let marker = match presence {
        Some(p) => {
            if p == Presence::Offline {
                name_style = name_style.add_modifier(Modifier::DIM);
            }
            let glyph = if p == Presence::Offline {
                DOT_OFFLINE
            } else {
                DOT
            };
            Span::styled(glyph, theme.presence(p))
        }
        None => Span::raw(" "),
    };
    Line::from(vec![
        marker,
        Span::raw(" "),
        Span::styled(fit(name, width.saturating_sub(2)), name_style),
    ])
}

fn group_title(st: &State, guild: Snowflake, id: GroupId, count: u32) -> String {
    match id {
        GroupId::Online => format!("Çevrimiçi — {count}"),
        GroupId::Offline => format!("Çevrimdışı — {count}"),
        GroupId::Role(r) => {
            let name = st.role(guild, r).map_or("Rol", |r| r.name.as_str());
            format!("{} — {count}", name.to_uppercase())
        }
    }
}

/// Satırlar ve her satırın kullanıcısı (grup başlığı: `None`); aynı uzunlukta.
type Rows = (Vec<Line<'static>>, Vec<Option<Snowflake>>);

fn guild_rows(theme: &Theme, st: &State, guild: Snowflake, width: usize, max_rows: usize) -> Rows {
    let Some(list) = st.member_list(guild) else {
        return (Vec::new(), Vec::new());
    };
    let mut out = Vec::new();
    let mut users = Vec::new();
    for item in list.items() {
        if out.len() >= max_rows {
            break;
        }
        match item {
            None => {}
            Some(ListItem::Group { id, count }) => {
                // Sayı çoğunlukla `groups` özetinde gelir; öğedekini yedek kullan.
                let count = list
                    .groups
                    .iter()
                    .find(|g| g.id == *id)
                    .map(|g| g.count)
                    .or(*count)
                    .unwrap_or(0);
                out.push(Line::styled(
                    fit(&group_title(st, guild, *id, count), width),
                    theme.member_group(),
                ));
                users.push(None);
            }
            Some(ListItem::Member(m)) => {
                let m: &ListMember = m;
                let name = m.nick.as_deref().unwrap_or_else(|| m.user.display_name());
                out.push(member_line(
                    theme,
                    name,
                    st.member_color(guild, &m.roles),
                    Some(m.presence),
                    width,
                ));
                users.push(Some(m.user.id));
            }
        }
    }
    (out, users)
}

/// DM/grup DM: alıcılar (grup DM'de ben de); durum bilgisi olmadığından işaretsiz.
fn dm_rows(theme: &Theme, st: &State, channel: Snowflake, width: usize, max_rows: usize) -> Rows {
    let Some(ch) = st.channel(channel) else {
        return (Vec::new(), Vec::new());
    };
    let mut ids: Vec<Snowflake> = ch.recipient_ids.clone();
    if ch.kind == channel_kind::GROUP_DM
        && let Some(me) = st.me()
        && !ids.contains(&me.id)
    {
        ids.push(me.id);
    }
    let mut out = vec![Line::styled(
        fit(&format!("Üyeler — {}", ids.len()), width),
        theme.member_group(),
    )];
    let mut users = vec![None];
    for id in ids {
        if out.len() >= max_rows {
            break;
        }
        let name = st
            .user(id)
            .map_or_else(|| id.to_string(), |u| u.display_name().to_owned());
        out.push(member_line(theme, &name, None, None, width));
        users.push(Some(id));
    }
    (out, users)
}

/// Panelin iç alanına sığan satırlar. `open`: açık kanal `(guild, kanal)`; yoksa boş.
/// `skip`: üstte atlanan satır (tekerlek); listenin sonunu aşarsa son sayfaya çekilir.
/// Dönüş: satırlar, satırların kullanıcıları ve uygulanan `skip`.
pub(crate) fn rows(
    theme: &Theme,
    st: &State,
    open: Option<(Option<Snowflake>, Snowflake)>,
    width: usize,
    max_rows: usize,
    skip: usize,
) -> (Vec<Line<'static>>, Vec<Option<Snowflake>>, usize) {
    let want = skip.saturating_add(max_rows);
    let (mut lines, mut users) = match open {
        Some((Some(guild), _)) => guild_rows(theme, st, guild, width, want),
        Some((None, channel)) => dm_rows(theme, st, channel, width, want),
        None => (Vec::new(), Vec::new()),
    };
    // Liste istenen aralıktan kısaysa son sayfaya hizala.
    let skip = skip.min(lines.len().saturating_sub(max_rows));
    lines.drain(..skip);
    users.drain(..skip);
    lines.truncate(max_rows);
    users.truncate(max_rows);
    (lines, users, skip)
}
