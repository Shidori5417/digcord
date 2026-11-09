//! Yeniden atanabilir kısayollar: `eylem -> tuş` eşlemesi.
//!
//! Metin düzenleme tuşları (INSERT/COMMAND girişi), açılır pencereler (hızlı geçiş, resim
//! görüntüleyici, silme onayı) ve `Ctrl+C` sabittir. Varsayılanlar eski sabit davranışın aynısıdır.

use std::collections::HashMap;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Tuş dizesi ayrıştırma hatası.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyParseError {
    #[error("boş tuş")]
    Empty,
    #[error("bilinmeyen tuş \"{0}\"")]
    UnknownKey(String),
}

/// Bir tuş ve değiştiriciler (`ctrl+k`, `alt+up`, `G`...). Terminallerin farklı bildirdiği
/// biçimler (büyük harf + SHIFT, `BackTab`...) tek biçime indirgenir.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct KeyChord {
    code: KeyCode,
    mods: KeyModifiers,
}

fn first(mut it: impl Iterator<Item = char>, fallback: char) -> char {
    it.next().unwrap_or(fallback)
}

impl KeyChord {
    pub(crate) fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        let mut mods = mods
            & (KeyModifiers::CONTROL
                | KeyModifiers::ALT
                | KeyModifiers::SHIFT
                | KeyModifiers::SUPER);
        let code = match code {
            KeyCode::Char(c) if c.is_alphabetic() => {
                let shifted = mods.contains(KeyModifiers::SHIFT) || c.is_uppercase();
                if mods.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
                {
                    // Ctrl/Alt ile: harf küçük, büyütme SHIFT ile anlatılır.
                    mods.set(KeyModifiers::SHIFT, shifted);
                    KeyCode::Char(first(c.to_lowercase(), c))
                } else {
                    // Düz harf: büyük/küçük harfin kendisi bilgiyi taşır.
                    mods.remove(KeyModifiers::SHIFT);
                    KeyCode::Char(if shifted {
                        first(c.to_uppercase(), c)
                    } else {
                        c
                    })
                }
            }
            KeyCode::Char(c) => {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::Char(c)
            }
            KeyCode::BackTab => {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::BackTab
            }
            KeyCode::Tab if mods.contains(KeyModifiers::SHIFT) => {
                mods.remove(KeyModifiers::SHIFT);
                KeyCode::BackTab
            }
            other => other,
        };
        Self { code, mods }
    }

    pub(crate) fn of(key: &KeyEvent) -> Self {
        Self::new(key.code, key.modifiers)
    }
}

const MODIFIERS: &[(&str, KeyModifiers)] = &[
    ("ctrl+", KeyModifiers::CONTROL),
    ("control+", KeyModifiers::CONTROL),
    ("alt+", KeyModifiers::ALT),
    ("shift+", KeyModifiers::SHIFT),
    ("super+", KeyModifiers::SUPER),
];

/// `ctrl+k`, `alt+up`, `shift+tab`, `enter`, `esc`, `pgdown`, `f1`, tek karakter...
pub(crate) fn parse_key(text: &str) -> Result<KeyChord, KeyParseError> {
    let mut rest = text.trim();
    if rest.is_empty() {
        return Err(KeyParseError::Empty);
    }
    let mut mods = KeyModifiers::NONE;
    while let Some((prefix, m)) = MODIFIERS.iter().find(|(p, _)| {
        rest.len() > p.len()
            && rest
                .get(..p.len())
                .is_some_and(|h| h.eq_ignore_ascii_case(p))
    }) {
        mods |= *m;
        rest = &rest[prefix.len()..];
    }
    let unknown = || KeyParseError::UnknownKey(text.trim().to_owned());
    let mut chars = rest.chars();
    let code = match (chars.next(), chars.next()) {
        (Some(c), None) => KeyCode::Char(c),
        _ => match rest.to_lowercase().as_str() {
            "enter" | "return" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "tab" => KeyCode::Tab,
            "backtab" => KeyCode::BackTab,
            "space" => KeyCode::Char(' '),
            "plus" => KeyCode::Char('+'),
            "backspace" | "bs" => KeyCode::Backspace,
            "delete" | "del" => KeyCode::Delete,
            "insert" | "ins" => KeyCode::Insert,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "pageup" | "pgup" => KeyCode::PageUp,
            "pagedown" | "pgdown" | "pgdn" => KeyCode::PageDown,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            name => {
                let n = name
                    .strip_prefix('f')
                    .and_then(|n| n.parse::<u8>().ok())
                    .filter(|n| (1..=24).contains(n))
                    .ok_or_else(unknown)?;
                KeyCode::F(n)
            }
        },
    };
    Ok(KeyChord::new(code, mods))
}

/// Eylemin geçerli olduğu yer. `Global` her modda (giriş ekranı hariç) önce bakılır.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Scope {
    Global,
    Normal,
    Select,
}

/// Yeniden atanabilir eylemler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Act {
    // Genel
    ToggleMute,
    ToggleDeaf,
    PushToTalk,
    // NORMAL
    Quit,
    Switcher,
    NextFocus,
    PrevFocus,
    Activate,
    InsertMode,
    SelectMode,
    CommandMode,
    PageDown,
    PageUp,
    ScrollBottom,
    NextUnread,
    PrevUnread,
    // NORMAL + SELECT
    MoveDown,
    MoveUp,
    // SELECT
    Back,
    Reply,
    Edit,
    Delete,
    React,
    Copy,
    OpenMedia,
    Retry,
}

struct Def {
    act: Act,
    name: &'static str,
    scopes: &'static [Scope],
    keys: &'static [&'static str],
}

const fn def(
    act: Act,
    name: &'static str,
    scopes: &'static [Scope],
    keys: &'static [&'static str],
) -> Def {
    Def {
        act,
        name,
        scopes,
        keys,
    }
}

const G: &[Scope] = &[Scope::Global];
const N: &[Scope] = &[Scope::Normal];
const S: &[Scope] = &[Scope::Select];
const NS: &[Scope] = &[Scope::Normal, Scope::Select];

/// Eylem adları, geçerli oldukları yerler ve varsayılan tuşlar (eski sabit davranış).
const ACTIONS: &[Def] = &[
    def(Act::ToggleMute, "toggle_mute", G, &["ctrl+shift+m"]),
    def(Act::ToggleDeaf, "toggle_deaf", G, &["ctrl+shift+d"]),
    def(Act::PushToTalk, "push_to_talk", G, &["ctrl+space"]),
    def(Act::Quit, "quit", N, &["q"]),
    def(Act::Switcher, "switcher", N, &["ctrl+k"]),
    def(Act::NextFocus, "next_focus", N, &["tab"]),
    def(Act::PrevFocus, "prev_focus", N, &["shift+tab"]),
    def(Act::Activate, "activate", N, &["enter", "l"]),
    def(Act::InsertMode, "insert_mode", N, &["i"]),
    def(Act::SelectMode, "select_mode", N, &["v"]),
    def(Act::CommandMode, "command_mode", N, &[":"]),
    def(Act::PageDown, "page_down", N, &["pagedown", "ctrl+d"]),
    def(Act::PageUp, "page_up", N, &["pageup", "ctrl+u"]),
    def(Act::ScrollBottom, "scroll_bottom", N, &["G", "end"]),
    def(Act::NextUnread, "next_unread", N, &["alt+down"]),
    def(Act::PrevUnread, "prev_unread", N, &["alt+up"]),
    def(Act::MoveDown, "move_down", NS, &["j", "down"]),
    def(Act::MoveUp, "move_up", NS, &["k", "up"]),
    def(Act::Back, "back", S, &["esc"]),
    def(Act::Reply, "reply", S, &["r"]),
    def(Act::Edit, "edit", S, &["e"]),
    def(Act::Delete, "delete", S, &["d"]),
    def(Act::React, "react", S, &["+"]),
    def(Act::Copy, "copy", S, &["y"]),
    def(Act::OpenMedia, "open_media", S, &["o"]),
    def(Act::Retry, "retry", S, &["enter"]),
];

/// Tuş → eylem eşlemesi.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    binds: HashMap<(Scope, KeyChord), Act>,
}

impl Default for Keymap {
    fn default() -> Self {
        let mut binds = HashMap::new();
        for d in ACTIONS {
            for key in d.keys {
                // Varsayılan tablo testte doğrulanır; ayrıştırılamayan satır sessizce atlanır.
                if let Ok(chord) = parse_key(key) {
                    for scope in d.scopes {
                        binds.insert((*scope, chord), d.act);
                    }
                }
            }
        }
        Self { binds }
    }
}

impl Keymap {
    /// Tüm eylem adları (belgeler ve testler için).
    pub fn action_names() -> impl Iterator<Item = &'static str> {
        ACTIONS.iter().map(|d| d.name)
    }

    /// Varsayılanlar üzerine `eylem -> tuşlar` geçersiz kılmalarını uygular. Bir eylemin tuşları
    /// verilince varsayılanları tamamen değişir. Bilinmeyen eylem, ayrıştırılamayan tuş ve
    /// çakışmalar uyarı olur (açılış başarısız olmaz); çakışmada son atama geçerlidir.
    pub fn from_overrides(overrides: &[(String, Vec<String>)]) -> (Self, Vec<String>) {
        let mut map = Self::default();
        let mut warnings = Vec::new();
        for (name, keys) in overrides {
            let Some(def) = ACTIONS.iter().find(|d| d.name == name) else {
                warnings.push(format!("bilinmeyen kısayol eylemi: {name}"));
                continue;
            };
            let mut chords = Vec::new();
            for key in keys {
                match parse_key(key) {
                    Ok(chord) => chords.push((key, chord)),
                    Err(e) => warnings.push(format!("keys.{name}: {e}")),
                }
            }
            if chords.is_empty() {
                warnings.push(format!(
                    "keys.{name}: geçerli tuş yok, varsayılan korunuyor"
                ));
                continue;
            }
            map.binds.retain(|_, act| *act != def.act);
            for (key, chord) in chords {
                for scope in def.scopes {
                    let clash: &[Scope] = match scope {
                        Scope::Global => &[Scope::Global, Scope::Normal, Scope::Select],
                        Scope::Normal => &[Scope::Normal, Scope::Global],
                        Scope::Select => &[Scope::Select, Scope::Global],
                    };
                    for other_scope in clash {
                        if let Some(other) = map.binds.remove(&(*other_scope, chord))
                            && other != def.act
                        {
                            let other_name = ACTIONS
                                .iter()
                                .find(|d| d.act == other)
                                .map_or("?", |d| d.name);
                            warnings.push(format!(
                                "kısayol çakışması: \"{key}\" hem \"{other_name}\" hem \"{name}\" \
                                 eylemine atanmış; \"{name}\" geçerli"
                            ));
                        }
                    }
                    map.binds.insert((*scope, chord), def.act);
                }
            }
        }
        (map, warnings)
    }

    pub(crate) fn lookup(&self, scope: Scope, key: &KeyEvent) -> Option<Act> {
        self.binds.get(&(scope, KeyChord::of(key))).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(s: &str) -> KeyChord {
        parse_key(s).unwrap()
    }

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn parses_modifiers_names_and_chars() {
        assert_eq!(
            chord("ctrl+k"),
            KeyChord::new(KeyCode::Char('k'), KeyModifiers::CONTROL)
        );
        assert_eq!(
            chord("Alt+Up"),
            KeyChord::new(KeyCode::Up, KeyModifiers::ALT)
        );
        assert_eq!(chord("shift+tab"), chord("backtab"));
        assert_eq!(
            chord("enter"),
            KeyChord::new(KeyCode::Enter, KeyModifiers::NONE)
        );
        assert_eq!(chord("esc"), chord("escape"));
        assert_eq!(chord("pgdown"), chord("pagedown"));
        assert_eq!(
            chord("f1"),
            KeyChord::new(KeyCode::F(1), KeyModifiers::NONE)
        );
        assert_eq!(
            chord("f24"),
            KeyChord::new(KeyCode::F(24), KeyModifiers::NONE)
        );
        assert_eq!(
            chord("space"),
            KeyChord::new(KeyCode::Char(' '), KeyModifiers::NONE)
        );
        assert_eq!(
            chord(":"),
            KeyChord::new(KeyCode::Char(':'), KeyModifiers::NONE)
        );
        assert_eq!(chord("+"), chord("plus"));
        assert_eq!(
            chord("ctrl++"),
            KeyChord::new(KeyCode::Char('+'), KeyModifiers::CONTROL)
        );
        assert_eq!(
            chord("ctrl+shift+m"),
            KeyChord::new(
                KeyCode::Char('m'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )
        );
        // Büyük harf = shift'li harf.
        assert_eq!(
            chord("G"),
            KeyChord::new(KeyCode::Char('G'), KeyModifiers::NONE)
        );
        assert_eq!(chord("shift+g"), chord("G"));
        assert_eq!(chord("ctrl+M"), chord("ctrl+shift+m"));
    }

    #[test]
    fn rejects_bad_keys() {
        assert_eq!(parse_key(""), Err(KeyParseError::Empty));
        assert_eq!(parse_key("  "), Err(KeyParseError::Empty));
        assert!(matches!(
            parse_key("ctrl+"),
            Err(KeyParseError::UnknownKey(_))
        ));
        assert!(matches!(
            parse_key("hyper"),
            Err(KeyParseError::UnknownKey(_))
        ));
        assert!(matches!(parse_key("f0"), Err(KeyParseError::UnknownKey(_))));
        assert!(matches!(
            parse_key("f25"),
            Err(KeyParseError::UnknownKey(_))
        ));
        assert!(matches!(
            parse_key("ctrl+ctrl"),
            Err(KeyParseError::UnknownKey(_))
        ));
    }

    #[test]
    fn terminal_reports_are_normalized() {
        // Eski terminal: Shift+Tab → BackTab(+SHIFT); büyük harf → Char('G')+SHIFT.
        assert_eq!(
            KeyChord::of(&ev(KeyCode::BackTab, KeyModifiers::SHIFT)),
            chord("shift+tab")
        );
        assert_eq!(
            KeyChord::of(&ev(KeyCode::Char('G'), KeyModifiers::SHIFT)),
            chord("G")
        );
        assert_eq!(
            KeyChord::of(&ev(
                KeyCode::Char('M'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            )),
            chord("ctrl+shift+m")
        );
        assert_eq!(
            KeyChord::of(&ev(KeyCode::Char('+'), KeyModifiers::SHIFT)),
            chord("+")
        );
    }

    #[test]
    fn defaults_parse_and_do_not_clash() {
        let map = Keymap::default();
        let expected: usize = ACTIONS.iter().map(|d| d.keys.len() * d.scopes.len()).sum();
        assert_eq!(
            map.binds.len(),
            expected,
            "varsayılanlar çakışıyor ya da ayrıştırılamıyor"
        );
        let k = |c, m| ev(c, m);
        assert_eq!(
            map.lookup(Scope::Normal, &k(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(Act::Quit)
        );
        assert_eq!(
            map.lookup(Scope::Select, &k(KeyCode::Char('q'), KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            map.lookup(Scope::Select, &k(KeyCode::Down, KeyModifiers::NONE)),
            Some(Act::MoveDown)
        );
        assert_eq!(
            map.lookup(Scope::Normal, &k(KeyCode::Down, KeyModifiers::ALT)),
            Some(Act::NextUnread)
        );
    }

    #[test]
    fn names_are_unique() {
        let names: Vec<_> = Keymap::action_names().collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names.len(), sorted.len());
    }

    fn ov(pairs: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
        pairs
            .iter()
            .map(|(n, k)| ((*n).to_owned(), k.iter().map(|s| (*s).to_owned()).collect()))
            .collect()
    }

    #[test]
    fn override_replaces_default_keys() {
        let (map, warnings) =
            Keymap::from_overrides(&ov(&[("quit", &["x"]), ("switcher", &["ctrl+o"])]));
        assert!(warnings.is_empty(), "{warnings:?}");
        let x = ev(KeyCode::Char('x'), KeyModifiers::NONE);
        let q = ev(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(map.lookup(Scope::Normal, &x), Some(Act::Quit));
        assert_eq!(map.lookup(Scope::Normal, &q), None);
        let ck = ev(KeyCode::Char('k'), KeyModifiers::CONTROL);
        assert_eq!(map.lookup(Scope::Normal, &ck), None);
    }

    #[test]
    fn unknown_action_and_bad_key_warn() {
        let (map, warnings) = Keymap::from_overrides(&ov(&[
            ("fly", &["f"]),
            ("quit", &["ctrl+"]),
            ("copy", &["Y", "???"]),
        ]));
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        assert!(warnings[0].contains("fly"));
        // Geçersiz tuşlu eylem varsayılanını korur; yarım geçerli olan geçerlisini alır.
        let q = ev(KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(map.lookup(Scope::Normal, &q), Some(Act::Quit));
        let y = ev(KeyCode::Char('Y'), KeyModifiers::SHIFT);
        assert_eq!(map.lookup(Scope::Select, &y), Some(Act::Copy));
    }

    #[test]
    fn conflict_warns_and_last_wins() {
        // `j` MoveDown'a aitti; `quit = j` onu alır. Aynı kapsamda ikinci atama da kazanır.
        let (map, warnings) =
            Keymap::from_overrides(&ov(&[("quit", &["j"]), ("insert_mode", &["j"])]));
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings[0].contains("move_down") && warnings[0].contains("quit"));
        assert!(warnings[1].contains("quit") && warnings[1].contains("insert_mode"));
        let j = ev(KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(map.lookup(Scope::Normal, &j), Some(Act::InsertMode));
        // SELECT'te `j` hâlâ MoveDown (kapsam farklı).
        assert_eq!(map.lookup(Scope::Select, &j), Some(Act::MoveDown));
    }

    #[test]
    fn global_action_clashes_with_scoped_bindings() {
        let (map, warnings) = Keymap::from_overrides(&ov(&[("toggle_mute", &["r"])]));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        let r = ev(KeyCode::Char('r'), KeyModifiers::NONE);
        assert_eq!(map.lookup(Scope::Global, &r), Some(Act::ToggleMute));
        assert_eq!(map.lookup(Scope::Select, &r), None);
    }

    #[test]
    fn rebinding_to_own_current_key_is_not_a_conflict() {
        let (_, warnings) =
            Keymap::from_overrides(&ov(&[("quit", &["q"]), ("move_down", &["j", "down"])]));
        assert!(warnings.is_empty(), "{warnings:?}");
    }
}
