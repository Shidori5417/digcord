//! Çok satırlı mesaj yazma editörü: saf mantık + ekran yerleşimi, IO yok.
//!
//! İmleç `(satır, bayt)` olarak tutulur; bayt her zaman char sınırındadır.
//! "Char" = Unicode scalar; grapheme kümeleri ayrıca ele alınmaz.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::text::Line;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Sekme karakterinin yerine konan boşluk sayısı.
const TAB_WIDTH: usize = 4;

/// Çok satırlı metin girdisi.
#[derive(Debug, Clone)]
pub struct Editor {
    /// Her zaman en az bir satır içerir; satırlarda `\n` yoktur.
    lines: Vec<String>,
    row: usize,
    /// `lines[row]` içinde bayt konumu (char sınırı).
    col: usize,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

/// `handle_key` sonucu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOutcome {
    /// Metin veya imleç değişti (yeniden çiz).
    Changed,
    /// Değiştiricisiz Enter: çağıran metni göndermeli.
    Submit,
    /// Editör tuşu işlemedi (çağıran kullanabilir, örn. Esc, ilk satırda Up).
    Ignored,
}

impl Editor {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            row: 0,
            col: 0,
        }
    }

    /// Satırlar `\n` ile birleştirilmiş metin.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Metni değiştirir (yapıştırma kurallarıyla normalize eder), imleç sona gider.
    pub fn set_text(&mut self, text: &str) {
        self.clear();
        self.insert_str(text);
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.lines.push(String::new());
        self.row = 0;
        self.col = 0;
    }

    /// Metin tamamen boşsa `true` (boşluk karakteri boş sayılmaz).
    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    /// Yapıştırma: `\r\n` ve `\r` -> `\n`; `\n` ve `\t` dışındaki kontrol
    /// karakterleri atılır; `\t` -> 4 boşluk.
    pub fn insert_str(&mut self, s: &str) {
        let normalized = s.replace("\r\n", "\n").replace('\r', "\n");
        for c in normalized.chars() {
            match c {
                '\n' => self.newline(),
                '\t' => {
                    for _ in 0..TAB_WIDTH {
                        self.insert_char(' ');
                    }
                }
                c if c.is_control() => {}
                c => self.insert_char(c),
            }
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> EditOutcome {
        if key.kind == KeyEventKind::Release {
            return EditOutcome::Ignored;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);

        match key.code {
            KeyCode::Char(c) if !ctrl && !alt => self.insert_char(c),
            KeyCode::Char(c) if ctrl && !alt => match c.to_ascii_lowercase() {
                'j' => self.newline(),
                'w' => self.delete_prev_word(),
                'u' => self.delete_to_line_start(),
                'k' => self.delete_to_line_end(),
                'a' => self.col = 0,
                'e' => self.col = self.lines[self.row].len(),
                _ => return EditOutcome::Ignored,
            },
            KeyCode::Char(c) if alt && !ctrl => match c.to_ascii_lowercase() {
                'b' => self.word_left(),
                'f' => self.word_right(),
                _ => return EditOutcome::Ignored,
            },
            KeyCode::Enter => {
                if ctrl {
                    return EditOutcome::Ignored;
                }
                if shift || alt {
                    self.newline();
                } else {
                    return EditOutcome::Submit;
                }
            }
            KeyCode::Backspace => {
                if ctrl || alt {
                    self.delete_prev_word();
                } else {
                    self.backspace();
                }
            }
            KeyCode::Delete => self.delete_forward(),
            KeyCode::Left => {
                if ctrl || alt {
                    self.word_left();
                } else {
                    self.move_left();
                }
            }
            KeyCode::Right => {
                if ctrl || alt {
                    self.word_right();
                } else {
                    self.move_right();
                }
            }
            KeyCode::Home => self.col = 0,
            KeyCode::End => self.col = self.lines[self.row].len(),
            KeyCode::Up => {
                if self.row == 0 {
                    return EditOutcome::Ignored;
                }
                self.move_vertical(self.row - 1);
            }
            KeyCode::Down => {
                if self.row + 1 >= self.lines.len() {
                    return EditOutcome::Ignored;
                }
                self.move_vertical(self.row + 1);
            }
            _ => return EditOutcome::Ignored,
        }
        EditOutcome::Changed
    }

    /// İmleçten önceki kelime (son boşluktan imlece) ve başlangıç char sütunu.
    pub fn current_word(&self) -> (String, usize) {
        let before = &self.lines[self.row][..self.col];
        let start = Self::word_start(before);
        (before[start..].to_string(), before[..start].chars().count())
    }

    /// `current_word` kelimesini `replacement` ile değiştirir, imleç sonrasına gelir.
    pub fn replace_current_word(&mut self, replacement: &str) {
        let start = Self::word_start(&self.lines[self.row][..self.col]);
        self.lines[self.row].replace_range(start..self.col, replacement);
        self.col = start + replacement.len();
    }

    /// Yumuşak kaydırmalı görsel satırlar ve imleç `(satır, sütun)`.
    ///
    /// Boşlukta kırılır, uzun kelimeler sert bölünür, hiçbir satır `width`'i aşmaz
    /// (tek istisna: `width` bir geniş karakterden dar ise o karakter tek başına
    /// bir satıra konur). Dolu satırın sonundaki imleç bir sonraki satırın başına geçer.
    pub fn layout(&self, width: u16) -> (Vec<Line<'static>>, (u16, u16)) {
        let width = usize::from(width.max(1));
        let mut out: Vec<Line<'static>> = Vec::new();
        let mut cursor = (0usize, 0usize);
        for (r, line) in self.lines.iter().enumerate() {
            let rows = wrap_line(line, width);
            let base = out.len();
            let mut extra_row = false;
            if r == self.row {
                let idx = rows
                    .iter()
                    .position(|&(_, end)| self.col < end)
                    .unwrap_or(rows.len() - 1);
                let (start, _) = rows[idx];
                let col_w = line[start..self.col].width();
                if idx == rows.len() - 1 && col_w >= width {
                    // Dolu son satır: imleç için boş bir satır ekle.
                    cursor = (base + rows.len(), 0);
                    extra_row = true;
                } else {
                    cursor = (base + idx, col_w);
                }
            }
            for &(s, e) in &rows {
                out.push(Line::from(line[s..e].to_string()));
            }
            if extra_row {
                out.push(Line::from(String::new()));
            }
        }
        (
            out,
            (
                u16::try_from(cursor.0).unwrap_or(u16::MAX),
                u16::try_from(cursor.1).unwrap_or(u16::MAX),
            ),
        )
    }

    /// `layout(width).0.len()` (en az 1).
    pub fn height(&self, width: u16) -> u16 {
        u16::try_from(self.layout(width).0.len().max(1)).unwrap_or(u16::MAX)
    }

    // ---- iç yardımcılar ----

    /// `before` içinde son boşluktan sonraki bayt konumu.
    fn word_start(before: &str) -> usize {
        before
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or(0, |(i, c)| i + c.len_utf8())
    }

    fn insert_char(&mut self, c: char) {
        self.lines[self.row].insert(self.col, c);
        self.col += c.len_utf8();
    }

    fn newline(&mut self) {
        let rest = self.lines[self.row].split_off(self.col);
        self.row += 1;
        self.lines.insert(self.row, rest);
        self.col = 0;
    }

    fn backspace(&mut self) {
        if self.col > 0 {
            let line = &mut self.lines[self.row];
            if let Some(c) = line[..self.col].chars().next_back() {
                self.col -= c.len_utf8();
                line.remove(self.col);
            }
        } else if self.row > 0 {
            let cur = self.lines.remove(self.row);
            self.row -= 1;
            self.col = self.lines[self.row].len();
            self.lines[self.row].push_str(&cur);
        }
    }

    fn delete_forward(&mut self) {
        if self.col < self.lines[self.row].len() {
            self.lines[self.row].remove(self.col);
        } else if self.row + 1 < self.lines.len() {
            let next = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&next);
        }
    }

    fn delete_prev_word(&mut self) {
        if self.col == 0 {
            self.backspace();
            return;
        }
        let start = prev_word_pos(&self.lines[self.row], self.col);
        self.lines[self.row].replace_range(start..self.col, "");
        self.col = start;
    }

    fn delete_to_line_start(&mut self) {
        self.lines[self.row].replace_range(..self.col, "");
        self.col = 0;
    }

    fn delete_to_line_end(&mut self) {
        self.lines[self.row].truncate(self.col);
    }

    fn move_left(&mut self) {
        if let Some(c) = self.lines[self.row][..self.col].chars().next_back() {
            self.col -= c.len_utf8();
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.lines[self.row].len();
        }
    }

    fn move_right(&mut self) {
        if let Some(c) = self.lines[self.row][self.col..].chars().next() {
            self.col += c.len_utf8();
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    fn word_left(&mut self) {
        if self.col == 0 {
            self.move_left();
        } else {
            self.col = prev_word_pos(&self.lines[self.row], self.col);
        }
    }

    fn word_right(&mut self) {
        let line = &self.lines[self.row];
        if self.col >= line.len() {
            self.move_right();
            return;
        }
        let rest = &line[self.col..];
        let after_ws = rest.trim_start();
        let word_len = after_ws.find(char::is_whitespace).unwrap_or(after_ws.len());
        self.col += rest.len() - after_ws.len() + word_len;
    }

    /// Görüntü sütununu koruyarak başka mantıksal satıra geç.
    fn move_vertical(&mut self, target: usize) {
        let want = self.lines[self.row][..self.col].width();
        let line = &self.lines[target];
        let mut w = 0;
        let mut pos = line.len();
        for (i, c) in line.char_indices() {
            let cw = c.width().unwrap_or(0);
            if w + cw > want {
                pos = i;
                break;
            }
            w += cw;
        }
        self.row = target;
        self.col = pos;
    }
}

/// `col`'dan önceki kelimenin başlangıcı: önce boşlukları, sonra kelimeyi atla.
fn prev_word_pos(line: &str, col: usize) -> usize {
    line[..col]
        .trim_end_matches(char::is_whitespace)
        .trim_end_matches(|c: char| !c.is_whitespace())
        .len()
}

/// Bir mantıksal satırı görsel satırlara böler: `(başlangıç, bitiş)` bayt aralıkları.
fn wrap_line(line: &str, width: usize) -> Vec<(usize, usize)> {
    let mut rows = Vec::new();
    let mut start = 0;
    let mut w = 0;
    // Son boşluktan sonraki konum: kırılabilir nokta.
    let mut brk: Option<usize> = None;
    for (i, c) in line.char_indices() {
        let cw = c.width().unwrap_or(0);
        while w + cw > width && w > 0 {
            let at = match brk {
                Some(b) if b > start => b,
                _ => i,
            };
            rows.push((start, at));
            start = at;
            w = line[at..i].width();
            brk = None;
        }
        w += cw;
        if c == ' ' {
            brk = Some(i + c.len_utf8());
        }
    }
    rows.push((start, line.len()));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn km(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }
    fn ctrl(c: char) -> KeyEvent {
        km(KeyCode::Char(c), KeyModifiers::CONTROL)
    }
    fn alt(c: char) -> KeyEvent {
        km(KeyCode::Char(c), KeyModifiers::ALT)
    }
    fn typ(e: &mut Editor, s: &str) {
        for c in s.chars() {
            assert_eq!(e.handle_key(k(KeyCode::Char(c))), EditOutcome::Changed);
        }
    }
    fn with(s: &str) -> Editor {
        let mut e = Editor::new();
        e.set_text(s);
        e
    }
    fn plain(e: &Editor, w: u16) -> Vec<String> {
        e.layout(w)
            .0
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn types_ascii_turkish_emoji() {
        let mut e = Editor::new();
        assert!(e.is_empty());
        typ(&mut e, "hi çğıöşü 😀");
        assert_eq!(e.text(), "hi çğıöşü 😀");
        assert!(!e.is_empty());
        e.handle_key(k(KeyCode::Backspace));
        e.handle_key(k(KeyCode::Backspace));
        assert_eq!(e.text(), "hi çğıöşü");
        e.handle_key(k(KeyCode::Left));
        e.handle_key(k(KeyCode::Delete));
        assert_eq!(e.text(), "hi çğıöş");
    }

    #[test]
    fn shift_char_inserts_and_whitespace_is_not_empty() {
        let mut e = Editor::new();
        e.handle_key(km(KeyCode::Char('A'), KeyModifiers::SHIFT));
        assert_eq!(e.text(), "A");
        e.clear();
        e.handle_key(k(KeyCode::Char(' ')));
        assert!(!e.is_empty());
    }

    #[test]
    fn release_ignored_repeat_handled() {
        let mut e = Editor::new();
        let mut rel = k(KeyCode::Char('a'));
        rel.kind = KeyEventKind::Release;
        assert_eq!(e.handle_key(rel), EditOutcome::Ignored);
        let mut rep = k(KeyCode::Char('a'));
        rep.kind = KeyEventKind::Repeat;
        assert_eq!(e.handle_key(rep), EditOutcome::Changed);
        assert_eq!(e.text(), "a");
    }

    #[test]
    fn enter_variants() {
        let mut e = with("ab");
        assert_eq!(e.handle_key(k(KeyCode::Enter)), EditOutcome::Submit);
        assert_eq!(e.text(), "ab");
        assert_eq!(
            e.handle_key(km(KeyCode::Enter, KeyModifiers::SHIFT)),
            EditOutcome::Changed
        );
        assert_eq!(
            e.handle_key(km(KeyCode::Enter, KeyModifiers::ALT)),
            EditOutcome::Changed
        );
        assert_eq!(e.handle_key(ctrl('j')), EditOutcome::Changed);
        assert_eq!(e.text(), "ab\n\n\n");
        assert_eq!(
            e.handle_key(km(KeyCode::Enter, KeyModifiers::CONTROL)),
            EditOutcome::Ignored
        );
    }

    #[test]
    fn newline_splits_line_at_cursor() {
        let mut e = with("abçd");
        e.handle_key(k(KeyCode::Left));
        e.handle_key(k(KeyCode::Left));
        e.handle_key(km(KeyCode::Enter, KeyModifiers::SHIFT));
        assert_eq!(e.text(), "ab\nçd");
        typ(&mut e, "X");
        assert_eq!(e.text(), "ab\nXçd");
    }

    #[test]
    fn unhandled_keys_ignored() {
        let mut e = with("a");
        for key in [
            k(KeyCode::Esc),
            k(KeyCode::Tab),
            k(KeyCode::F(1)),
            k(KeyCode::PageUp),
            ctrl('z'),
            alt('x'),
        ] {
            assert_eq!(e.handle_key(key), EditOutcome::Ignored, "{key:?}");
        }
        assert_eq!(e.text(), "a");
    }

    #[test]
    fn left_right_cross_lines() {
        let mut e = with("ab\ncd");
        for _ in 0..2 {
            e.handle_key(k(KeyCode::Left));
        }
        assert_eq!((e.row, e.col), (1, 0));
        e.handle_key(k(KeyCode::Left));
        assert_eq!((e.row, e.col), (0, 2));
        e.handle_key(k(KeyCode::Right));
        assert_eq!((e.row, e.col), (1, 0));
        let mut e = with("");
        assert_eq!(e.handle_key(k(KeyCode::Left)), EditOutcome::Changed);
        assert_eq!((e.row, e.col), (0, 0));
    }

    #[test]
    fn multibyte_movement() {
        let mut e = with("ç😀ğ");
        e.handle_key(k(KeyCode::Left));
        e.handle_key(k(KeyCode::Left));
        assert_eq!(e.col, "ç".len());
        e.handle_key(k(KeyCode::Right));
        assert_eq!(e.col, "ç😀".len());
    }

    #[test]
    fn word_jumps() {
        let mut e = with("foo  bar baz");
        e.handle_key(km(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(e.col, 9);
        e.handle_key(alt('b'));
        assert_eq!(e.col, 5);
        e.handle_key(km(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(e.col, 0);
        e.handle_key(km(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(e.col, 3);
        e.handle_key(alt('f'));
        assert_eq!(e.col, 8);
        e.handle_key(alt('f'));
        assert_eq!(e.col, 12);
    }

    #[test]
    fn word_jumps_cross_lines() {
        let mut e = with("ab\ncd");
        e.handle_key(km(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!((e.row, e.col), (1, 0));
        e.handle_key(km(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!((e.row, e.col), (0, 2));
        e.handle_key(km(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!((e.row, e.col), (1, 0));
        e.handle_key(km(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!((e.row, e.col), (1, 2));
    }

    #[test]
    fn home_end() {
        let mut e = with("ab\ncdé");
        e.handle_key(k(KeyCode::Home));
        assert_eq!((e.row, e.col), (1, 0));
        e.handle_key(k(KeyCode::End));
        assert_eq!(e.col, "cdé".len());
        e.handle_key(ctrl('a'));
        assert_eq!(e.col, 0);
        e.handle_key(ctrl('e'));
        assert_eq!(e.col, "cdé".len());
    }

    #[test]
    fn backspace_delete_join_lines() {
        let mut e = with("ab\ncd");
        e.handle_key(k(KeyCode::Home));
        e.handle_key(k(KeyCode::Backspace));
        assert_eq!(e.text(), "abcd");
        assert_eq!((e.row, e.col), (0, 2));
        e.handle_key(km(KeyCode::Enter, KeyModifiers::SHIFT));
        e.handle_key(k(KeyCode::Left));
        e.handle_key(k(KeyCode::Delete));
        assert_eq!(e.text(), "abcd");
        assert_eq!((e.row, e.col), (0, 2));
        let mut e = with("x");
        e.handle_key(k(KeyCode::Delete));
        e.handle_key(k(KeyCode::Home));
        e.handle_key(k(KeyCode::Backspace));
        assert_eq!(e.text(), "x");
    }

    #[test]
    fn ctrl_w_variants() {
        let mut e = with("foo bar  ");
        e.handle_key(ctrl('w'));
        assert_eq!(e.text(), "foo ");
        e.handle_key(km(KeyCode::Backspace, KeyModifiers::CONTROL));
        assert_eq!(e.text(), "");
        let mut e = with("çay şeker");
        e.handle_key(km(KeyCode::Backspace, KeyModifiers::ALT));
        assert_eq!(e.text(), "çay ");
        let mut e = with("a\nb");
        e.handle_key(k(KeyCode::Home));
        e.handle_key(ctrl('w'));
        assert_eq!(e.text(), "ab");
    }

    #[test]
    fn ctrl_u_and_k() {
        let mut e = with("one\ntwo three");
        for _ in 0..5 {
            e.handle_key(k(KeyCode::Left));
        }
        e.handle_key(ctrl('k'));
        assert_eq!(e.text(), "one\ntwo ");
        e.handle_key(ctrl('u'));
        assert_eq!(e.text(), "one\n");
        assert_eq!((e.row, e.col), (1, 0));
        let mut e = with("abc def");
        e.handle_key(alt('b'));
        e.handle_key(ctrl('u'));
        assert_eq!(e.text(), "def");
        e.handle_key(ctrl('k'));
        assert_eq!(e.text(), "");
    }

    #[test]
    fn up_down_edges_ignored() {
        let mut e = with("ab");
        assert_eq!(e.handle_key(k(KeyCode::Up)), EditOutcome::Ignored);
        assert_eq!(e.handle_key(k(KeyCode::Down)), EditOutcome::Ignored);
        let mut e = with("a\nb");
        assert_eq!(e.handle_key(k(KeyCode::Down)), EditOutcome::Ignored);
        assert_eq!(e.handle_key(k(KeyCode::Up)), EditOutcome::Changed);
        assert_eq!(e.handle_key(k(KeyCode::Up)), EditOutcome::Ignored);
        assert_eq!(e.handle_key(k(KeyCode::Down)), EditOutcome::Changed);
    }

    #[test]
    fn up_down_keep_display_column() {
        let mut e = with("abcdef\nxy\n日本語ab");
        // Sonda sütun genişliği 8; Up ile "xy" sonuna kısaltılır.
        e.handle_key(k(KeyCode::Up));
        assert_eq!((e.row, e.col), (1, 2));
        e.handle_key(k(KeyCode::Up));
        assert_eq!((e.row, e.col), (0, 2));
        e.handle_key(k(KeyCode::Home));
        for _ in 0..3 {
            e.handle_key(k(KeyCode::Right));
        }
        e.handle_key(k(KeyCode::Down));
        assert_eq!((e.row, e.col), (1, 2));
        // Sütun 3: bir geniş karakter (2) sığar, ikincisi sığmaz.
        let mut e = with("abc\n日本語");
        e.handle_key(k(KeyCode::Up));
        e.handle_key(k(KeyCode::End));
        e.handle_key(k(KeyCode::Down));
        assert_eq!(e.col, "日".len());
    }

    #[test]
    fn paste_normalization() {
        let mut e = Editor::new();
        e.insert_str("a\r\nb\rc\td\x07e\x1b[0mf\n");
        assert_eq!(e.text(), "a\nb\nc    de[0mf\n");
        assert_eq!((e.row, e.col), (3, 0));
        e.set_text("x\r\ny");
        assert_eq!(e.text(), "x\ny");
        assert_eq!((e.row, e.col), (1, 1));
        e.insert_str("çş😀");
        assert_eq!(e.text(), "x\nyçş😀");
    }

    #[test]
    fn set_text_and_clear() {
        let mut e = with("abc\ndef");
        assert_eq!((e.row, e.col), (1, 3));
        e.clear();
        assert!(e.is_empty());
        assert_eq!(e.text(), "");
        assert_eq!(e.height(10), 1);
    }

    #[test]
    fn current_word_cases() {
        let e = with("");
        assert_eq!(e.current_word(), (String::new(), 0));
        let e = with("@ali");
        assert_eq!(e.current_word(), ("@ali".to_string(), 0));
        let e = with("merhaba @ali");
        assert_eq!(e.current_word(), ("@ali".to_string(), 8));
        let e = with("merhaba ");
        assert_eq!(e.current_word(), (String::new(), 8));
        let e = with("çok :şmi");
        assert_eq!(e.current_word(), (":şmi".to_string(), 4));
        let e = with("önce\n#gen");
        assert_eq!(e.current_word(), ("#gen".to_string(), 0));
        let mut e = with("hello wörld");
        e.handle_key(k(KeyCode::Left));
        e.handle_key(k(KeyCode::Left));
        assert_eq!(e.current_word(), ("wör".to_string(), 6));
    }

    #[test]
    fn replace_current_word_cases() {
        let mut e = with("@ali");
        e.replace_current_word("<@123> ");
        assert_eq!(e.text(), "<@123> ");
        assert_eq!(e.col, "<@123> ".len());
        let mut e = with("selam :şm");
        e.replace_current_word("😀");
        assert_eq!(e.text(), "selam 😀");
        typ(&mut e, "!");
        assert_eq!(e.text(), "selam 😀!");
        let mut e = with("a #ge son");
        for _ in 0..4 {
            e.handle_key(k(KeyCode::Left));
        }
        assert_eq!(e.current_word().0, "#ge");
        e.replace_current_word("#genel");
        assert_eq!(e.text(), "a #genel son");
        assert_eq!(e.col, "a #genel".len());
        let mut e = with("x ");
        e.replace_current_word("y");
        assert_eq!(e.text(), "x y");
    }

    #[test]
    fn layout_simple() {
        let e = with("hello world");
        let (lines, cur) = e.layout(20);
        assert_eq!(lines.len(), 1);
        assert_eq!(cur, (0, 11));
        assert_eq!(plain(&e, 6), vec!["hello ", "world"]);
        assert_eq!(e.layout(6).1, (1, 5));
        assert_eq!(e.height(6), 2);
    }

    #[test]
    fn layout_hard_break() {
        let e = with("abcdefghij");
        assert_eq!(plain(&e, 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(e.layout(4).1, (2, 2));
    }

    #[test]
    fn layout_cursor_full_width_goes_next_row() {
        let e = with("abcd");
        let (lines, cur) = e.layout(4);
        assert_eq!(cur, (1, 0));
        assert_eq!(lines.len(), 2);
        assert_eq!(e.height(4), 2);
        let mut e = with("abcd");
        e.handle_key(k(KeyCode::Left));
        assert_eq!(e.layout(4).1, (0, 3));
        assert_eq!(e.height(4), 1);
    }

    #[test]
    fn layout_cursor_at_wrap_boundary() {
        let mut e = with("abcdef");
        for _ in 0..2 {
            e.handle_key(k(KeyCode::Left));
        }
        // "abcd|ef": sınırdaki imleç sonraki satırın başı
        assert_eq!(e.layout(4).1, (1, 0));
        e.handle_key(k(KeyCode::Left));
        assert_eq!(e.layout(4).1, (0, 3));
    }

    #[test]
    fn layout_multiline_cursor_row_offset() {
        let mut e = with("abcdef\nxy");
        assert_eq!(plain(&e, 4), vec!["abcd", "ef", "xy"]);
        assert_eq!(e.layout(4).1, (2, 2));
        e.handle_key(k(KeyCode::Up));
        assert_eq!(e.layout(4).1, (0, 2));
        let e = with("a\n\nb");
        assert_eq!(plain(&e, 5), vec!["a", "", "b"]);
    }

    #[test]
    fn layout_wide_chars_not_split() {
        let e = with("日本語");
        assert_eq!(plain(&e, 3), vec!["日", "本", "語"]);
        assert_eq!(plain(&e, 4), vec!["日本", "語"]);
        assert_eq!(e.layout(4).1, (1, 2));
        let e = with("a😀b😀");
        assert_eq!(plain(&e, 2), vec!["a", "😀", "b", "😀", ""]);
        assert_eq!(e.layout(2).1, (4, 0));
    }

    #[test]
    fn layout_width_one_and_zero() {
        let e = with("abc");
        assert_eq!(plain(&e, 1), vec!["a", "b", "c", ""]);
        assert_eq!(e.layout(1).1, (3, 0));
        assert_eq!(e.layout(0).0.len(), e.layout(1).0.len());
        let e = with("日a");
        assert_eq!(plain(&e, 1), vec!["日", "a", ""]);
    }

    #[test]
    fn layout_never_exceeds_width() {
        let texts = [
            "",
            "a",
            "hello world this is a fairly long chat message with spaces",
            "supercalifragilisticexpialidocious",
            "  leading and   multiple   spaces  ",
            "çğıöşü İstanbul'da güzel bir gün 🙂 evet",
            "日本語のテキストとenglishが混ざっている😀😀",
            "satır bir\nsatır iki çok uzun bir satır\n\nson",
            "e\u{301}e\u{301} zero width \u{200b} marks",
            "a b c d e f g h i j k l m n o p",
        ];
        for text in texts {
            let e = with(text);
            for width in 1u16..=40 {
                let (lines, (cr, cc)) = e.layout(width);
                assert_eq!(usize::from(e.height(width)), lines.len().max(1));
                assert!(!lines.is_empty());
                let rows = plain(&e, width);
                for row in &rows {
                    let has_wide = row.chars().any(|c| c.width().unwrap_or(0) > 1);
                    if !(width == 1 && has_wide) {
                        assert!(
                            row.width() <= usize::from(width),
                            "w={width} text={text:?} row={row:?}"
                        );
                    }
                }
                assert!(usize::from(cr) < rows.len(), "w={width} {text:?}");
                assert!(usize::from(cc) <= usize::from(width).max(2));
                // Satırlar birleşince metin geri elde edilir.
                let joined: String = rows.concat();
                assert_eq!(joined, text.replace('\n', ""), "w={width} {text:?}");
            }
        }
    }

    #[test]
    fn cursor_every_position_in_bounds() {
        let mut e = with("çok uzun bir mesaj 😀 var\nikinci satır");
        loop {
            for w in 1u16..=12 {
                let (lines, (r, c)) = e.layout(w);
                assert!(
                    usize::from(r) < lines.len(),
                    "w={w} pos=({},{})",
                    e.row,
                    e.col
                );
                assert!(usize::from(c) <= usize::from(w).max(2));
            }
            if (e.row, e.col) == (0, 0) {
                break;
            }
            e.handle_key(k(KeyCode::Left));
        }
    }

    #[test]
    fn height_counts_rows() {
        let e = with("one two three four");
        assert_eq!(e.height(9), 3); // "one two ", "three ", "four"
        assert_eq!(e.height(100), 1);
        assert_eq!(Editor::new().height(5), 1);
    }
}
