//! Tema dosyaları: `~/.config/digcord/themes/<ad>.toml`.
//!
//! Dosya, tema yuvalarını (bkz. `digcord_tui::Theme`) düz anahtarlar olarak içerir:
//! `author = "#ff8800"`, `border_focused = "cyan"`. Eksik yuvalar yerleşik temadan gelir,
//! bilinmeyen anahtarlar uyarıdır, geçersiz bir renk dosya adıyla birlikte hatadır.
//! Disk okuması UI thread'inde yapılmaz (açılışta ve denetleyicide çağrılır).

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use digcord_tui::{Theme, ThemeError};

/// Yerleşik temanın adı (dosya aranmaz).
pub const DEFAULT_NAME: &str = "default";

/// Tema dizini: `dirs::config_dir()/digcord/themes`.
pub fn dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("digcord").join("themes"))
}

/// Temayı adıyla yükler (`default` yerleşiktir). Uyarılar dosya adı içerir.
pub fn load(name: &str) -> anyhow::Result<(Theme, Vec<String>)> {
    if name == DEFAULT_NAME {
        return Ok((Theme::default(), Vec::new()));
    }
    // Ad dizin gezmesine (`../`) izin vermemeli.
    if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
        bail!("geçersiz tema adı: \"{name}\"");
    }
    let dir = dir().context("yapılandırma dizini bulunamadı")?;
    load_file(&dir.join(format!("{name}.toml")))
}

/// Tema dosyasını okur.
pub fn load_file(path: &Path) -> anyhow::Result<(Theme, Vec<String>)> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("tema {} okunamadı", path.display()))?;
    let (theme, warnings) =
        parse(&text).with_context(|| format!("tema {} geçersiz", path.display()))?;
    let file = path.file_name().map_or_else(
        || path.display().to_string(),
        |f| f.to_string_lossy().into_owned(),
    );
    Ok((
        theme,
        warnings
            .into_iter()
            .map(|w| format!("{file}: {w}"))
            .collect(),
    ))
}

/// TOML metnini temaya çevirir; bilinmeyen anahtarlar uyarı olarak döner.
pub fn parse(text: &str) -> anyhow::Result<(Theme, Vec<String>)> {
    let table: toml::Table = toml::from_str(text).context("ayrıştırılamadı")?;
    let mut theme = Theme::default();
    let mut warnings = Vec::new();
    for (slot, value) in &table {
        let Some(color) = value.as_str() else {
            bail!("{slot}: renk metin olmalı (\"#rrggbb\", \"red\", \"reset\"...)");
        };
        match theme.set(slot, color) {
            Ok(()) => {}
            Err(ThemeError::UnknownSlot(_)) => {
                warnings.push(format!("bilinmeyen tema anahtarı: {slot}"));
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok((theme, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use digcord_tui::Color;

    #[test]
    fn empty_file_is_the_default_theme() {
        let (theme, warnings) = parse("").expect("boş tema geçerli");
        assert_eq!(theme, Theme::default());
        assert!(warnings.is_empty());
    }

    #[test]
    fn shipped_light_theme_is_valid_and_complete() {
        let (theme, warnings) = parse(include_str!("../themes/light.toml")).expect("geçerli");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_ne!(theme, Theme::default());
        // Her yuva dosyada tanımlı olmalı (örnek dosya yuva listesinin belgesidir).
        let table: toml::Table =
            toml::from_str(include_str!("../themes/light.toml")).expect("toml");
        for slot in Theme::SLOTS {
            assert!(table.contains_key(*slot), "light.toml'da {slot} yok");
        }
    }

    #[test]
    fn parses_hex_names_and_falls_back_for_missing_slots() {
        let (theme, warnings) = parse(
            "# yorum\nauthor = \"#ff8800\"\nborder_focused = \"darkgray\"\nlink = \"reset\"\n",
        )
        .expect("geçerli");
        assert!(warnings.is_empty());
        assert_eq!(theme.author, Color::Rgb(255, 136, 0));
        assert_eq!(theme.border_focused, Color::DarkGray);
        assert_eq!(theme.link, Color::Reset);
        // Verilmeyen yuvalar yerleşik değerde.
        assert_eq!(theme.code, Theme::default().code);
    }

    #[test]
    fn unknown_keys_warn_and_bad_values_fail() {
        let (theme, warnings) = parse("renk_yok = \"red\"\nerror = \"blue\"\n").expect("geçerli");
        assert_eq!(
            warnings,
            vec!["bilinmeyen tema anahtarı: renk_yok".to_owned()]
        );
        assert_eq!(theme.error, Color::Blue);
        // Geçersiz renk adı ve yuva adını içerir.
        let err = parse("author = \"#12\"\n").expect_err("geçersiz renk");
        assert!(format!("{err:#}").contains("author"), "{err:#}");
        assert!(parse("author = 5\n").is_err());
        assert!(parse("author = [\n").is_err());
        // Bilinmeyen yuva + bozuk değer: yuva önce kontrol edilir (uyarı, hata değil).
        assert!(parse("renk_yok = \"zzz\"\n").is_ok());
    }

    #[test]
    fn load_file_errors_name_the_file() {
        let dir = std::env::temp_dir().join(format!("digcord-theme-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("geçici dizin");
        let bad = dir.join("kotu.toml");
        std::fs::write(&bad, "author = \"mor\"\n").expect("yazılır");
        let err = load_file(&bad).expect_err("geçersiz renk");
        assert!(format!("{err:#}").contains("kotu.toml"), "{err:#}");

        let ok = dir.join("iyi.toml");
        std::fs::write(&ok, "bilinmeyen = \"red\"\nauthor = \"red\"\n").expect("yazılır");
        let (theme, warnings) = load_file(&ok).expect("geçerli");
        assert_eq!(theme.author, Color::Red);
        assert_eq!(
            warnings,
            vec!["iyi.toml: bilinmeyen tema anahtarı: bilinmeyen".to_owned()]
        );
        let missing = load_file(&dir.join("yok.toml")).expect_err("dosya yok");
        assert!(format!("{missing:#}").contains("yok.toml"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn names_cannot_escape_the_themes_dir() {
        assert!(load("default").is_ok());
        for bad in ["", "../x", "a/b", ".gizli", "a\\b"] {
            assert!(load(bad).is_err(), "{bad:?} reddedilmeli");
        }
    }
}
