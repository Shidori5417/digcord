//! Kod bloğu sözdizimi renklendirme (`syntect`, saf Rust `regex-fancy`).
//!
//! Sözdizimi/tema dökümlerinin açılması onlarca ms sürer; bu yüzden arka plan iş parçacığında
//! ([`warm_up`]) tembelce yüklenir. Yüklenene kadar [`highlight`] `None` döner ve kod düz
//! stille çizilir. Renklendirme satır yüksekliğini değiştirmez (sarma karakter başına aynı),
//! dolayısıyla kaydırma hesapları etkilenmez.
//!
//! Sonuçlar (dil, tema, kod) özetine göre önbelleğe alınır: sıcak yol kare başına yalnızca
//! özet hesaplar.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use ratatui::style::{Color, Modifier, Style};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

/// Yerleşik renk teması (syntect'in varsayılan kümesinden).
pub const DEFAULT_THEME: &str = "base16-ocean.dark";
/// `ui.code_theme` için seçilebilen adlar (syntect varsayılan kümesi).
pub const THEMES: &[&str] = &[
    "base16-ocean.dark",
    "base16-eighties.dark",
    "base16-mocha.dark",
    "base16-ocean.light",
    "InspiredGitHub",
    "Solarized (dark)",
    "Solarized (light)",
];

/// Bundan büyük bloklar renklendirilmez (UI iş parçacığını korur).
const MAX_BYTES: usize = 8 * 1024;
const MAX_LINES: usize = 100;
/// Önbellekte tutulan en fazla blok; dolunca temizlenir.
const CACHE_CAP: usize = 128;

/// Bir kaynak satırın renkli parçaları.
pub(crate) type Runs = Vec<(String, Style)>;

struct Engine {
    syntaxes: SyntaxSet,
    themes: ThemeSet,
}

static ENGINE: OnceLock<Engine> = OnceLock::new();
type Cache = Mutex<HashMap<u64, Arc<Vec<Runs>>>>;
static CACHE: OnceLock<Cache> = OnceLock::new();

fn load() -> &'static Engine {
    ENGINE.get_or_init(|| Engine {
        syntaxes: SyntaxSet::load_defaults_newlines(),
        themes: ThemeSet::load_defaults(),
    })
}

/// Dökümleri arka planda yükler; UI iş parçacığı beklemez.
pub fn warm_up() {
    if ENGINE.get().is_none() {
        let _ = std::thread::Builder::new()
            .name("digcord-syntax".into())
            .spawn(|| {
                load();
                prime();
            });
    }
}

/// Yaygın dillerin düzenli ifadeleri ilk kullanımda derlenir (onlarca ms); bunu UI
/// iş parçacığı yerine burada ödetir.
fn prime() {
    for lang in ["rust", "py", "js", "json", "sh", "c", "go", "java"] {
        let _ = highlight(lang, DEFAULT_THEME, "x = 1 // \"a\"\n");
    }
}

/// Testler için: dökümleri çağıran iş parçacığında yükler.
#[cfg(test)]
pub(crate) fn load_blocking() {
    load();
}

/// Sık kullanılan çit adlarını sözdizimi adlarına eşler (pakette olmayanlar en yakına).
fn alias(lang: &str) -> &str {
    match lang {
        "ts" | "typescript" | "tsx" | "jsx" | "mjs" | "cjs" => "js",
        "shell" | "zsh" | "fish" | "console" | "shellscript" => "sh",
        "c++" => "cpp",
        "golang" => "go",
        "yml" => "yaml",
        "rs" => "rust",
        other => other,
    }
}

fn find<'a>(engine: &'a Engine, lang: &str) -> Option<&'a SyntaxReference> {
    let lower = lang.trim().to_lowercase();
    engine.syntaxes.find_syntax_by_token(alias(&lower))
}

fn key(lang: &str, theme: &str, code: &str) -> u64 {
    let mut h = DefaultHasher::new();
    lang.hash(&mut h);
    theme.hash(&mut h);
    code.hash(&mut h);
    h.finish()
}

/// `code`'u `lang` diliyle renklendirir: kaynak satır başına renkli parçalar.
/// Dökümler yüklenmediyse, dil bilinmiyorsa ya da kod çok büyükse `None` (düz çizilir).
pub(crate) fn highlight(lang: &str, theme: &str, code: &str) -> Option<Arc<Vec<Runs>>> {
    if code.len() > MAX_BYTES || code.lines().count() > MAX_LINES {
        return None;
    }
    let engine = ENGINE.get()?;
    let k = key(lang, theme, code);
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(&k).cloned()) {
        return Some(hit);
    }
    let syntax = find(engine, lang)?;
    let theme = engine
        .themes
        .themes
        .get(theme)
        .or_else(|| engine.themes.themes.get(DEFAULT_THEME))?;
    let mut hl = HighlightLines::new(syntax, theme);
    let mut lines = Vec::new();
    for line in LinesWithEndings::from(code) {
        let ranges = hl.highlight_line(line, &engine.syntaxes).ok()?;
        let mut runs: Runs = Vec::new();
        for (st, text) in ranges {
            let text = text.trim_end_matches(['\n', '\r']);
            if text.is_empty() {
                continue;
            }
            let style = to_style(st);
            match runs.last_mut() {
                Some((t, s)) if *s == style => t.push_str(text),
                _ => runs.push((text.to_owned(), style)),
            }
        }
        lines.push(runs);
    }
    let out = Arc::new(lines);
    if let Ok(mut c) = cache.lock() {
        if c.len() >= CACHE_CAP {
            c.clear();
        }
        c.insert(k, Arc::clone(&out));
    }
    Some(out)
}

fn to_style(st: syntect::highlighting::Style) -> Style {
    let c = st.foreground;
    let mut s = Style::new().fg(Color::Rgb(c.r, c.g, c.b));
    if st.font_style.contains(FontStyle::BOLD) {
        s = s.add_modifier(Modifier::BOLD);
    }
    if st.font_style.contains(FontStyle::ITALIC) {
        s = s.add_modifier(Modifier::ITALIC);
    }
    if st.font_style.contains(FontStyle::UNDERLINE) {
        s = s.add_modifier(Modifier::UNDERLINED);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_snippet_gets_several_colors() {
        load_blocking();
        let out = highlight(
            "rust",
            DEFAULT_THEME,
            "fn main() {\n    let x = 1; // sayı\n}",
        )
        .expect("renklendirildi");
        assert_eq!(out.len(), 3);
        let colors: std::collections::HashSet<_> =
            out.iter().flatten().filter_map(|(_, s)| s.fg).collect();
        assert!(colors.len() >= 3, "{colors:?}");
        // Metin kaybolmaz.
        let joined: String = out[1].iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(joined, "    let x = 1; // sayı");
    }

    #[test]
    fn aliases_and_unknown_languages() {
        load_blocking();
        for lang in ["py", "python", "js", "ts", "json", "bash", "sh", "RS"] {
            assert!(highlight(lang, DEFAULT_THEME, "x").is_some(), "{lang}");
        }
        assert!(highlight("nosuchlang", DEFAULT_THEME, "x").is_none());
    }

    #[test]
    fn oversize_blocks_stay_plain() {
        load_blocking();
        let big = "x\n".repeat(MAX_LINES + 1);
        assert!(highlight("rust", DEFAULT_THEME, &big).is_none());
    }

    #[test]
    fn all_listed_themes_exist() {
        load_blocking();
        for name in THEMES {
            assert!(load().themes.themes.contains_key(*name), "{name}");
        }
    }

    /// Ölçüm: `cargo test -p digcord-tui --release -- --ignored --nocapture code_highlight_cost`
    #[test]
    #[ignore]
    fn code_highlight_cost() {
        use std::time::Instant;
        let t = Instant::now();
        load_blocking();
        println!("döküm yükleme: {:?}", t.elapsed());
        let t = Instant::now();
        prime();
        println!("ısındırma (arka planda): {:?}", t.elapsed());
        let code = "fn main() {\n    let v: Vec<u32> = (0..10).collect(); // yorum\n    println!(\"{v:?}\");\n}\n"
            .repeat(10);
        let t = Instant::now();
        let first = highlight("rust", DEFAULT_THEME, &code);
        println!("ilk renklendirme (40 satır): {:?}", t.elapsed());
        assert!(first.is_some());
        let t = Instant::now();
        for _ in 0..1000 {
            let _ = highlight("rust", DEFAULT_THEME, &code);
        }
        println!("önbellekten (kare başına blok): {:?}", t.elapsed() / 1000);
    }
}
