//! `?` / `:help` tam ekran yardım: tüm kısayollar (canlı [`Keymap`]'ten, `[keys]` geçersiz
//! kılmaları dahil) kapsamlara göre gruplanır, ardından komutlar. Ağ/disk yok.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::compose::Mode;
use crate::keymap::{Act, Keymap, Scope};
use crate::theme::Theme;
use crate::{Action, App};

/// Eylemin tek satırlık Türkçe açıklaması.
fn describe(act: Act) -> &'static str {
    match act {
        Act::ToggleMute => "mikrofonu sustur / aç",
        Act::ToggleDeaf => "kulaklığı kapat / aç",
        Act::PushToTalk => "bas-konuş (basılıyken konuş)",
        Act::ToggleMembers => "üye panelini aç / kapat",
        Act::Quit => "çık",
        Act::Switcher => "hızlı kanal geçişi",
        Act::NextFocus => "sonraki panel",
        Act::PrevFocus => "önceki panel",
        Act::Activate => "aç / seç (mesaj panelinde SEÇ moduna girer)",
        Act::InsertMode => "mesaj yaz (YAZ modu)",
        Act::SelectMode => "mesaj seç (SEÇ modu)",
        Act::CommandMode => "komut gir",
        Act::PageDown => "yarım sayfa aşağı",
        Act::PageUp => "yarım sayfa yukarı",
        Act::ScrollBottom => "en alta (güncele) dön",
        Act::NextUnread => "sonraki okunmamış kanal",
        Act::PrevUnread => "önceki okunmamış kanal",
        Act::MoveDown => "aşağı",
        Act::MoveUp => "yukarı",
        Act::Help => "bu yardım ekranı",
        Act::Back => "NORMAL moda dön",
        Act::Reply => "yanıtla",
        Act::Edit => "düzenle (kendi mesajın)",
        Act::Delete => "sil (iki kez bas)",
        Act::React => "tepki ekle / kaldır",
        Act::Copy => "mesajı kopyala",
        Act::OpenMedia => "resmi / bağlantıyı aç",
        Act::Retry => "başarısız mesajı yeniden gönder",
    }
}

/// Yeniden atanamayan tuşlar: (bölüm başlığı, [(tuşlar, açıklama)]).
const FIXED: &[(&str, &[(&str, &str)])] = &[
    ("Sabit", &[("Ctrl+C", "her yerden çık (atanamaz)")]),
    (
        "YAZ (INSERT)",
        &[
            ("Enter", "gönder"),
            ("Alt+Enter / Shift+Enter / Ctrl+J", "yeni satır"),
            ("Tab / ↑ ↓", "tamamlamayı seç (@, #, :)"),
            ("↑ (boş girişte)", "son mesajını düzenle"),
            (
                "Ctrl+W / Ctrl+U / Ctrl+K",
                "kelime / satır başına / satır sonuna kadar sil",
            ),
            ("Ctrl+A / Ctrl+E", "satır başı / sonu"),
            ("Esc", "NORMAL moda dön"),
        ],
    ),
    (
        "Resim görüntüleyici",
        &[
            ("→ l j / ← h k", "sonraki / önceki resim"),
            ("o", "harici uygulamada aç"),
            ("Esc q", "kapat"),
        ],
    ),
    (
        "Arama / sabitlenenler (:search, :pins)",
        &[
            ("j k ↓ ↑", "sonuç seç"),
            ("g G Home End", "ilk / son sonuç"),
            ("n p PgDn PgUp", "sonraki / önceki sayfa (arama)"),
            ("Enter", "mesaja atla"),
            ("Esc q", "kapat"),
        ],
    ),
    (
        "Ayarlar (:settings)",
        &[
            ("j k ↓ ↑", "ayar seç"),
            ("Enter Boşluk", "aç / kapat, seçenek değiştir"),
            ("+ - ← →", "sayı / seçenek değiştir"),
            ("s", "config.toml'a kaydet"),
            ("Esc q", "kapat"),
        ],
    ),
    (
        "Yardım (bu ekran)",
        &[("j k ↓ ↑ PgDn PgUp", "kaydır"), ("Esc q", "kapat")],
    ),
];

/// Komutlar (`:` ile): (kullanım, açıklama).
const COMMANDS: &[(&str, &str)] = &[
    (":upload <yol> [mesaj]", "dosya yükle (kısa: :u)"),
    (
        ":search <sözcük> [from: in: has:]",
        "kanalda / sunucuda ara",
    ),
    (":pins", "sabitlenmiş mesajlar"),
    (":threads", "seçili kanalın thread listesini aç / kapat"),
    (":archived", "arşivli thread'leri yükle"),
    (":members", "üye panelini aç / kapat"),
    (":settings", "ayarlar ekranı"),
    (":theme <ad>", "tema yükle (default: yerleşik)"),
    (":devices", "ses giriş / çıkış aygıtlarını listele"),
    (":leave", "ses kanalından ayrıl"),
    (":mute", "mikrofonu sustur / aç"),
    (":deaf", "kulaklığı kapat / aç"),
    (":call", "açık DM'deki aramaya katıl"),
    (
        ":vol <ad> <0-200>",
        "ses kanalındaki kişinin ses seviyesi (%)",
    ),
    (":help", "bu yardım ekranı"),
    (":q", "çık"),
];

/// Tuş sütununun genişliği: kısayol tablolarında dar, sabit tuş ve komut satırlarında geniş.
const KEY_COL: usize = 36;
const ACTION_KEY_COL: usize = 22;

fn heading(theme: &Theme, text: &str) -> Line<'static> {
    Line::styled(
        text.to_owned(),
        theme.author().add_modifier(Modifier::UNDERLINED),
    )
}

fn row(theme: &Theme, keys: &str, text: &str, name: Option<&str>) -> Line<'static> {
    let col = if name.is_some() {
        ACTION_KEY_COL
    } else {
        KEY_COL
    };
    let mut spans = vec![
        Span::styled(
            format!("  {keys:<col$}"),
            theme.highlight().add_modifier(Modifier::BOLD),
        ),
        Span::raw(text.to_owned()),
    ];
    if let Some(name) = name {
        spans.push(Span::styled(format!("  [{name}]"), theme.dim()));
    }
    Line::from(spans)
}

fn scope_section(theme: &Theme, keys: &Keymap, title: &str, scope: Scope) -> Vec<Line<'static>> {
    let mut out = vec![heading(theme, title)];
    for (act, name, bound) in keys.scope_rows(scope) {
        let shown = if bound.is_empty() {
            "(atanmamış)".to_owned()
        } else {
            bound.join("  ")
        };
        out.push(row(theme, &shown, describe(act), Some(name)));
    }
    out.push(Line::default());
    out
}

/// Yardım ekranının tüm satırları (kaydırma uygulanmadan).
pub(crate) fn lines(keys: &Keymap, theme: &Theme) -> Vec<Line<'static>> {
    let mut out = vec![
        Line::styled(
            "Kısayollar [keys] ile yeniden atanabilir; köşeli parantezde eylem adı yazar.",
            theme.dim(),
        ),
        Line::default(),
    ];
    out.extend(scope_section(
        theme,
        keys,
        "Genel (her modda)",
        Scope::Global,
    ));
    out.extend(scope_section(theme, keys, "NORMAL", Scope::Normal));
    out.extend(scope_section(theme, keys, "SEÇ (SELECT)", Scope::Select));
    for (title, rows) in FIXED {
        out.push(heading(theme, title));
        for (k, d) in *rows {
            out.push(row(theme, k, d, None));
        }
        out.push(Line::default());
    }
    out.push(heading(theme, "Komutlar (NORMAL modda : ile)"));
    for (cmd, d) in COMMANDS {
        out.push(row(theme, cmd, d, None));
    }
    out
}

impl App {
    /// Yardım ekranını açar; kapanınca `from` moduna dönülür.
    pub(crate) fn open_help(&mut self, from: Mode) {
        self.help_from = from;
        self.help_scroll = 0;
        self.mode = Mode::Help;
    }

    pub(crate) fn help_key(&mut self, key: KeyEvent) -> Option<Action> {
        let page = usize::from(self.help_height.get().max(2) - 1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.mode = self.help_from,
            KeyCode::Char('j') | KeyCode::Down => {
                self.help_scroll = self.help_scroll.saturating_add(1)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.help_scroll = self.help_scroll.saturating_sub(1)
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.help_scroll = self.help_scroll.saturating_add(page);
            }
            KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(page),
            KeyCode::Home | KeyCode::Char('g') => self.help_scroll = 0,
            KeyCode::End | KeyCode::Char('G') => self.help_scroll = usize::MAX,
            _ => {}
        }
        // Üst sınır çizimde (görünür yükseklik bilinince) uygulanır; burada sadece taşma önlenir.
        self.help_scroll = self.help_scroll.min(self.help_max_scroll());
        None
    }

    /// Kaydırmanın üst sınırı: son çizimdeki yüksekliğe göre.
    pub(crate) fn help_max_scroll(&self) -> usize {
        lines(&self.keys, &self.theme)
            .len()
            .saturating_sub(usize::from(self.help_height.get()))
    }
}

pub(crate) fn draw(app: &App, frame: &mut Frame) {
    let t = &app.theme;
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.buffer_mut().set_style(area, t.base());
    let block = Block::bordered()
        .title(" Yardım ")
        .border_style(t.border_style(true));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [body, footer] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(inner);
    app.help_height.set(body.height);
    let all = lines(&app.keys, t);
    let max = all.len().saturating_sub(usize::from(body.height));
    let scroll = app.help_scroll.min(max);
    let visible: Vec<Line> = all
        .into_iter()
        .skip(scroll)
        .take(usize::from(body.height))
        .collect();
    frame.render_widget(Paragraph::new(visible), body);
    frame.render_widget(
        Paragraph::new(Line::styled(
            "j/k kaydır · PgUp/PgDn sayfa · Esc/q kapat",
            t.dim(),
        )),
        footer,
    );
}
