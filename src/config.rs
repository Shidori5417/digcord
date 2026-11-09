//! Kullanıcı yapılandırması: `~/.config/digcord/config.toml`.
//!
//! Bilinmeyen anahtarlar hata değil, uyarıdır: belgelenmiş ama henüz uygulanmamış
//! ayarlar kullanıcının dosyasını bozmaz.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

/// Bu sürümde uygulanan bölümler. Bunların dışındaki bölümler uyarı üretir.
const KNOWN_SECTIONS: &[&str] = &["ui", "behavior", "notifications", "client"];

/// Bu sürümde uygulanan `bölüm.anahtar` yolları. Yeni ayar eklenince buraya da eklenmeli.
const KNOWN_KEYS: &[&str] = &[
    "ui.show_member_list",
    "ui.images",
    "ui.image_max_rows",
    "behavior.send_typing",
    "behavior.send_read_receipts",
    "notifications.enabled",
    "notifications.on",
    "client.build_number",
    "client.chrome_major",
];

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub ui: Ui,
    pub behavior: Behavior,
    pub notifications: Notifications,
    pub client: Client,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Ui {
    pub show_member_list: bool,
    pub images: Images,
    /// Bir resmin en fazla satır yüksekliği.
    pub image_max_rows: u16,
}

/// `ui.images`: resim gösterimi.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Images {
    Off,
    Thumbnail,
    Full,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Behavior {
    pub send_typing: bool,
    pub send_read_receipts: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotifyOn {
    Mention,
    Dm,
    All,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Notifications {
    pub enabled: bool,
    pub on: Vec<NotifyOn>,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct Client {
    /// 0 = yerleşik varsayılan.
    pub build_number: u32,
    /// 0 = yerleşik varsayılan.
    pub chrome_major: u32,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            show_member_list: true,
            images: Images::Thumbnail,
            image_max_rows: 10,
        }
    }
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            send_typing: true,
            send_read_receipts: true,
        }
    }
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            enabled: true,
            on: vec![NotifyOn::Mention, NotifyOn::Dm],
        }
    }
}

impl Config {
    /// TUI'ye giden ayarlar.
    pub fn tui_options(&self) -> digcord_tui::Options {
        digcord_tui::Options {
            images: match self.ui.images {
                Images::Off => digcord_tui::ImageMode::Off,
                Images::Thumbnail => digcord_tui::ImageMode::Thumbnail,
                Images::Full => digcord_tui::ImageMode::Full,
            },
            image_max_rows: self.ui.image_max_rows.clamp(1, 100),
        }
    }

    /// TOML metnini ayrıştırır; uyarıları atar (uygulama `load` kullanır).
    #[cfg(test)]
    pub fn parse(text: &str) -> anyhow::Result<Config> {
        parse_with_warnings(text).map(|(config, _)| config)
    }

    /// Varsayılan yol: `dirs::config_dir()/digcord/config.toml`. Yapılandırma dizini yoksa `None`.
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("digcord").join("config.toml"))
    }

    /// Dosyadan yükler. Dosya yoksa varsayılanları döner; okunamaz ya da geçersizse
    /// yolu içeren bir hata döner.
    pub fn load(path: &Path) -> anyhow::Result<(Config, Vec<String>)> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok((Config::default(), Vec::new()));
            }
            Err(err) => {
                return Err(err).with_context(|| format!("{} okunamadı", path.display()));
            }
        };
        parse_with_warnings(&text).with_context(|| format!("{} geçersiz", path.display()))
    }
}

/// TOML metnini ayrıştırır ve tanınmayan ayarları `bilinmeyen ayar: ...` uyarılarıyla döner.
pub fn parse_with_warnings(text: &str) -> anyhow::Result<(Config, Vec<String>)> {
    let config: Config = toml::from_str(text).context("yapılandırma ayrıştırılamadı")?;
    let table: toml::Table = toml::from_str(text).context("yapılandırma ayrıştırılamadı")?;

    let mut warnings = Vec::new();
    collect_unknown_keys(&table, &mut warnings);
    Ok((config, warnings))
}

fn collect_unknown_keys(table: &toml::Table, warnings: &mut Vec<String>) {
    for (section, value) in table {
        let Some(keys) = value.as_table() else {
            warnings.push(format!("bilinmeyen ayar: {section}"));
            continue;
        };
        if !KNOWN_SECTIONS.contains(&section.as_str()) {
            // Tüm bölüm bilinmiyorsa iç anahtarlarını tek tek saymaya gerek yok.
            warnings.push(format!("bilinmeyen ayar: {section}"));
            continue;
        }
        for key in keys.keys() {
            let path = format!("{section}.{key}");
            if !KNOWN_KEYS.contains(&path.as_str()) {
                warnings.push(format!("bilinmeyen ayar: {path}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dokümandaki (docs/TUI_DESIGN.md) örnek yapılandırma.
    const DOCUMENTED_EXAMPLE: &str = r#"
[ui]
keymap = "vim"           # "vim" | "simple"
theme = "default"        # temalar ~/.config/digcord/themes/*.toml
timestamp_format = "%H:%M"
show_member_list = true
images = "thumbnail"     # "off" | "thumbnail" | "full"
image_max_rows = 10

[behavior]
send_typing = true
send_read_receipts = true
message_window = 200      # kanal başına bellekte tutulan mesaj

[notifications]
enabled = true
on = ["mention", "dm"]

[voice]
input_device = "default"
output_device = "default"
mode = "vad"             # "vad" | "ptt"
ptt_key = "ctrl+space"

[client]
# İstemci kimliği profili — bkz. docs/PROTOCOL_NOTES.md
build_number = 0          # 0 = otomatik/varsayılan

[keys]
# "eylem" = "tuş" geçersiz kılmaları
"#;

    #[test]
    fn empty_text_yields_defaults() {
        let (config, warnings) = parse_with_warnings("").expect("boş metin geçerli");
        assert_eq!(config, Config::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn full_valid_file_is_applied_without_warnings() {
        let text = r#"
[ui]
show_member_list = false
images = "full"
image_max_rows = 20

[behavior]
send_typing = false
send_read_receipts = false

[notifications]
enabled = false
on = ["all"]

[client]
build_number = 12345
chrome_major = 130
"#;
        let (config, warnings) = parse_with_warnings(text).expect("geçerli dosya");
        assert!(warnings.is_empty(), "beklenmeyen uyarılar: {warnings:?}");
        assert_eq!(
            config,
            Config {
                ui: Ui {
                    show_member_list: false,
                    images: Images::Full,
                    image_max_rows: 20,
                },
                behavior: Behavior {
                    send_typing: false,
                    send_read_receipts: false,
                },
                notifications: Notifications {
                    enabled: false,
                    on: vec![NotifyOn::All],
                },
                client: Client {
                    build_number: 12345,
                    chrome_major: 130,
                },
            }
        );
    }

    #[test]
    fn bad_image_mode_is_an_error() {
        assert!(Config::parse("[ui]\nimages = \"big\"\n").is_err());
        let opts = Config::parse("[ui]\nimages = \"off\"\nimage_max_rows = 0\n")
            .expect("geçerli")
            .tui_options();
        assert_eq!(opts.images, digcord_tui::ImageMode::Off);
        assert_eq!(opts.image_max_rows, 1);
    }

    #[test]
    fn partial_sections_keep_other_defaults() {
        let config = Config::parse("[ui]\nshow_member_list = false\n").expect("geçerli");
        assert!(!config.ui.show_member_list);
        assert_eq!(config.behavior, Behavior::default());
        assert_eq!(config.notifications, Notifications::default());
        assert_eq!(config.client, Client::default());
    }

    #[test]
    fn wrong_types_are_errors() {
        assert!(Config::parse("[ui]\nshow_member_list = \"yes\"\n").is_err());
        assert!(Config::parse("[client]\nbuild_number = -1\n").is_err());
        assert!(Config::parse("[notifications]\non = \"mention\"\n").is_err());
    }

    #[test]
    fn notify_on_accepts_all_variants() {
        let config = Config::parse("[notifications]\non = [\"mention\", \"dm\", \"all\"]\n")
            .expect("geçerli");
        assert_eq!(
            config.notifications.on,
            vec![NotifyOn::Mention, NotifyOn::Dm, NotifyOn::All]
        );
    }

    #[test]
    fn invalid_enum_value_is_error() {
        assert!(Config::parse("[notifications]\non = [\"server\"]\n").is_err());
    }

    #[test]
    fn unknown_keys_and_sections_become_warnings() {
        let text = "[ui]\ntheme = \"x\"\n\n[voice]\ninput_device = \"default\"\n";
        let (config, warnings) = parse_with_warnings(text).expect("bilinmeyen anahtar hata değil");
        assert_eq!(config, Config::default());
        assert_eq!(
            warnings,
            vec![
                "bilinmeyen ayar: ui.theme".to_string(),
                "bilinmeyen ayar: voice".to_string(),
            ]
        );
    }

    #[test]
    fn documented_example_parses_with_warnings() {
        let (config, warnings) =
            parse_with_warnings(DOCUMENTED_EXAMPLE).expect("belgelenmiş örnek geçerli olmalı");
        assert!(config.ui.show_member_list);
        assert_eq!(
            config.notifications.on,
            vec![NotifyOn::Mention, NotifyOn::Dm]
        );
        assert_eq!(warnings.len(), 6, "uyarılar: {warnings:?}");
        assert_eq!(config.ui.images, Images::Thumbnail);
        assert_eq!(config.ui.image_max_rows, 10);
        for expected in [
            "bilinmeyen ayar: ui.keymap",
            "bilinmeyen ayar: ui.theme",
            "bilinmeyen ayar: ui.timestamp_format",
            "bilinmeyen ayar: behavior.message_window",
            "bilinmeyen ayar: voice",
            "bilinmeyen ayar: keys",
        ] {
            assert!(
                warnings.iter().any(|w| w == expected),
                "{expected} uyarısı yok: {warnings:?}"
            );
        }
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let path = std::env::temp_dir().join(format!(
            "digcord-config-test-{}-missing.toml",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let (config, warnings) = Config::load(&path).expect("eksik dosya hata değil");
        assert_eq!(config, Config::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn load_invalid_file_error_mentions_path() {
        let path = std::env::temp_dir().join(format!(
            "digcord-config-test-{}-invalid.toml",
            std::process::id()
        ));
        std::fs::write(&path, "[ui]\nshow_member_list = \"yes\"\n").expect("test dosyası yazılır");
        let result = Config::load(&path);
        let _ = std::fs::remove_file(&path);
        let err = result.expect_err("geçersiz dosya hata vermeli");
        assert!(
            err.to_string().contains(&path.display().to_string()),
            "hata yolu içermiyor: {err:#}"
        );
    }
}
