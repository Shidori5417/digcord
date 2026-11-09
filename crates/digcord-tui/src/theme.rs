//! Renk teması: anlamsal "yuva"lar. `Theme::default()` yerleşik görünümdür; kullanıcı
//! temaları (`~/.config/digcord/themes/<ad>.toml`) ikili tarafından okunur ve yuvaları
//! [`Theme::set`] ile doldurur. Bu modülde disk/ayrıştırıcı yok (saf veri).
//!
//! Yuva değeri `Color::Reset` ise "bu yuva için renk ayarlama" demektir: metin yuvalarında
//! terminalin varsayılan rengi kullanılır; `dim`, `timestamp` ve `quote` yuvalarında ise
//! (yerleşik görünüm) renk yerine soluk (DIM) biçim uygulanır.

use std::str::FromStr;

use ratatui::style::{Color, Modifier, Style};

/// Tema yuvası ayarlanırken oluşan hatalar.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ThemeError {
    #[error("bilinmeyen tema yuvası: {0}")]
    UnknownSlot(String),
    #[error("{slot}: geçersiz renk \"{value}\" (\"#rrggbb\", \"red\", \"darkgray\", \"reset\"...)")]
    InvalidColor { slot: String, value: String },
}

macro_rules! theme {
    ($( $(#[$doc:meta])* $field:ident = $default:expr ),+ $(,)?) => {
        /// Arayüzün tüm renkleri.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct Theme {
            $( $(#[$doc])* pub $field: Color, )+
        }

        impl Theme {
            /// Yerleşik tema.
            pub const DEFAULT: Theme = Theme { $( $field: $default, )+ };

            /// Tüm yuva adları (TOML anahtarları).
            pub const SLOTS: &'static [&'static str] = &[ $( stringify!($field), )+ ];

            fn slot_mut(&mut self, name: &str) -> Option<&mut Color> {
                match name {
                    $( stringify!($field) => Some(&mut self.$field), )+
                    _ => None,
                }
            }
        }
    };
}

theme! {
    /// Tüm ekranın arka planı (`reset`: terminalin arka planı).
    background = Color::Reset,
    /// Varsayılan metin rengi.
    text = Color::Reset,
    /// Odakta olmayan panel çerçevesi.
    border = Color::Reset,
    /// Odaktaki panel/pencere çerçevesi.
    border_focused = Color::Cyan,
    /// Mesaj yazarı adı.
    author = Color::Cyan,
    /// Saatler (`reset`: soluk).
    timestamp = Color::Reset,
    /// İkincil/soluk metin (`reset`: soluk).
    dim = Color::Reset,
    /// Alıntı satırı işareti (`reset`: soluk).
    quote = Color::Reset,
    /// Kullanıcı ve kanal anmaları.
    mention = Color::LightBlue,
    /// Rol, `@everyone` ve `@here` anmaları.
    mention_role = Color::LightMagenta,
    /// Seni anan mesajdaki anmanın arka planı.
    mention_me_bg = Color::Rgb(60, 50, 0),
    /// Bağlantılar.
    link = Color::Cyan,
    /// Bağlantılı embed başlığı ve kendi tepkilerin.
    highlight = Color::LightBlue,
    /// Satır içi kod ve kod blokları.
    code = Color::Yellow,
    /// Özel emoji adları (`:ad:`).
    emoji = Color::Yellow,
    /// Gizli (spoiler) metin.
    spoiler = Color::DarkGray,
    /// Seçili mesajın arka planı.
    selection_bg = Color::Rgb(45, 45, 60),
    /// Tamamlama penceresi aday arka planı.
    popup_bg = Color::Rgb(30, 30, 40),
    /// Kanal adları.
    channel = Color::Gray,
    /// Okunmamış kanal/sunucu (kalın; `reset`: sadece kalın).
    unread = Color::Reset,
    /// Açık kanal ve sunucu.
    current = Color::Green,
    /// Ses kanalında konuşan kişi.
    voice_speaking = Color::Green,
    /// Mention sayacı rozeti.
    badge = Color::Red,
    /// Hatalar.
    error = Color::Red,
    /// Uyarılar ve bildirimler.
    warning = Color::Yellow,
    /// Başarı/bağlı durumu.
    success = Color::Green,
    /// Durum çubuğu metni.
    status_fg = Color::Reset,
    /// Durum çubuğu arka planı.
    status_bg = Color::Reset,
}

impl Default for Theme {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Theme {
    /// Bir yuvayı metinden ayarlar: `"#rrggbb"`, ANSI adı (`red`, `darkgray`, `lightblue`...),
    /// 0-255 indeks ya da `reset`.
    pub fn set(&mut self, slot: &str, value: &str) -> Result<(), ThemeError> {
        if !Self::SLOTS.contains(&slot) {
            return Err(ThemeError::UnknownSlot(slot.to_owned()));
        }
        let color = Color::from_str(value.trim()).map_err(|_| ThemeError::InvalidColor {
            slot: slot.to_owned(),
            value: value.to_owned(),
        })?;
        if let Some(c) = self.slot_mut(slot) {
            *c = color;
        }
        Ok(())
    }

    // ------------------------------------------------------------ stil yardımcıları

    /// `Reset` → renk yok; aksi halde ön plan rengi.
    fn fg(color: Color) -> Style {
        match color {
            Color::Reset => Style::new(),
            c => Style::new().fg(c),
        }
    }

    /// `Reset` → soluk; aksi halde renk.
    fn muted(color: Color) -> Style {
        match color {
            Color::Reset => Style::new().add_modifier(Modifier::DIM),
            c => Style::new().fg(c),
        }
    }

    /// Tüm ekranın temel stili (arka plan + metin).
    pub(crate) fn base(&self) -> Style {
        let mut s = Self::fg(self.text);
        if self.background != Color::Reset {
            s = s.bg(self.background);
        }
        s
    }

    pub(crate) fn dim(&self) -> Style {
        Self::muted(self.dim)
    }
    pub(crate) fn timestamp(&self) -> Style {
        Self::muted(self.timestamp)
    }
    pub(crate) fn quote(&self) -> Style {
        Self::muted(self.quote)
    }
    pub(crate) fn author(&self) -> Style {
        Self::fg(self.author).add_modifier(Modifier::BOLD)
    }
    pub(crate) fn code(&self) -> Style {
        Self::fg(self.code)
    }
    pub(crate) fn emoji(&self) -> Style {
        Self::fg(self.emoji)
    }
    pub(crate) fn spoiler(&self) -> Style {
        Self::fg(self.spoiler)
    }
    pub(crate) fn link(&self) -> Style {
        Self::fg(self.link).add_modifier(Modifier::UNDERLINED)
    }
    pub(crate) fn mention(&self) -> Style {
        Self::fg(self.mention)
    }
    pub(crate) fn mention_role(&self) -> Style {
        Self::fg(self.mention_role)
    }
    pub(crate) fn highlight(&self) -> Style {
        Self::fg(self.highlight)
    }
    pub(crate) fn channel(&self) -> Style {
        Self::fg(self.channel)
    }
    pub(crate) fn unread(&self) -> Style {
        Self::fg(self.unread).add_modifier(Modifier::BOLD)
    }
    pub(crate) fn current(&self) -> Style {
        Self::fg(self.current)
    }
    pub(crate) fn speaking(&self) -> Style {
        Self::fg(self.voice_speaking).add_modifier(Modifier::BOLD)
    }
    pub(crate) fn badge(&self) -> Style {
        Self::fg(self.badge).add_modifier(Modifier::BOLD)
    }
    pub(crate) fn error(&self) -> Style {
        Self::fg(self.error)
    }
    pub(crate) fn warning(&self) -> Style {
        Self::fg(self.warning)
    }
    pub(crate) fn success(&self) -> Style {
        Self::fg(self.success)
    }
    pub(crate) fn selection_bg(&self) -> Style {
        Style::new().bg(self.selection_bg)
    }
    pub(crate) fn popup_bg(&self) -> Style {
        Style::new().bg(self.popup_bg)
    }
    pub(crate) fn border_style(&self, focused: bool) -> Style {
        Self::fg(if focused {
            self.border_focused
        } else {
            self.border
        })
    }
    pub(crate) fn status(&self) -> Style {
        let mut s = Self::fg(self.status_fg);
        if self.status_bg != Color::Reset {
            s = s.bg(self.status_bg);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_names_and_reset() {
        let mut t = Theme::default();
        t.set("author", "#ff8000").unwrap();
        assert_eq!(t.author, Color::Rgb(255, 128, 0));
        t.set("error", "darkgray").unwrap();
        assert_eq!(t.error, Color::DarkGray);
        t.set("code", "LightBlue").unwrap();
        assert_eq!(t.code, Color::LightBlue);
        t.set("border_focused", "reset").unwrap();
        assert_eq!(t.border_focused, Color::Reset);
        t.set("link", "42").unwrap();
        assert_eq!(t.link, Color::Indexed(42));
    }

    #[test]
    fn errors_name_the_slot_and_value() {
        let mut t = Theme::default();
        assert_eq!(
            t.set("author", "#12345"),
            Err(ThemeError::InvalidColor {
                slot: "author".into(),
                value: "#12345".into()
            })
        );
        assert!(t.set("author", "mor").is_err());
        assert_eq!(
            t.set("nonexistent", "red"),
            Err(ThemeError::UnknownSlot("nonexistent".into()))
        );
        // Hatalı çağrı temayı değiştirmez.
        assert_eq!(t, Theme::default());
    }

    #[test]
    fn every_slot_is_settable() {
        let mut t = Theme::default();
        for slot in Theme::SLOTS {
            t.set(slot, "#010203").unwrap();
        }
        assert_eq!(t.status_bg, Color::Rgb(1, 2, 3));
        assert_eq!(t.background, Color::Rgb(1, 2, 3));
    }

    #[test]
    fn default_styles_match_the_builtin_look() {
        let t = Theme::default();
        assert_eq!(t.dim(), Style::new().add_modifier(Modifier::DIM));
        assert_eq!(t.unread(), Style::new().add_modifier(Modifier::BOLD));
        assert_eq!(t.border_style(false), Style::new());
        assert_eq!(t.border_style(true), Style::new().fg(Color::Cyan));
        assert_eq!(t.base(), Style::new());
        assert_eq!(t.status(), Style::new());
        // `dim` bir renge ayarlanınca soluk biçim yerine renk kullanılır.
        let mut t = Theme::default();
        t.set("dim", "gray").unwrap();
        assert_eq!(t.dim(), Style::new().fg(Color::Gray));
    }
}
