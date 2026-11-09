//! Discord-flavored markdown parser for message content (pure `std`).
//!
//! `parse` turns message text into one `Block` per source line; each block holds
//! inline `Span`s with nested styles, mentions, emoji, timestamps and links.

/// Inline style flags; styles nest (e.g. bold+italic).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub spoiler: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Span {
    Text {
        text: String,
        style: Style,
    },
    /// `inline code` (no styles inside)
    Code(String),
    /// `<@123>` and `<@!123>`
    UserMention(u64),
    /// `<#123>`
    ChannelMention(u64),
    /// `<@&123>`
    RoleMention(u64),
    Everyone,
    Here,
    /// `<:name:id>` / `<a:name:id>`
    Emoji {
        name: String,
        id: u64,
        animated: bool,
    },
    /// `<t:1618953630>` / `<t:1618953630:R>`
    Timestamp {
        unix: i64,
        format: Option<char>,
    },
    /// Bare `http://` or `https://` URL
    Link(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockKind {
    Text,
    Quote,
    Heading(u8),
    ListItem,
    Subtext,
    CodeBlock { lang: Option<String> },
}

/// One rendered source line. For `CodeBlock`, `spans` holds exactly one
/// `Span::Text` with the raw line (no inline parsing) and default style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub spans: Vec<Span>,
}

const FENCE: &str = "```";

/// Parse message content into per-line blocks.
pub fn parse(content: &str) -> Vec<Block> {
    let lines: Vec<&str> = content.split('\n').collect();
    let mut blocks = Vec::with_capacity(lines.len());
    let mut quote_all = false;
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if let Some(rest) = line.strip_prefix(FENCE) {
            // Single-line ```code```.
            if let Some(end) = rest.find(FENCE) {
                blocks.push(code_line(None, &rest[..end]));
                i += 1;
                continue;
            }
            // Multi-line: closing fence is the first later line containing ```.
            if let Some(p) = lines[i + 1..].iter().position(|l| l.contains(FENCE)) {
                let j = i + 1 + p;
                let (lang, first) = split_lang(rest);
                if let Some(f) = first {
                    blocks.push(code_line(None, f));
                }
                // `lang` applies to every line; patch the first one if it was pushed.
                let first_idx = blocks.len().saturating_sub(usize::from(first.is_some()));
                for l in &lines[i + 1..j] {
                    blocks.push(code_line(None, l));
                }
                let before = lines[j].split_once(FENCE).map_or("", |(b, _)| b);
                if !before.is_empty() {
                    blocks.push(code_line(None, before));
                }
                for b in &mut blocks[first_idx..] {
                    b.kind = BlockKind::CodeBlock { lang: lang.clone() };
                }
                i = j + 1;
                continue;
            }
            // Unclosed fence: fall through and parse as plain text.
        }
        blocks.push(line_block(line, &mut quote_all));
        i += 1;
    }
    blocks
}

fn code_line(lang: Option<String>, raw: &str) -> Block {
    Block {
        kind: BlockKind::CodeBlock { lang },
        spans: vec![Span::Text {
            text: raw.to_owned(),
            style: Style::default(),
        }],
    }
}

/// Text right after the opening fence: a lone language word, or first code line.
fn split_lang(rest: &str) -> (Option<String>, Option<&str>) {
    if rest.is_empty() {
        (None, None)
    } else if rest
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-'))
    {
        (Some(rest.to_owned()), None)
    } else {
        (None, Some(rest))
    }
}

fn line_block(line: &str, quote_all: &mut bool) -> Block {
    let (kind, rest) = if *quote_all {
        (BlockKind::Quote, line)
    } else if let Some(r) = line.strip_prefix(">>> ") {
        *quote_all = true;
        (BlockKind::Quote, r)
    } else if let Some(r) = line.strip_prefix("> ") {
        (BlockKind::Quote, r)
    } else if let Some(r) = line.strip_prefix("### ") {
        (BlockKind::Heading(3), r)
    } else if let Some(r) = line.strip_prefix("## ") {
        (BlockKind::Heading(2), r)
    } else if let Some(r) = line.strip_prefix("# ") {
        (BlockKind::Heading(1), r)
    } else if let Some(r) = line.strip_prefix("-# ") {
        (BlockKind::Subtext, r)
    } else {
        let t = line.trim_start_matches(' ');
        match t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            Some(r) => (BlockKind::ListItem, r),
            None => (BlockKind::Text, line),
        }
    };
    let chars: Vec<char> = rest.chars().collect();
    let mut spans = Vec::new();
    parse_inline(&chars, Style::default(), &mut spans);
    Block { kind, spans }
}

// ---------------------------------------------------------------- inline

fn push_char(out: &mut Vec<Span>, c: char, style: Style) {
    if let Some(Span::Text { text, style: s }) = out.last_mut()
        && *s == style
    {
        text.push(c);
        return;
    }
    out.push(Span::Text {
        text: c.to_string(),
        style,
    });
}

fn starts_with(chars: &[char], i: usize, pat: &str) -> bool {
    pat.chars()
        .enumerate()
        .all(|(o, c)| chars.get(i + o) == Some(&c))
}

fn run_len(chars: &[char], i: usize) -> usize {
    chars[i..].iter().take_while(|&&c| c == chars[i]).count()
}

fn is_escapable(c: char) -> bool {
    matches!(
        c,
        '*' | '_' | '~' | '|' | '`' | '\\' | '>' | '<' | '#' | '-'
    )
}

fn parse_inline(chars: &[char], style: Style, out: &mut Vec<Span>) {
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if chars.get(i + 1).is_some_and(|&n| is_escapable(n)) => {
                push_char(out, chars[i + 1], style);
                i += 2;
                continue;
            }
            '`' => {
                let r = run_len(chars, i);
                if let Some((n, close)) = find_code_close(chars, i) {
                    out.push(Span::Code(chars[i + n..close].iter().collect()));
                    i = close + n;
                } else {
                    for _ in 0..r {
                        push_char(out, '`', style);
                    }
                    i += r;
                }
                continue;
            }
            '<' => {
                if let Some((span, len)) = parse_angle(chars, i) {
                    out.push(span);
                    i += len;
                    continue;
                }
            }
            '@' => {
                if starts_with(chars, i, "@everyone") {
                    out.push(Span::Everyone);
                    i += "@everyone".len();
                    continue;
                }
                if starts_with(chars, i, "@here") {
                    out.push(Span::Here);
                    i += "@here".len();
                    continue;
                }
            }
            'h' => {
                if let Some(len) = url_len(chars, i) {
                    out.push(Span::Link(chars[i..i + len].iter().collect()));
                    i += len;
                    continue;
                }
            }
            '*' | '_' | '~' | '|' => {
                if let Some(next) = try_delim(chars, i, style, out) {
                    i = next;
                    continue;
                }
            }
            _ => {}
        }
        push_char(out, c, style);
        i += 1;
    }
}

/// Inline code starting at `i`: returns (backtick count, index of closing run).
fn find_code_close(chars: &[char], i: usize) -> Option<(usize, usize)> {
    let n = run_len(chars, i);
    if n > 2 {
        return None;
    }
    let mut k = i + n;
    while k < chars.len() {
        if chars[k] == '`' {
            let m = run_len(chars, k);
            if m == n && k > i + n {
                return Some((n, k));
            }
            k += m;
        } else {
            k += 1;
        }
    }
    None
}

/// Try a styled region opening at `i`. Returns the index after the closing marker.
fn try_delim(chars: &[char], i: usize, style: Style, out: &mut Vec<Span>) -> Option<usize> {
    let c = chars[i];
    let r = run_len(chars, i);
    let (min_n, max_n) = match c {
        '*' => (1, 3),
        '_' => (1, 2),
        _ => (2, 2),
    };
    if r < min_n {
        return None;
    }
    let start = i + r;
    for n in (min_n..=r.min(max_n)).rev() {
        let italic = n == 1;
        if italic && chars.get(start).is_none_or(|ch| ch.is_whitespace()) {
            continue;
        }
        let word = c == '_' && italic;
        if word && i > 0 && chars[i - 1].is_alphanumeric() {
            continue;
        }
        let Some(close) = find_close(chars, start, c, n, word) else {
            continue;
        };
        for _ in 0..r - n {
            push_char(out, c, style);
        }
        let mut inner = style;
        match (c, n) {
            ('*', 1) | ('_', 1) => inner.italic = true,
            ('*', 2) => inner.bold = true,
            ('*', _) => {
                inner.bold = true;
                inner.italic = true;
            }
            ('_', _) => inner.underline = true,
            ('~', _) => inner.strike = true,
            _ => inner.spoiler = true,
        }
        parse_inline(&chars[start..close], inner, out);
        return Some(close + n);
    }
    None
}

/// Find the closing marker (`n` x `ch`) for content starting at `from`.
/// Returns the index where the marker begins. Skips escapes and code spans.
fn find_close(chars: &[char], from: usize, ch: char, n: usize, word: bool) -> Option<usize> {
    let mut k = from;
    while k < chars.len() {
        let x = chars[k];
        if x == '\\' && chars.get(k + 1).is_some_and(|&c| is_escapable(c)) {
            k += 2;
        } else if x == '`' {
            k = match find_code_close(chars, k) {
                Some((m, close)) => close + m,
                None => k + run_len(chars, k),
            };
        } else if x == ch {
            let m = run_len(chars, k);
            if n == 1 {
                let boundary_ok = !word || chars.get(k + 1).is_none_or(|c| !c.is_alphanumeric());
                if m == 1 && k > from && boundary_ok {
                    return Some(k);
                }
            } else if m >= n && k + m - n > from {
                return Some(k + m - n);
            }
            k += m;
        } else {
            k += 1;
        }
    }
    None
}

fn parse_id(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `<...>` constructs. Returns the span and the number of chars consumed.
fn parse_angle(chars: &[char], i: usize) -> Option<(Span, usize)> {
    let end = chars[i + 1..].iter().take(100).position(|&c| c == '>')? + i + 1;
    let body: String = chars[i + 1..end].iter().collect();
    let span = if let Some(r) = body.strip_prefix("@&") {
        Span::RoleMention(parse_id(r)?)
    } else if let Some(r) = body.strip_prefix("@!").or_else(|| body.strip_prefix('@')) {
        Span::UserMention(parse_id(r)?)
    } else if let Some(r) = body.strip_prefix('#') {
        Span::ChannelMention(parse_id(r)?)
    } else if let Some(r) = body.strip_prefix("t:") {
        let (num, format) = match r.split_once(':') {
            Some((n, f)) => {
                let mut it = f.chars();
                match (it.next(), it.next()) {
                    (Some(c), None) if "tTdDfFR".contains(c) => (n, Some(c)),
                    _ => return None,
                }
            }
            None => (r, None),
        };
        let digits = num.strip_prefix('-').unwrap_or(num);
        parse_id(digits)?;
        Span::Timestamp {
            unix: num.parse().ok()?,
            format,
        }
    } else {
        let (animated, r) = match body.strip_prefix("a:") {
            Some(r) => (true, r),
            None => (false, body.strip_prefix(':')?),
        };
        let (name, id) = r.split_once(':')?;
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }
        Span::Emoji {
            name: name.to_owned(),
            id: parse_id(id)?,
            animated,
        }
    };
    Some((span, end - i + 1))
}

/// Length (in chars) of a bare URL starting at `i`, after trimming trailing punctuation.
fn url_len(chars: &[char], i: usize) -> Option<usize> {
    let scheme = if starts_with(chars, i, "https://") {
        "https://".len()
    } else if starts_with(chars, i, "http://") {
        "http://".len()
    } else {
        return None;
    };
    let mut len = chars[i..].iter().take_while(|c| !c.is_whitespace()).count();
    loop {
        let url = &chars[i..i + len];
        let last = url[len - 1];
        let open = url.iter().filter(|&&c| c == '(').count();
        let close = url.iter().filter(|&&c| c == ')').count();
        if matches!(last, '.' | ',' | '!' | '?' | ':' | ';') || (last == ')' && close > open) {
            len -= 1;
        } else {
            break;
        }
        if len <= scheme {
            return None;
        }
    }
    (len > scheme).then_some(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(f: impl FnOnce(&mut Style)) -> Style {
        let mut s = Style::default();
        f(&mut s);
        s
    }
    fn t(text: &str) -> Span {
        Span::Text {
            text: text.into(),
            style: Style::default(),
        }
    }
    fn ts(text: &str, style: Style) -> Span {
        Span::Text {
            text: text.into(),
            style,
        }
    }
    fn bold() -> Style {
        st(|s| s.bold = true)
    }
    fn italic() -> Style {
        st(|s| s.italic = true)
    }
    /// Spans of the single-line message.
    fn inline(s: &str) -> Vec<Span> {
        let b = parse(s);
        assert_eq!(b.len(), 1, "{b:?}");
        b.into_iter().next().map(|b| b.spans).unwrap_or_default()
    }
    fn block(kind: BlockKind, spans: Vec<Span>) -> Block {
        Block { kind, spans }
    }

    #[test]
    fn empty_input_and_empty_lines() {
        assert_eq!(parse(""), vec![block(BlockKind::Text, vec![])]);
        assert_eq!(
            parse("a\n\nb"),
            vec![
                block(BlockKind::Text, vec![t("a")]),
                block(BlockKind::Text, vec![]),
                block(BlockKind::Text, vec![t("b")]),
            ]
        );
    }

    #[test]
    fn plain_text() {
        assert_eq!(inline("merhaba dünya"), vec![t("merhaba dünya")]);
    }

    #[test]
    fn basic_styles() {
        assert_eq!(inline("**b**"), vec![ts("b", bold())]);
        assert_eq!(inline("*i*"), vec![ts("i", italic())]);
        assert_eq!(inline("_i_"), vec![ts("i", italic())]);
        assert_eq!(inline("__u__"), vec![ts("u", st(|s| s.underline = true))]);
        assert_eq!(inline("~~s~~"), vec![ts("s", st(|s| s.strike = true))]);
        assert_eq!(inline("||sp||"), vec![ts("sp", st(|s| s.spoiler = true))]);
    }

    #[test]
    fn bold_italic_triple() {
        let both = st(|s| {
            s.bold = true;
            s.italic = true;
        });
        assert_eq!(inline("***x***"), vec![ts("x", both)]);
    }

    #[test]
    fn nesting() {
        let both = st(|s| {
            s.bold = true;
            s.italic = true;
        });
        assert_eq!(
            inline("**bold *both* bold**"),
            vec![ts("bold ", bold()), ts("both", both), ts(" bold", bold())]
        );
        let bu = st(|s| {
            s.bold = true;
            s.underline = true;
        });
        assert_eq!(inline("__**x**__"), vec![ts("x", bu)]);
        assert_eq!(inline("**a *b***"), vec![ts("a ", bold()), ts("b", both)]);
    }

    #[test]
    fn unmatched_markers_are_literal() {
        assert_eq!(inline("2*3"), vec![t("2*3")]);
        assert_eq!(inline("a ** b"), vec![t("a ** b")]);
        assert_eq!(inline("**oops"), vec![t("**oops")]);
        assert_eq!(inline("~single~ | pipe"), vec![t("~single~ | pipe")]);
        assert_eq!(inline("2 * 3 * 4"), vec![t("2 * 3 * 4")]);
        assert_eq!(inline("`open"), vec![t("`open")]);
    }

    #[test]
    fn partial_run_fallback() {
        assert_eq!(inline("**a*"), vec![t("*"), ts("a", italic())]);
    }

    #[test]
    fn snake_case_stays_literal() {
        assert_eq!(inline("snake_case_words"), vec![t("snake_case_words")]);
        assert_eq!(inline("a _b_ c"), vec![t("a "), ts("b", italic()), t(" c")]);
        assert_eq!(inline("_b_c"), vec![t("_b_c")]);
    }

    #[test]
    fn escapes() {
        assert_eq!(inline(r"\*not\*"), vec![t("*not*")]);
        assert_eq!(
            inline(r"\_ \~ \| \` \\ \> \< \# \-"),
            vec![t(r"_ ~ | ` \ > < # -")]
        );
        assert_eq!(inline(r"\a"), vec![t(r"\a")]);
        assert_eq!(inline(r"**a\*\***"), vec![ts("a**", bold())]);
    }

    #[test]
    fn inline_code() {
        assert_eq!(
            inline("a `b **c**` d"),
            vec![t("a "), Span::Code("b **c**".into()), t(" d")]
        );
        assert_eq!(inline("``co`de``"), vec![Span::Code("co`de".into())]);
        // Markers inside code do not close an outer style.
        assert_eq!(
            inline("**a `**` b**"),
            vec![ts("a ", bold()), Span::Code("**".into()), ts(" b", bold())]
        );
    }

    #[test]
    fn fenced_with_lang() {
        let b = parse("x\n```rust\nlet a = **1**;\n\nfn f() {}\n```\ny");
        let cb = |l: &str| {
            block(
                BlockKind::CodeBlock {
                    lang: Some("rust".into()),
                },
                vec![t(l)],
            )
        };
        assert_eq!(
            b,
            vec![
                block(BlockKind::Text, vec![t("x")]),
                cb("let a = **1**;"),
                cb(""),
                cb("fn f() {}"),
                block(BlockKind::Text, vec![t("y")]),
            ]
        );
    }

    #[test]
    fn fenced_without_lang() {
        let b = parse("```\na\nb\n```");
        let cb = |l: &str| block(BlockKind::CodeBlock { lang: None }, vec![t(l)]);
        assert_eq!(b, vec![cb("a"), cb("b")]);
        // First line is code when it is not a single word.
        let b = parse("```hello world\nb\n```");
        assert_eq!(b, vec![cb("hello world"), cb("b")]);
    }

    #[test]
    fn fenced_single_line_and_unclosed() {
        assert_eq!(
            parse("```a *b*```"),
            vec![block(BlockKind::CodeBlock { lang: None }, vec![t("a *b*")])]
        );
        assert_eq!(
            parse("```\n*a*"),
            vec![
                block(BlockKind::Text, vec![t("```")]),
                block(BlockKind::Text, vec![ts("a", italic())]),
            ]
        );
    }

    #[test]
    fn quotes() {
        assert_eq!(
            parse("> hi *x*\nno"),
            vec![
                block(BlockKind::Quote, vec![t("hi "), ts("x", italic())]),
                block(BlockKind::Text, vec![t("no")]),
            ]
        );
        assert_eq!(
            parse("a\n>>> b\nc\n\nd"),
            vec![
                block(BlockKind::Text, vec![t("a")]),
                block(BlockKind::Quote, vec![t("b")]),
                block(BlockKind::Quote, vec![t("c")]),
                block(BlockKind::Quote, vec![]),
                block(BlockKind::Quote, vec![t("d")]),
            ]
        );
        assert_eq!(inline(">no space"), vec![t(">no space")]);
    }

    #[test]
    fn headings() {
        for (src, lvl) in [("# a", 1), ("## a", 2), ("### a", 3)] {
            assert_eq!(
                parse(src),
                vec![block(BlockKind::Heading(lvl), vec![t("a")])]
            );
        }
        assert_eq!(inline("#a"), vec![t("#a")]);
        assert_eq!(inline("#### a"), vec![t("#### a")]);
        assert_eq!(
            parse("# **x**"),
            vec![block(BlockKind::Heading(1), vec![ts("x", bold())])]
        );
    }

    #[test]
    fn subtext_and_lists() {
        assert_eq!(
            parse("-# small"),
            vec![block(BlockKind::Subtext, vec![t("small")])]
        );
        assert_eq!(parse("- a"), vec![block(BlockKind::ListItem, vec![t("a")])]);
        assert_eq!(parse("* a"), vec![block(BlockKind::ListItem, vec![t("a")])]);
        assert_eq!(
            parse("  - a"),
            vec![block(BlockKind::ListItem, vec![t("a")])]
        );
        assert_eq!(inline("-a"), vec![t("-a")]);
        assert_eq!(inline("*a*"), vec![ts("a", italic())]);
    }

    #[test]
    fn mentions() {
        assert_eq!(inline("<@123>"), vec![Span::UserMention(123)]);
        assert_eq!(inline("<@!123>"), vec![Span::UserMention(123)]);
        assert_eq!(inline("<#456>"), vec![Span::ChannelMention(456)]);
        assert_eq!(inline("<@&789>"), vec![Span::RoleMention(789)]);
        assert_eq!(
            inline("hi <@1> and @everyone @here!"),
            vec![
                t("hi "),
                Span::UserMention(1),
                t(" and "),
                Span::Everyone,
                t(" "),
                Span::Here,
                t("!"),
            ]
        );
    }

    #[test]
    fn invalid_mentions_are_literal() {
        assert_eq!(inline("<@abc>"), vec![t("<@abc>")]);
        assert_eq!(inline("<@>"), vec![t("<@>")]);
        assert_eq!(inline("<#12x>"), vec![t("<#12x>")]);
        assert_eq!(
            inline("<@99999999999999999999999>"),
            vec![t("<@99999999999999999999999>")]
        );
        assert_eq!(inline("<@1"), vec![t("<@1")]);
    }

    #[test]
    fn emoji() {
        assert_eq!(
            inline("<:smile:123>"),
            vec![Span::Emoji {
                name: "smile".into(),
                id: 123,
                animated: false
            }]
        );
        assert_eq!(
            inline("<a:dance:9>"),
            vec![Span::Emoji {
                name: "dance".into(),
                id: 9,
                animated: true
            }]
        );
        assert_eq!(inline("<:bad:x>"), vec![t("<:bad:x>")]);
        assert_eq!(inline("<::1>"), vec![t("<::1>")]);
    }

    #[test]
    fn timestamps() {
        assert_eq!(
            inline("<t:1618953630>"),
            vec![Span::Timestamp {
                unix: 1618953630,
                format: None
            }]
        );
        assert_eq!(
            inline("<t:1618953630:R>"),
            vec![Span::Timestamp {
                unix: 1618953630,
                format: Some('R')
            }]
        );
        assert_eq!(inline("<t:1:x>"), vec![t("<t:1:x>")]);
        assert_eq!(inline("<t:abc>"), vec![t("<t:abc>")]);
    }

    #[test]
    fn links() {
        assert_eq!(
            inline("see https://a.b/c_d*e. ok"),
            vec![t("see "), Span::Link("https://a.b/c_d*e".into()), t(". ok")]
        );
        assert_eq!(
            inline("(http://x.y/z)"),
            vec![t("("), Span::Link("http://x.y/z".into()), t(")")]
        );
        assert_eq!(
            inline("https://en.wikipedia.org/wiki/A_(b)!?"),
            vec![
                Span::Link("https://en.wikipedia.org/wiki/A_(b)".into()),
                t("!?")
            ]
        );
        assert_eq!(inline("https://"), vec![t("https://")]);
        assert_eq!(inline("http://,"), vec![t("http://,")]);
        assert_eq!(inline("hello"), vec![t("hello")]);
    }

    #[test]
    fn turkish_multibyte() {
        assert_eq!(
            inline("**çok güzel** şey"),
            vec![ts("çok güzel", bold()), t(" şey")]
        );
        assert_eq!(inline("_ığüşöç_"), vec![ts("ığüşöç", italic())]);
        assert_eq!(inline("ç_ğ_ş"), vec![t("ç_ğ_ş")]);
        assert_eq!(
            inline("`ç` <@5> İ"),
            vec![
                Span::Code("ç".into()),
                t(" "),
                Span::UserMention(5),
                t(" İ")
            ]
        );
    }

    #[test]
    fn adjacent_text_is_merged() {
        assert_eq!(inline(r"a\*b\*c"), vec![t("a*b*c")]);
        assert_eq!(inline("a ** b ~ c"), vec![t("a ** b ~ c")]);
        // Literal marker chars inside a styled region merge with its text.
        assert_eq!(inline("**a ~ b**"), vec![ts("a ~ b", bold())]);
        assert_eq!(
            inline("x *y* *z*"),
            vec![t("x "), ts("y", italic()), t(" "), ts("z", italic())]
        );
    }
}
