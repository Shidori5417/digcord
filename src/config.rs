//! Kullanıcı yapılandırması: `~/.config/digcord/config.toml`.
//!
//! Bilinmeyen anahtarlar hata değil, uyarıdır: belgelenmiş ama henüz uygulanmamış
//! ayarlar kullanıcının dosyasını bozmaz.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context;
use digcord_tui::{SettingItem, SettingKind, SettingValue};
use serde::Deserialize;

/// Bu sürümde uygulanan bölümler. Bunların dışındaki bölümler uyarı üretir.
const KNOWN_SECTIONS: &[&str] = &["ui", "behavior", "notifications", "voice", "client", "keys"];

/// Bu sürümde uygulanan `bölüm.anahtar` yolları. Yeni ayar eklenince buraya da eklenmeli.
const KNOWN_KEYS: &[&str] = &[
    "ui.theme",
    "ui.show_member_list",
    "ui.images",
    "ui.image_max_rows",
    "behavior.send_typing",
    "behavior.send_read_receipts",
    "notifications.enabled",
    "notifications.on",
    "voice.mode",
    "voice.jitter_ms",
    "client.build_number",
    "client.chrome_major",
];

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub ui: Ui,
    pub behavior: Behavior,
    pub notifications: Notifications,
    pub voice: Voice,
    pub client: Client,
    /// `[keys]`: `eylem = "tuş"` (ya da tuş listesi) geçersiz kılmaları; bkz. `key_overrides`.
    pub keys: BTreeMap<String, toml::Value>,
}

/// `voice.mode`: ses algılama ya da bas-konuş (Ctrl+Space; kitty klavye protokolü gerekir).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VoiceMode {
    Vad,
    Ptt,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Voice {
    pub mode: VoiceMode,
    /// Çalmaya başlamadan önce biriktirilen ses (ms; 20'nin katına yuvarlanır).
    pub jitter_ms: u32,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            mode: VoiceMode::Vad,
            jitter_ms: 60,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Ui {
    /// Tema adı (`~/.config/digcord/themes/<ad>.toml`); `default` yerleşik.
    pub theme: String,
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

impl Images {
    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Thumbnail => "thumbnail",
            Self::Full => "full",
        }
    }
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

impl NotifyOn {
    fn as_str(self) -> &'static str {
        match self {
            Self::Mention => "mention",
            Self::Dm => "dm",
            Self::All => "all",
        }
    }
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
            theme: crate::theme::DEFAULT_NAME.to_owned(),
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
    /// TUI'ye giden ayarlar ve (kısayol tablosundan gelen) uyarılar.
    pub fn tui_options(&self, theme: digcord_tui::Theme) -> (digcord_tui::Options, Vec<String>) {
        let (overrides, mut warnings) = self.key_overrides();
        let (keys, key_warnings) = digcord_tui::Keymap::from_overrides(&overrides);
        warnings.extend(key_warnings);
        let options = digcord_tui::Options {
            images: match self.ui.images {
                Images::Off => digcord_tui::ImageMode::Off,
                Images::Thumbnail => digcord_tui::ImageMode::Thumbnail,
                Images::Full => digcord_tui::ImageMode::Full,
            },
            image_max_rows: self.ui.image_max_rows.clamp(1, 100),
            push_to_talk: self.voice.mode == VoiceMode::Ptt,
            theme,
            keys,
            settings: self.setting_items(),
        };
        (options, warnings)
    }

    /// `[keys]` tablosu: `(eylem, tuşlar)` çiftleri (eylem adına göre sıralı) ve
    /// değeri metin ya da metin listesi olmayanlar için uyarılar.
    pub fn key_overrides(&self) -> (Vec<(String, Vec<String>)>, Vec<String>) {
        let mut out = Vec::new();
        let mut warnings = Vec::new();
        for (action, value) in &self.keys {
            match value {
                toml::Value::String(key) => out.push((action.clone(), vec![key.clone()])),
                toml::Value::Array(list) if list.iter().all(toml::Value::is_str) => {
                    let keys = list
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect();
                    out.push((action.clone(), keys));
                }
                _ => warnings.push(format!(
                    "keys.{action}: tuş metin (\"ctrl+k\") ya da metin listesi olmalı"
                )),
            }
        }
        (out, warnings)
    }

    /// `:settings` ekranının satırları. Anahtarlar `KNOWN_KEYS` ile uyumlu olmalı (testte denetlenir).
    pub fn setting_items(&self) -> Vec<SettingItem> {
        use SettingKind::{Bool, Choice, Int, ReadOnly};
        let text = |s: &str| SettingValue::Str(s.to_owned());
        let texts = |opts: &[&str]| Choice(opts.iter().map(|s| text(s)).collect());
        let list =
            |opts: &[&str]| SettingValue::List(opts.iter().map(|s| (*s).to_owned()).collect());
        let on = SettingValue::List(
            self.notifications
                .on
                .iter()
                .map(|o| o.as_str().to_owned())
                .collect(),
        );
        vec![
            SettingItem::new("ui.theme", text(&self.ui.theme), ReadOnly, false),
            SettingItem::new(
                "ui.show_member_list",
                SettingValue::Bool(self.ui.show_member_list),
                Bool,
                false,
            ),
            SettingItem::new(
                "ui.images",
                text(self.ui.images.as_str()),
                texts(&["off", "thumbnail", "full"]),
                false,
            ),
            SettingItem::new(
                "ui.image_max_rows",
                SettingValue::Int(i64::from(self.ui.image_max_rows)),
                Int {
                    min: 1,
                    max: 100,
                    step: 1,
                },
                false,
            ),
            SettingItem::new(
                "behavior.send_typing",
                SettingValue::Bool(self.behavior.send_typing),
                Bool,
                true,
            ),
            SettingItem::new(
                "behavior.send_read_receipts",
                SettingValue::Bool(self.behavior.send_read_receipts),
                Bool,
                true,
            ),
            SettingItem::new(
                "notifications.enabled",
                SettingValue::Bool(self.notifications.enabled),
                Bool,
                true,
            ),
            SettingItem::new(
                "notifications.on",
                on,
                Choice(vec![
                    list(&["mention"]),
                    list(&["dm"]),
                    list(&["mention", "dm"]),
                    list(&["all"]),
                ]),
                true,
            ),
            SettingItem::new(
                "voice.mode",
                text(match self.voice.mode {
                    VoiceMode::Vad => "vad",
                    VoiceMode::Ptt => "ptt",
                }),
                texts(&["vad", "ptt"]),
                false,
            ),
            SettingItem::new(
                "voice.jitter_ms",
                SettingValue::Int(i64::from(self.voice.jitter_ms)),
                Int {
                    min: 0,
                    max: 1000,
                    step: 20,
                },
                false,
            ),
        ]
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
        if section == "keys" {
            // Eylem adları burada değil, kısayol tablosu kurulurken doğrulanır.
            continue;
        }
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

/// Değişen ayarları `path` altındaki config.toml'a yazar; kullanıcının yorumlarını ve biçimini
/// korur (`toml_edit`). Dosya yoksa oluşturulur. Disk işi: UI thread'inde çağrılmaz.
pub fn save_settings(path: &Path, changes: &[(String, SettingValue)]) -> anyhow::Result<()> {
    use toml_edit::{DocumentMut, Item, Table, Value};

    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err).with_context(|| format!("{} okunamadı", path.display())),
    };
    let mut doc: DocumentMut = text
        .parse()
        .with_context(|| format!("{} geçersiz", path.display()))?;
    for (key, value) in changes {
        let (section, name) = key
            .split_once('.')
            .with_context(|| format!("geçersiz ayar anahtarı: {key}"))?;
        let mut new: Value = match value {
            SettingValue::Bool(b) => Value::from(*b),
            SettingValue::Int(n) => Value::from(*n),
            SettingValue::Str(s) => Value::from(s.as_str()),
            SettingValue::List(l) => l.iter().map(String::as_str).collect(),
        };
        if !doc.contains_key(section) {
            doc.insert(section, Item::Table(Table::new()));
        }
        let table = doc[section]
            .as_table_mut()
            .with_context(|| format!("{section} bir tablo olmalı"))?;
        // Satır sonu yorumu gibi süslemeler korunsun.
        if let Some(old) = table.get(name).and_then(Item::as_value) {
            *new.decor_mut() = old.decor().clone();
        }
        table[name] = Item::Value(new);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("{} oluşturulamadı", dir.display()))?;
    }
    // Yarım yazılmış config kalmasın: önce geçici dosyaya, sonra yer değiştir.
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, doc.to_string())
        .with_context(|| format!("{} yazılamadı", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("{} güncellenemedi", path.display()))?;
    Ok(())
}

/// `notifications.on` listesinden bildirim süzgeci (bilinmeyen adlar yok sayılır).
pub fn notify_filter(names: &[String]) -> digcord_store::NotifyFilter {
    digcord_store::NotifyFilter {
        mention: names.iter().any(|n| n == "mention"),
        dm: names.iter().any(|n| n == "dm"),
        all: names.iter().any(|n| n == "all"),
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
theme = "light"
show_member_list = false
images = "full"
image_max_rows = 20

[behavior]
send_typing = false
send_read_receipts = false

[notifications]
enabled = false
on = ["all"]

[voice]
mode = "ptt"
jitter_ms = 100

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
                    theme: "light".into(),
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
                voice: Voice {
                    mode: VoiceMode::Ptt,
                    jitter_ms: 100,
                },
                client: Client {
                    build_number: 12345,
                    chrome_major: 130,
                },
                keys: BTreeMap::new(),
            }
        );
    }

    #[test]
    fn bad_image_mode_is_an_error() {
        assert!(Config::parse("[ui]\nimages = \"big\"\n").is_err());
        let opts = Config::parse("[ui]\nimages = \"off\"\nimage_max_rows = 0\n")
            .expect("geçerli")
            .tui_options(digcord_tui::Theme::default())
            .0;
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
        let text = "[ui]\nkeymap = \"x\"\n\n[voice]\ninput_device = \"default\"\n";
        let (config, warnings) = parse_with_warnings(text).expect("bilinmeyen anahtar hata değil");
        assert_eq!(config, Config::default());
        assert_eq!(
            warnings,
            vec![
                "bilinmeyen ayar: ui.keymap".to_string(),
                "bilinmeyen ayar: voice.input_device".to_string(),
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
        assert_eq!(config.voice.mode, VoiceMode::Vad);
        assert_eq!(config.ui.images, Images::Thumbnail);
        assert_eq!(config.ui.image_max_rows, 10);
        for expected in [
            "bilinmeyen ayar: ui.keymap",
            "bilinmeyen ayar: ui.timestamp_format",
            "bilinmeyen ayar: behavior.message_window",
            "bilinmeyen ayar: voice.input_device",
            "bilinmeyen ayar: voice.output_device",
            "bilinmeyen ayar: voice.ptt_key",
        ] {
            assert!(
                warnings.iter().any(|w| w == expected),
                "{expected} uyarısı yok: {warnings:?}"
            );
        }
    }

    #[test]
    fn keys_table_is_not_unknown_and_builds_overrides() {
        let text = "[keys]\nquit = \"x\"\ncopy = [\"y\", \"ctrl+c\"]\nreply = 5\n";
        let (config, warnings) = parse_with_warnings(text).expect("geçerli");
        assert!(warnings.is_empty(), "{warnings:?}");
        let (overrides, bad) = config.key_overrides();
        assert_eq!(
            overrides,
            vec![
                ("copy".to_owned(), vec!["y".to_owned(), "ctrl+c".to_owned()]),
                ("quit".to_owned(), vec!["x".to_owned()]),
            ]
        );
        assert_eq!(bad.len(), 1);
        assert!(bad[0].contains("keys.reply"));
        // Bilinmeyen eylem ve bozuk tuş, açılışı durdurmayan uyarıdır.
        let (config, _) =
            parse_with_warnings("[keys]\nfly = \"f\"\nquit = \"ctrl+\"\n").expect("geçerli");
        let (opts, warnings) = config.tui_options(digcord_tui::Theme::default());
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert_eq!(opts.keys, digcord_tui::Keymap::default());
    }

    #[test]
    fn setting_items_match_known_keys_and_defaults() {
        let items = Config::default().setting_items();
        for item in &items {
            assert!(
                KNOWN_KEYS.contains(&item.key.as_str()),
                "{} KNOWN_KEYS'te yok",
                item.key
            );
        }
        // Ayarlar ekranı `client.*` dışındaki tüm bilinen anahtarları kapsar.
        for key in KNOWN_KEYS.iter().filter(|k| !k.starts_with("client.")) {
            assert!(
                items.iter().any(|i| i.key == *key),
                "{key} ayarlar ekranında yok"
            );
        }
        let on = items
            .iter()
            .find(|i| i.key == "notifications.on")
            .expect("satır");
        assert_eq!(
            on.value,
            SettingValue::List(vec!["mention".into(), "dm".into()])
        );
        // Mevcut değer her zaman seçenekler arasında (döngü ondan başlar).
        for item in &items {
            if let SettingKind::Choice(opts) = &item.kind {
                assert!(
                    opts.contains(&item.value),
                    "{} değeri seçeneklerde yok",
                    item.key
                );
            }
        }
    }

    #[test]
    fn save_settings_preserves_comments_and_adds_missing_keys() {
        let dir = std::env::temp_dir().join(format!("digcord-save-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("geçici dizin");
        let path = dir.join("config.toml");
        let original = "# Benim ayarlarım\n[ui]\nimages = \"thumbnail\"   # \"off\" | \"thumbnail\"\nimage_max_rows = 10\n\n[keys]\nquit = \"x\"\n\n[behavior]\n# yazıyor göstergesi\nsend_typing = true\n";
        std::fs::write(&path, original).expect("yazılır");
        save_settings(
            &path,
            &[
                ("ui.images".into(), SettingValue::Str("full".into())),
                ("behavior.send_typing".into(), SettingValue::Bool(false)),
                ("notifications.enabled".into(), SettingValue::Bool(false)),
                (
                    "notifications.on".into(),
                    SettingValue::List(vec!["all".into()]),
                ),
                ("voice.jitter_ms".into(), SettingValue::Int(100)),
            ],
        )
        .expect("kaydedilir");
        let saved = std::fs::read_to_string(&path).expect("okunur");
        // Yorumlar, satır sonu yorumu, [keys] ve sıralama korunur.
        assert!(
            saved.starts_with(
                "# Benim ayarlarım\n[ui]\nimages = \"full\"   # \"off\" | \"thumbnail\"\n"
            ),
            "{saved}"
        );
        assert!(saved.contains("image_max_rows = 10\n"));
        assert!(saved.contains("[keys]\nquit = \"x\""));
        assert!(
            saved.contains("# yazıyor göstergesi\nsend_typing = false"),
            "{saved}"
        );
        // Olmayan bölüm/anahtarlar eklenir ve dosya yine geçerli yapılandırmadır.
        let (config, warnings) = parse_with_warnings(&saved).expect("geçerli");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(config.ui.images, Images::Full);
        assert!(!config.behavior.send_typing);
        assert!(!config.notifications.enabled);
        assert_eq!(config.notifications.on, vec![NotifyOn::All]);
        assert_eq!(config.voice.jitter_ms, 100);
        assert!(!dir.join("config.toml.tmp").exists());

        // Dosya yoksa oluşturulur; bozuk dosya hata verir ve dokunulmaz.
        let fresh = dir.join("yeni").join("config.toml");
        save_settings(
            &fresh,
            &[("ui.theme".into(), SettingValue::Str("light".into()))],
        )
        .expect("oluşturulur");
        assert_eq!(Config::load(&fresh).expect("geçerli").0.ui.theme, "light");
        let broken = dir.join("bozuk.toml");
        std::fs::write(&broken, "[ui\n").expect("yazılır");
        let err = save_settings(
            &broken,
            &[("ui.images".into(), SettingValue::Str("off".into()))],
        )
        .expect_err("bozuk dosya");
        assert!(format!("{err:#}").contains("bozuk.toml"));
        assert_eq!(std::fs::read_to_string(&broken).expect("okunur"), "[ui\n");
        let _ = std::fs::remove_dir_all(&dir);
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
