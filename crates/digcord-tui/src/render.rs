//! Bir Discord mesajını sarılmış, biçimlendirilmiş ratatui satırlarına çevirir.
//!
//! Saf fonksiyonlar: ağ/disk yok. Dış dünya (isim çözümleme, saat biçimi) `Resolver` ile gelir.

use digcord_model::{Message, User, message_kind as kind};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::markdown::{self, Block, BlockKind, Span as Md};

/// Gruplama penceresi: aynı yazarın bu süre içindeki mesajları başlıksız birleşir.
const GROUP_WINDOW_MS: u64 = 7 * 60 * 1000;
/// Mesaj içeriğinin girintisi (boşluk sayısı).
const INDENT: usize = 2;
const EDITED: &str = " (düzenlendi)";
/// Yanıt (reply) mesaj türü.
const KIND_REPLY: u8 = 19;

/// Çizimin dış dünyadan ihtiyaç duyduğu her şey.
pub trait Resolver {
    /// `<@id>` için kullanıcı adı.
    fn user_name(&self, id: u64) -> Option<String>;
    /// `<#id>` için kanal adı.
    fn channel_name(&self, id: u64) -> Option<String>;
    /// `<@&id>` için rol adı.
    fn role_name(&self, id: u64) -> Option<String>;
    /// Benim kimliğim mi (bahsetmeleri vurgula).
    fn is_me(&self, id: u64) -> bool;
    /// Takma ad > global ad > kullanıcı adı (çağıran karar verir).
    fn author_name(&self, user: &User) -> String;
    /// Mesaj başlığındaki saat metni, örn. "14:02".
    fn clock(&self, unix_ms: u64) -> String;
    /// Discord `<t:unix:F>` zaman damgası metni.
    fn timestamp(&self, unix_secs: i64, format: Option<char>) -> String;
}

/// Aynı yazar, öncekinden 7 dakikadan az sonra ve yanıt değil → başlık satırı yok.
pub fn is_grouped(prev: Option<&Message>, msg: &Message) -> bool {
    let Some(prev) = prev else { return false };
    !prev.is_system()
        && !msg.is_system()
        && prev.author.id == msg.author.id
        && msg.referenced_message.is_none()
        && msg.kind != KIND_REPLY
        && msg.timestamp_ms().abs_diff(prev.timestamp_ms()) < GROUP_WINDOW_MS
}

/// Bir mesajın tüm terminal satırları (`width` >= 1; hiçbir satır `width`'ten geniş olmaz).
///
/// Gruplanmamış mesajın ilk satırı boş ayırıcıdır; çağıran ilk mesajda atar.
pub fn message_lines(
    msg: &Message,
    grouped: bool,
    r: &dyn Resolver,
    width: u16,
) -> Vec<Line<'static>> {
    let width = usize::from(width).max(1);
    let mut out = Vec::new();

    if let Some(text) = system_text(msg, r) {
        out.push(Line::default());
        out.push(fit_line(
            vec![
                ("→ ".into(), dim()),
                (text, Style::new().add_modifier(Modifier::ITALIC)),
                ("  ".into(), Style::new()),
                (r.clock(msg.timestamp_ms()), dim()),
            ],
            width,
        ));
        return out;
    }

    if !grouped {
        out.push(Line::default());
        if let Some(reply) = &msg.referenced_message {
            out.push(reply_line(reply, r, width));
        } else if msg.kind == KIND_REPLY {
            out.push(fit_line(
                vec![("  ↳ (silinmiş mesaj)".into(), dim())],
                width,
            ));
        }
        out.push(fit_line(
            vec![
                (
                    r.author_name(&msg.author),
                    Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
                ("  ".into(), Style::new()),
                (r.clock(msg.timestamp_ms()), dim()),
            ],
            width,
        ));
    }

    let mut content = content_lines(msg, r, width);
    let extras = extra_lines(msg, width);
    if content.is_empty() && msg.attachments.is_empty() && extras.is_empty() {
        content.push(fit_line(vec![("  (boş mesaj)".into(), dim())], width));
    }

    if msg.is_edited() {
        match content.last_mut() {
            Some(last) if last.width() + EDITED.width() <= width => {
                last.push_span(Span::styled(EDITED, dim()));
            }
            _ => {
                let text = format!("{}{}", " ".repeat(indent_for(width)), EDITED.trim_start());
                content.push(fit_line(vec![(text, dim())], width));
            }
        }
    }
    out.extend(content);
    out.extend(extras);

    for a in &msg.attachments {
        let info = match (a.width, a.height) {
            (Some(w), Some(h)) => format!("{w}×{h}"),
            _ => human_size(a.size),
        };
        let text = format!("  [ek: {} {info}]", a.filename);
        out.push(fit_line(vec![(text, dim())], width));
    }
    out
}

/// Sistem mesajının tek satırlık metni; normal mesajlarda `None`.
fn system_text(msg: &Message, r: &dyn Resolver) -> Option<String> {
    if !msg.is_system() {
        return None;
    }
    let who = r.author_name(&msg.author);
    let target = msg
        .mentions
        .first()
        .map(|u| r.author_name(u))
        .unwrap_or_else(|| "birini".into());
    Some(match msg.kind {
        kind::CALL => {
            let ended = msg
                .call
                .as_ref()
                .is_some_and(|c| c.ended_timestamp.is_some());
            if ended {
                format!("{who} bir arama başlattı (bitti)")
            } else {
                format!("{who} bir arama başlattı")
            }
        }
        kind::RECIPIENT_ADD => format!("{who}, {target} kişisini gruba ekledi"),
        kind::RECIPIENT_REMOVE if msg.mentions.first().is_none_or(|u| u.id == msg.author.id) => {
            format!("{who} gruptan ayrıldı")
        }
        kind::RECIPIENT_REMOVE => format!("{who}, {target} kişisini gruptan çıkardı"),
        kind::CHANNEL_NAME_CHANGE => format!("{who} adı değiştirdi: {}", msg.content),
        kind::CHANNEL_ICON_CHANGE => format!("{who} grup simgesini değiştirdi"),
        kind::PINNED => format!("{who} bir mesajı sabitledi"),
        kind::MEMBER_JOIN => format!("{who} sunucuya katıldı"),
        kind::BOOST => format!("{who} sunucuya takviye yaptı"),
        kind::BOOST_TIER_1 | kind::BOOST_TIER_2 | kind::BOOST_TIER_3 => {
            format!("{who} sunucuya takviye yaptı, sunucu seviye atladı")
        }
        kind::THREAD_CREATED => format!("{who} bir konu başlattı: {}", msg.content),
        other => format!("{who} · sistem mesajı (tür {other})"),
    })
}

/// Çıkartma ve embed satırları.
fn extra_lines(msg: &Message, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for s in &msg.sticker_items {
        out.push(fit_line(
            vec![(format!("  [çıkartma: {}]", s.name), dim())],
            width,
        ));
    }
    for e in &msg.embeds {
        let title = e.title.as_deref().filter(|t| !t.is_empty());
        let desc = e
            .description
            .as_deref()
            .and_then(|d| d.lines().next())
            .filter(|d| !d.is_empty());
        let text = match (title, desc) {
            (Some(t), Some(d)) => format!("{t} — {d}"),
            (Some(t), None) => t.to_owned(),
            (None, Some(d)) => d.to_owned(),
            // Sadece bağlantı önizlemesi: bağlantı içerikte zaten görünüyor.
            (None, None) if !msg.content.is_empty() => continue,
            (None, None) => e.url.clone().unwrap_or_else(|| "(içerik yok)".into()),
        };
        out.push(fit_line(vec![(format!("  [embed: {text}]"), dim())], width));
    }
    out
}

fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

fn cwidth(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// Girinti, çok dar genişlikte en az 1 sütun içerik bırakacak şekilde kısılır.
fn indent_for(width: usize) -> usize {
    INDENT.min(width.saturating_sub(1))
}

fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    // Yalnızca gösterim için; kesinlik gerekmez.
    let b = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{:.1} MB", b / (KB * KB))
    }
}

/// Kontrol/sıfır genişlikli karakterleri atar, sekmeyi 4 boşluğa açar.
fn clean(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '\t' {
            out.push_str("    ");
        } else if cwidth(c) > 0 {
            out.push(c);
        }
    }
    out
}

/// Parçaları tek satıra sığdırır; taşarsa `…` ile keser.
fn fit_line(parts: Vec<(String, Style)>, width: usize) -> Line<'static> {
    let parts: Vec<(String, Style)> = parts.into_iter().map(|(t, s)| (clean(&t), s)).collect();
    let total: usize = parts.iter().map(|(t, _)| t.width()).sum();
    if total <= width {
        return Line::from(
            parts
                .into_iter()
                .map(|(t, s)| Span::styled(t, s))
                .collect::<Vec<_>>(),
        );
    }
    let budget = width.saturating_sub(1);
    let mut spans = Vec::new();
    let mut used = 0;
    let mut last_style = Style::new();
    'outer: for (text, style) in parts {
        last_style = style;
        let mut buf = String::new();
        for c in text.chars() {
            let cw = cwidth(c);
            if used + cw > budget {
                if !buf.is_empty() {
                    spans.push(Span::styled(buf, style));
                }
                break 'outer;
            }
            buf.push(c);
            used += cw;
        }
        if !buf.is_empty() {
            spans.push(Span::styled(buf, style));
        }
    }
    spans.push(Span::styled("…", last_style));
    Line::from(spans)
}

fn reply_line(reply: &Message, r: &dyn Resolver, width: usize) -> Line<'static> {
    let first = reply.content.lines().next().unwrap_or("");
    fit_line(
        vec![
            ("  ↳ ".into(), dim()),
            (
                r.author_name(&reply.author),
                dim().add_modifier(Modifier::BOLD),
            ),
            (": ".into(), dim()),
            (first.to_string(), dim()),
        ],
        width,
    )
}

// ---------------------------------------------------------------- içerik

type Run = (String, Style);

fn content_lines(msg: &Message, r: &dyn Resolver, width: usize) -> Vec<Line<'static>> {
    let blocks = markdown::parse(&msg.content);
    let mut rendered: Vec<(Vec<Line<'static>>, bool)> = blocks
        .iter()
        .map(|b| {
            let empty = b
                .spans
                .iter()
                .all(|s| matches!(s, Md::Text { text, .. } if text.trim().is_empty()));
            (block_lines(b, msg, r, width), empty)
        })
        .collect();
    while rendered.last().is_some_and(|(_, empty)| *empty) {
        rendered.pop();
    }
    rendered.into_iter().flat_map(|(l, _)| l).collect()
}

fn block_lines(block: &Block, msg: &Message, r: &dyn Resolver, width: usize) -> Vec<Line<'static>> {
    let indent = indent_for(width);
    let (first, cont, pstyle) = match &block.kind {
        BlockKind::Quote => ("▎ ", "▎ ", dim()),
        BlockKind::ListItem => ("• ", "  ", Style::new()),
        _ => ("", "", Style::new()),
    };
    // Önek, en az 1 sütun içerik kalacak şekilde kısılır.
    let pmax = width.saturating_sub(indent + 1);
    let first = take_width(first, pmax);
    let cont = take_width(cont, pmax);
    let avail = width.saturating_sub(indent + first.width()).max(1);

    let runs = match &block.kind {
        BlockKind::CodeBlock { .. } => {
            let text = block.spans.iter().map(span_text).collect::<String>();
            hard_wrap(&text, Style::new().fg(Color::Yellow), avail)
        }
        kind => {
            let extra = match kind {
                BlockKind::Heading(1) => {
                    Style::new().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                }
                BlockKind::Heading(_) => Style::new().add_modifier(Modifier::BOLD),
                BlockKind::Subtext => dim(),
                _ => Style::new(),
            };
            let pieces = inline_pieces(&block.spans, extra, msg, r);
            wrap_words(&pieces, avail)
        }
    };

    // Boş Text/Code satırı girintisiz tamamen boş satır olur.
    if runs.len() == 1 && runs[0].is_empty() && first.is_empty() {
        return vec![Line::default()];
    }
    runs.into_iter()
        .enumerate()
        .map(|(i, line)| {
            let prefix = if i == 0 { &first } else { &cont };
            let mut spans = Vec::with_capacity(line.len() + 2);
            if indent > 0 {
                spans.push(Span::raw(" ".repeat(indent)));
            }
            if !prefix.is_empty() {
                spans.push(Span::styled(prefix.clone(), pstyle));
            }
            spans.extend(line.into_iter().map(|(t, s)| Span::styled(t, s)));
            Line::from(spans)
        })
        .collect()
}

/// Kod bloğunda ham metin (tek `Text` span'ı).
fn span_text(s: &Md) -> &str {
    match s {
        Md::Text { text, .. } => text,
        _ => "",
    }
}

fn take_width(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = cwidth(c);
        if w + cw > max {
            break;
        }
        out.push(c);
        w += cw;
    }
    out
}

fn mention_style(fg: Color, highlight: bool) -> Style {
    let s = Style::new().fg(fg);
    if highlight {
        s.bg(Color::Rgb(60, 50, 0)).add_modifier(Modifier::BOLD)
    } else {
        s
    }
}

fn inline_pieces(spans: &[Md], extra: Style, msg: &Message, r: &dyn Resolver) -> Vec<Run> {
    spans
        .iter()
        .map(|s| {
            let (text, style) = match s {
                Md::Text { text, style } => {
                    let mut st = Style::new();
                    for (on, m) in [
                        (style.bold, Modifier::BOLD),
                        (style.italic, Modifier::ITALIC),
                        (style.underline, Modifier::UNDERLINED),
                        (style.strike, Modifier::CROSSED_OUT),
                    ] {
                        if on {
                            st = st.add_modifier(m);
                        }
                    }
                    if style.spoiler {
                        (mask(text), st.fg(Color::DarkGray))
                    } else {
                        (text.clone(), st)
                    }
                }
                Md::Code(t) => (t.clone(), Style::new().fg(Color::Yellow)),
                Md::UserMention(id) => (
                    format!("@{}", r.user_name(*id).as_deref().unwrap_or("bilinmeyen")),
                    mention_style(Color::LightBlue, r.is_me(*id)),
                ),
                Md::ChannelMention(id) => (
                    format!(
                        "#{}",
                        r.channel_name(*id).as_deref().unwrap_or("bilinmeyen-kanal")
                    ),
                    mention_style(Color::LightBlue, false),
                ),
                Md::RoleMention(id) => (
                    format!(
                        "@{}",
                        r.role_name(*id).as_deref().unwrap_or("bilinmeyen-rol")
                    ),
                    mention_style(Color::LightMagenta, false),
                ),
                Md::Everyone => (
                    "@everyone".to_string(),
                    mention_style(Color::LightMagenta, msg.mention_everyone),
                ),
                Md::Here => (
                    "@here".to_string(),
                    mention_style(Color::LightMagenta, msg.mention_everyone),
                ),
                Md::Emoji { name, .. } => (format!(":{name}:"), Style::new().fg(Color::Yellow)),
                Md::Timestamp { unix, format } => (
                    r.timestamp(*unix, *format),
                    dim().add_modifier(Modifier::UNDERLINED),
                ),
                Md::Link(url) => (
                    url.clone(),
                    Style::new()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::UNDERLINED),
                ),
            };
            (text, style.patch(extra))
        })
        .collect()
}

/// Spoiler: boşluklar korunur, diğer karakterler aynı genişlikte `▒` olur.
fn mask(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == ' ' || c == '\t' {
            out.push(c);
        } else {
            for _ in 0..cwidth(c) {
                out.push('▒');
            }
        }
    }
    out
}

// ---------------------------------------------------------------- sarma

/// Satır biriktirici: aynı stildeki bitişik karakterleri tek run'da toplar.
struct LineBuf {
    avail: usize,
    out: Vec<Vec<Run>>,
    cur: Vec<Run>,
    cur_w: usize,
}

impl LineBuf {
    fn new(avail: usize) -> Self {
        Self {
            avail: avail.max(1),
            out: Vec::new(),
            cur: Vec::new(),
            cur_w: 0,
        }
    }

    fn push(&mut self, c: char, style: Style, cw: usize) {
        match self.cur.last_mut() {
            Some((t, s)) if *s == style => t.push(c),
            _ => self.cur.push((c.to_string(), style)),
        }
        self.cur_w += cw;
    }

    fn newline(&mut self) {
        self.out.push(std::mem::take(&mut self.cur));
        self.cur_w = 0;
    }

    /// Karakter karakter yerleştirir; sığmayan karakterde satır kırar.
    fn push_hard(&mut self, c: char, style: Style, cw: usize) {
        if cw > self.avail {
            return; // hiçbir satıra sığmaz
        }
        if self.cur_w + cw > self.avail {
            self.newline();
        }
        self.push(c, style, cw);
    }

    fn finish(mut self) -> Vec<Vec<Run>> {
        self.out.push(self.cur);
        self.out
    }
}

/// Kod bloğu: boşlukları koruyarak genişliğe göre sert kırar.
fn hard_wrap(text: &str, style: Style, avail: usize) -> Vec<Vec<Run>> {
    let mut buf = LineBuf::new(avail);
    for c in clean(text).chars() {
        buf.push_hard(c, style, cwidth(c));
    }
    buf.finish()
}

/// Açgözlü kelime sarma; stiller bölünen run'larda korunur.
fn wrap_words(pieces: &[Run], avail: usize) -> Vec<Vec<Run>> {
    let mut buf = LineBuf::new(avail);
    let mut pend: Vec<(Style, usize)> = Vec::new(); // bekleyen boşluklar
    let mut word: Vec<(char, Style, usize)> = Vec::new();

    for (text, style) in pieces {
        for c in clean(text).chars() {
            if c == ' ' {
                flush_word(&mut buf, &mut word, &mut pend);
                pend.push((*style, 1));
            } else {
                word.push((c, *style, cwidth(c)));
            }
        }
    }
    flush_word(&mut buf, &mut word, &mut pend);
    buf.finish()
}

fn flush_word(
    buf: &mut LineBuf,
    word: &mut Vec<(char, Style, usize)>,
    pend: &mut Vec<(Style, usize)>,
) {
    if word.is_empty() {
        return;
    }
    // Devam satırı başındaki boşluklar atılır; ilk satırdaki baştakiler korunur.
    let keep = buf.cur_w > 0 || buf.out.is_empty();
    let pend_w: usize = if keep {
        pend.iter().map(|p| p.1).sum()
    } else {
        0
    };
    let word_w: usize = word.iter().map(|w| w.2).sum();
    if buf.cur_w + pend_w + word_w <= buf.avail {
        if keep {
            for (s, w) in pend.iter() {
                buf.push(' ', *s, *w);
            }
        }
        for &(c, s, w) in word.iter() {
            buf.push(c, s, w);
        }
    } else {
        if buf.cur_w > 0 {
            buf.newline();
        }
        for &(c, s, w) in word.iter() {
            buf.push_hard(c, s, w);
        }
    }
    pend.clear();
    word.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    const T0: u64 = 1_700_000_000_000;
    const MIN: u64 = 60_000;

    struct Fake;

    impl Resolver for Fake {
        fn user_name(&self, id: u64) -> Option<String> {
            (id == 5 || id == 7).then(|| format!("kisi{id}"))
        }
        fn channel_name(&self, id: u64) -> Option<String> {
            (id == 1).then(|| "genel".to_string())
        }
        fn role_name(&self, id: u64) -> Option<String> {
            (id == 3).then(|| "yönetici".to_string())
        }
        fn is_me(&self, id: u64) -> bool {
            id == 7
        }
        fn author_name(&self, user: &User) -> String {
            user.display_name().to_string()
        }
        fn clock(&self, _unix_ms: u64) -> String {
            "14:02".to_string()
        }
        fn timestamp(&self, unix_secs: i64, _format: Option<char>) -> String {
            format!("T{unix_secs}")
        }
    }

    fn mjson(ms: u64, author: u64, content: &str) -> Value {
        let id = (ms - 1_420_070_400_000) << 22;
        json!({
            "id": id.to_string(), "channel_id": "2",
            "author": {"id": author.to_string(), "username": "u", "global_name": "Ayşe"},
            "content": content,
        })
    }

    fn build(v: Value) -> Message {
        serde_json::from_value(v).expect("mesaj")
    }

    fn msg(ms: u64, author: u64, content: &str) -> Message {
        build(mjson(ms, author, content))
    }

    fn text(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn texts(lines: &[Line]) -> Vec<String> {
        lines.iter().map(text).collect()
    }

    fn assert_fits(lines: &[Line], width: u16) {
        for l in lines {
            let w: usize = l.spans.iter().map(Span::width).sum();
            assert!(w <= usize::from(width), "{w} > {width}: {:?}", text(l));
        }
    }

    /// Gruplanmış çizim: sadece içerik (+ek/düzenlendi) satırları.
    fn body(m: &Message, width: u16) -> Vec<Line<'static>> {
        message_lines(m, true, &Fake, width)
    }

    fn last_style(l: &Line) -> Style {
        l.spans.last().map(|s| s.style).unwrap_or_default()
    }

    #[test]
    fn grouping_rules() {
        let a = msg(T0, 1, "a");
        assert!(!is_grouped(None, &a));
        assert!(is_grouped(Some(&a), &msg(T0 + 6 * MIN, 1, "b")));
        assert!(!is_grouped(Some(&a), &msg(T0 + 7 * MIN, 1, "b")));
        assert!(!is_grouped(Some(&a), &msg(T0 + MIN, 2, "b")));

        let mut v = mjson(T0 + MIN, 1, "b");
        v["referenced_message"] = mjson(T0, 2, "x");
        assert!(!is_grouped(Some(&a), &build(v)));

        let mut v = mjson(T0 + MIN, 1, "b");
        v["type"] = json!(19);
        assert!(!is_grouped(Some(&a), &build(v)));
    }

    #[test]
    fn header_and_spacer() {
        let m = msg(T0, 1, "selam");
        let lines = message_lines(&m, false, &Fake, 40);
        assert_eq!(texts(&lines), ["", "Ayşe  14:02", "  selam"]);
        let name = &lines[1].spans[0];
        assert_eq!(name.style.fg, Some(Color::Cyan));
        assert!(name.style.add_modifier.contains(Modifier::BOLD));
        assert!(lines[1].spans[2].style.add_modifier.contains(Modifier::DIM));
        // Gruplanmış: başlık ve ayırıcı yok.
        assert_eq!(texts(&body(&m, 40)), ["  selam"]);
    }

    #[test]
    fn header_truncated() {
        let mut v = mjson(T0, 1, "x");
        v["author"]["global_name"] = json!("Çok Uzun Bir Kullanıcı Adı 🎉🎉");
        let m = build(v);
        for w in 1..40 {
            assert_fits(&message_lines(&m, false, &Fake, w), w);
        }
        let lines = message_lines(&m, false, &Fake, 12);
        assert!(text(&lines[1]).ends_with('…'));
    }

    #[test]
    fn reply_preview() {
        let mut v = mjson(T0, 1, "cevap");
        v["type"] = json!(19);
        v["referenced_message"] = mjson(T0 - MIN, 2, "çılgın şeyler 🎉🎉🎉 oluyor\nikinci satır");
        let m = build(v);
        let lines = message_lines(&m, false, &Fake, 80);
        assert_eq!(text(&lines[1]), "  ↳ Ayşe: çılgın şeyler 🎉🎉🎉 oluyor");
        assert!(
            lines[1].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        for w in 1..60 {
            assert_fits(&message_lines(&m, false, &Fake, w), w);
        }
        let lines = message_lines(&m, false, &Fake, 22);
        assert!(text(&lines[1]).ends_with('…'));

        let mut v = mjson(T0, 1, "cevap");
        v["type"] = json!(19);
        let lines = message_lines(&build(v), false, &Fake, 40);
        assert_eq!(text(&lines[1]), "  ↳ (silinmiş mesaj)");
    }

    #[test]
    fn wraps_exactly_at_width() {
        // Genişlik 9 → içerik 7.
        let lines = body(&msg(T0, 1, "aaa bbb ccc"), 9);
        assert_eq!(texts(&lines), ["  aaa bbb", "  ccc"]);
        // Devam satırı başındaki boşluklar atılır.
        let lines = body(&msg(T0, 1, "aaaaaaa   bbb"), 9);
        assert_eq!(texts(&lines), ["  aaaaaaa", "  bbb"]);
    }

    #[test]
    fn never_exceeds_width() {
        let inputs = [
            "kısa",
            "Türkçe karakterler: ığüşöç İĞÜŞÖÇ ve uzunca bir cümle burada",
            "emoji 🎉🎉🎉 yan yana 👨‍👩‍👧 ve 日本語のテキストも var",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "> alıntı satırı biraz uzun olsun bakalım\n- liste öğesi de uzun olsun bakalım",
            "# Başlık çok uzun bir başlık\n-# küçük yazı\n```\nlet x = 1234567890123456;\n\tsekme\n```",
            "**kalın metin** *italik* ||gizli metin burada|| `kod` <@5> <#1> <@&3> @everyone <t:1:F>",
            "https://example.com/çok/uzun/bir/adres/olabilir?x=1&y=2 son",
            "tab\there \u{200b}zero\u{7} width",
        ];
        for input in inputs {
            let mut v = mjson(T0, 1, input);
            v["edited_timestamp"] = json!("2025-01-01T00:00:00+00:00");
            v["attachments"] = json!([
                {"id":"7","filename":"çok-uzun-dosya-adı.png","size":3,"width":10,"height":20}
            ]);
            let m = build(v);
            for w in 1..=60u16 {
                assert_fits(&message_lines(&m, false, &Fake, w), w);
                assert_fits(&message_lines(&m, true, &Fake, w), w);
            }
        }
    }

    #[test]
    fn long_word_is_split() {
        let lines = body(&msg(T0, 1, "abcdefghij"), 6);
        assert_eq!(texts(&lines), ["  abcd", "  efgh", "  ij"]);
        // Önceki kelime varsa uzun kelime yeni satırdan başlar.
        let lines = body(&msg(T0, 1, "ab abcdefghij"), 6);
        assert_eq!(texts(&lines), ["  ab", "  abcd", "  efgh", "  ij"]);
    }

    #[test]
    fn styles_survive_wrap() {
        let lines = body(&msg(T0, 1, "**aaaa bbbb cccc**"), 6);
        assert_eq!(texts(&lines), ["  aaaa", "  bbbb", "  cccc"]);
        for l in &lines {
            assert!(l.spans[1].style.add_modifier.contains(Modifier::BOLD));
        }
        // Tek kelime satırlara bölünse de stil her iki parçada kalır.
        let lines = body(&msg(T0, 1, "*abcdefgh*"), 6);
        assert_eq!(texts(&lines), ["  abcd", "  efgh"]);
        for l in &lines {
            assert!(l.spans[1].style.add_modifier.contains(Modifier::ITALIC));
        }
    }

    #[test]
    fn block_prefixes() {
        let lines = body(&msg(T0, 1, "> aaa bbb ccc"), 9);
        assert_eq!(texts(&lines), ["  ▎ aaa", "  ▎ bbb", "  ▎ ccc"]);
        assert!(lines[0].spans[1].style.add_modifier.contains(Modifier::DIM));

        let lines = body(&msg(T0, 1, "- aaa bbb ccc"), 9);
        assert_eq!(texts(&lines), ["  • aaa", "    bbb", "    ccc"]);

        let lines = body(&msg(T0, 1, "# Baslik\n## Alt\n-# kucuk"), 40);
        assert_eq!(texts(&lines), ["  Baslik", "  Alt", "  kucuk"]);
        let h1 = last_style(&lines[0]);
        assert!(
            h1.add_modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );
        let h2 = last_style(&lines[1]);
        assert!(h2.add_modifier.contains(Modifier::BOLD));
        assert!(!h2.add_modifier.contains(Modifier::UNDERLINED));
        assert!(last_style(&lines[2]).add_modifier.contains(Modifier::DIM));

        let lines = body(&msg(T0, 1, "```\nlet  x = 1;\n```"), 10);
        assert_eq!(texts(&lines), ["  let  x =", "   1;"]);
        for l in &lines {
            assert_eq!(last_style(l).fg, Some(Color::Yellow));
        }
    }

    #[test]
    fn spoiler_is_masked_with_same_width() {
        let lines = body(&msg(T0, 1, "||gizli yazı||"), 40);
        assert_eq!(text(&lines[0]), "  ▒▒▒▒▒ ▒▒▒▒");
        assert_eq!(text(&lines[0]).width(), "  gizli yazı".width());
        assert_eq!(lines[0].spans[1].style.fg, Some(Color::DarkGray));
    }

    #[test]
    fn mentions() {
        let m = msg(
            T0,
            1,
            "<@5> <@7> <@99> <#1> <#9> <@&3> <@&8> <:pog:12> <t:42:F>",
        );
        let lines = body(&m, 200);
        assert_eq!(
            text(&lines[0]),
            "  @kisi5 @kisi7 @bilinmeyen #genel #bilinmeyen-kanal @yönetici @bilinmeyen-rol :pog: T42"
        );
        let find = |s: &str| {
            lines[0]
                .spans
                .iter()
                .find(|sp| sp.content == s)
                .map(|sp| sp.style)
                .unwrap_or_default()
        };
        assert_eq!(find("@kisi5").fg, Some(Color::LightBlue));
        assert_eq!(find("@kisi5").bg, None);
        assert_eq!(find("@kisi7").bg, Some(Color::Rgb(60, 50, 0)));
        assert!(find("@kisi7").add_modifier.contains(Modifier::BOLD));
        assert_eq!(find("#genel").fg, Some(Color::LightBlue));
        assert_eq!(find("@yönetici").fg, Some(Color::LightMagenta));
        assert!(find("T42").add_modifier.contains(Modifier::UNDERLINED));

        let mut v = mjson(T0, 1, "@everyone @here");
        let plain = body(&build(v.clone()), 40);
        assert_eq!(plain[0].spans[1].style.bg, None);
        v["mention_everyone"] = json!(true);
        let hot = body(&build(v), 40);
        assert_eq!(hot[0].spans[1].style.bg, Some(Color::Rgb(60, 50, 0)));
        assert_eq!(hot[0].spans[1].style.fg, Some(Color::LightMagenta));
    }

    #[test]
    fn attachments() {
        let mut v = mjson(T0, 1, "");
        v["attachments"] = json!([
            {"id":"1","filename":"a.png","size":99,"width":1280,"height":720},
            {"id":"2","filename":"b.bin","size":500},
            {"id":"3","filename":"c.zip","size":1234},
            {"id":"4","filename":"d.mkv","size":1_258_291},
        ]);
        let lines = body(&build(v), 60);
        assert_eq!(
            texts(&lines),
            [
                "  [ek: a.png 1280×720]",
                "  [ek: b.bin 500 B]",
                "  [ek: c.zip 1.2 KB]",
                "  [ek: d.mkv 1.2 MB]",
            ]
        );
        assert!(lines[0].spans[0].style.add_modifier.contains(Modifier::DIM));

        let mut v = mjson(T0, 1, "");
        v["attachments"] = json!([{"id":"1","filename":"uzun-dosya-adi.png","size":1}]);
        let lines = body(&build(v), 10);
        assert_fits(&lines, 10);
        assert!(text(&lines[0]).ends_with('…'));
    }

    #[test]
    fn edited_marker() {
        let mut v = mjson(T0, 1, "merhaba");
        v["edited_timestamp"] = json!("2025-01-01T00:00:00+00:00");
        let m = build(v);
        assert_eq!(texts(&body(&m, 40)), ["  merhaba (düzenlendi)"]);
        // 2+7+13 = 22 sığar.
        assert_eq!(texts(&body(&m, 22)), ["  merhaba (düzenlendi)"]);
        // 21'de sığmaz → ayrı girintili satır.
        assert_eq!(texts(&body(&m, 21)), ["  merhaba", "  (düzenlendi)"]);
        // Çok dar: ayrı satır kırpılır ama taşmaz.
        assert_fits(&body(&m, 6), 6);
    }

    #[test]
    fn empty_message_and_trailing_blanks() {
        let lines = message_lines(&msg(T0, 1, ""), false, &Fake, 30);
        assert_eq!(texts(&lines), ["", "Ayşe  14:02", "  (boş mesaj)"]);
        assert!(lines[2].spans[0].style.add_modifier.contains(Modifier::DIM));

        let lines = body(&msg(T0, 1, "merhaba\n\n\n"), 30);
        assert_eq!(texts(&lines), ["  merhaba"]);
        // Aradaki boş satırlar korunur.
        let lines = body(&msg(T0, 1, "a\n\nb"), 30);
        assert_eq!(texts(&lines), ["  a", "", "  b"]);
    }

    #[test]
    fn system_messages_are_single_line_and_break_groups() {
        let mut call: Message = serde_json::from_value(serde_json::json!({
            "id": "1000000000000000000", "channel_id": "2",
            "author": {"id": "4", "username": "ali", "global_name": null},
            "content": "", "type": 3, "edited_timestamp": "2026-01-01T00:00:00+00:00",
            "call": {"participants": ["4"], "ended_timestamp": "2026-01-01T00:05:00+00:00"}
        }))
        .expect("arama");
        let lines = message_lines(&call, false, &SysRes, 60);
        assert_eq!(lines.len(), 2, "ayırıcı + tek satır");
        let text: String = lines[1].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("ali bir arama başlattı (bitti)"), "{text}");
        assert!(!text.contains("düzenlendi"));

        let normal: Message = serde_json::from_value(serde_json::json!({
            "id": "1000000000000000001", "channel_id": "2",
            "author": {"id": "4", "username": "ali", "global_name": null}, "content": "selam"
        }))
        .expect("mesaj");
        assert!(!is_grouped(Some(&call), &normal));
        call.kind = 0;
        assert!(is_grouped(Some(&call), &normal));
    }

    #[test]
    fn stickers_and_embeds_shown_instead_of_empty() {
        let m: Message = serde_json::from_value(serde_json::json!({
            "id": "1000000000000000000", "channel_id": "2",
            "author": {"id": "4", "username": "ali", "global_name": null}, "content": "",
            "sticker_items": [{"id": "1", "name": "kedi", "format_type": 1}],
            "embeds": [{"type": "rich", "title": "Başlık", "description": "açıklama\nikinci"}]
        }))
        .expect("mesaj");
        let text: Vec<String> = message_lines(&m, true, &SysRes, 60)
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        assert_eq!(text, ["  [çıkartma: kedi]", "  [embed: Başlık — açıklama]"]);
    }

    struct SysRes;
    impl Resolver for SysRes {
        fn user_name(&self, _: u64) -> Option<String> {
            None
        }
        fn channel_name(&self, _: u64) -> Option<String> {
            None
        }
        fn role_name(&self, _: u64) -> Option<String> {
            None
        }
        fn is_me(&self, _: u64) -> bool {
            false
        }
        fn author_name(&self, user: &User) -> String {
            user.username.clone()
        }
        fn clock(&self, _: u64) -> String {
            "12:00".into()
        }
        fn timestamp(&self, _: i64, _: Option<char>) -> String {
            String::new()
        }
    }
}
