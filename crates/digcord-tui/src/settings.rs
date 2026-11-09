//! `:settings` ekranı: config.toml ayarlarını listeler ve düzenler.
//!
//! Ayar şeması (anahtarlar, türler, sınırlar) ikili tarafından verilir (`Options::settings`);
//! burada sadece gezinme, düzenleme ve çizim vardır. Değişiklikler denetleyiciye `Action`
//! olarak gider; diske yazma da orada yapılır (UI thread'inde disk yok).

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::{Action, App};

/// Bir ayarın değeri (aynı zamanda diske yazılacak TOML değeri).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingValue {
    Bool(bool),
    Int(i64),
    Str(String),
    List(Vec<String>),
}

impl SettingValue {
    fn display(&self) -> String {
        match self {
            Self::Bool(true) => "açık".to_owned(),
            Self::Bool(false) => "kapalı".to_owned(),
            Self::Int(n) => n.to_string(),
            Self::Str(s) => s.clone(),
            Self::List(l) => l.join(", "),
        }
    }
}

/// Bir ayarın nasıl değiştirileceği.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingKind {
    Bool,
    Int {
        min: i64,
        max: i64,
        step: i64,
    },
    /// Verilen değerler arasında dolaşılır.
    Choice(Vec<SettingValue>),
    /// Burada değiştirilemez (ör. tema: `:theme <ad>`).
    ReadOnly,
}

/// Ayarlar ekranındaki bir satır.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingItem {
    /// `bölüm.anahtar` (config.toml yolu).
    pub key: String,
    pub value: SettingValue,
    pub kind: SettingKind,
    /// `true`: çalışırken uygulanır; `false`: yeniden başlatınca geçerli.
    pub live: bool,
}

impl SettingItem {
    pub fn new(key: &str, value: SettingValue, kind: SettingKind, live: bool) -> Self {
        Self {
            key: key.to_owned(),
            value,
            kind,
            live,
        }
    }
}

/// Ekranın durumu.
#[derive(Debug, Default)]
pub(crate) struct State {
    pub items: Vec<SettingItem>,
    pub sel: usize,
    /// `items` ile aynı sırada: kaydedilmemiş değişiklik var mı.
    dirty: Vec<bool>,
}

impl State {
    pub(crate) fn new(items: Vec<SettingItem>) -> Self {
        let dirty = vec![false; items.len()];
        Self {
            items,
            sel: 0,
            dirty,
        }
    }

    fn any_dirty(&self) -> bool {
        self.dirty.iter().any(|d| *d)
    }

    /// Dışarıdan (ör. `:theme`) değişen bir ayarı günceller ve kaydedilecek olarak işaretler.
    pub(crate) fn set_external(&mut self, key: &str, value: SettingValue) {
        if let Some(i) = self.items.iter().position(|it| it.key == key) {
            self.items[i].value = value;
            self.dirty[i] = true;
        }
    }

    /// Seçili ayarı `dir` yönünde (+1/-1) değiştirir. `Ok(değişti)`; `Err(not)`: değişmez.
    fn adjust(&mut self, dir: i64, toggle: bool) -> Result<bool, &'static str> {
        let Some(item) = self.items.get_mut(self.sel) else {
            return Ok(false);
        };
        let new = match (&item.kind, &item.value) {
            (SettingKind::Bool, SettingValue::Bool(b)) => SettingValue::Bool(!b),
            (SettingKind::Choice(opts), cur) if !opts.is_empty() => {
                let n = opts.len();
                let next = match (opts.iter().position(|o| o == cur), dir < 0) {
                    (Some(i), false) => (i + 1) % n,
                    (Some(i), true) => (i + n - 1) % n,
                    (None, true) => n - 1,
                    (None, false) => 0,
                };
                opts[next].clone()
            }
            (SettingKind::Int { .. }, _) if toggle => {
                return Err("Sayıyı +/- (ya da ←/→) ile değiştir");
            }
            (SettingKind::Int { min, max, step }, SettingValue::Int(n)) => {
                SettingValue::Int(n.saturating_add(step.saturating_mul(dir)).clamp(*min, *max))
            }
            (SettingKind::ReadOnly, _) => {
                return Err("Bu ayar burada değişmez (tema için :theme <ad>)");
            }
            _ => return Ok(false),
        };
        if new == item.value {
            return Ok(false);
        }
        item.value = new;
        self.dirty[self.sel] = true;
        Ok(true)
    }

    /// Kaydedilmemiş ayarları döner ve temiz işaretler.
    fn take_dirty(&mut self) -> Vec<(String, SettingValue)> {
        let out = self
            .items
            .iter()
            .zip(&self.dirty)
            .filter(|(_, d)| **d)
            .map(|(it, _)| (it.key.clone(), it.value.clone()))
            .collect();
        self.dirty.fill(false);
        out
    }
}

impl App {
    pub(crate) fn settings_key(&mut self, key: KeyEvent) -> Option<Action> {
        let s = &mut self.settings;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = crate::compose::Mode::Normal;
                if s.any_dirty() {
                    self.notice = Some("Kaydedilmemiş ayar değişikliği var (:settings, s)".into());
                }
                return None;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                s.sel = (s.sel + 1).min(s.items.len().saturating_sub(1));
                return None;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                s.sel = s.sel.saturating_sub(1);
                return None;
            }
            KeyCode::Char('s') => {
                let changes = s.take_dirty();
                if changes.is_empty() {
                    self.notice = Some("Kaydedilecek değişiklik yok".into());
                    return None;
                }
                return Some(Action::SaveSettings(changes));
            }
            _ => {}
        }
        let (dir, toggle) = match key.code {
            KeyCode::Enter | KeyCode::Char(' ') => (1, true),
            KeyCode::Right | KeyCode::Char('l' | '+' | '=') => (1, false),
            KeyCode::Left | KeyCode::Char('h' | '-' | '_') => (-1, false),
            _ => return None,
        };
        match s.adjust(dir, toggle) {
            Ok(false) => None,
            Ok(true) => {
                let item = s.items.get(s.sel)?;
                if item.live {
                    Some(Action::ApplySetting {
                        key: item.key.clone(),
                        value: item.value.clone(),
                    })
                } else {
                    self.notice = Some("yeniden başlatınca geçerli".into());
                    None
                }
            }
            Err(note) => {
                self.notice = Some(note.into());
                None
            }
        }
    }
}

pub(crate) fn draw(app: &App, frame: &mut Frame) {
    let t = &app.theme;
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.buffer_mut().set_style(area, t.base());
    let s = &app.settings;
    let title = if s.any_dirty() {
        " Ayarlar (kaydedilmedi) "
    } else {
        " Ayarlar "
    };
    let block = Block::bordered()
        .title(title)
        .border_style(t.border_style(true));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [list, help] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(inner);

    let h = usize::from(list.height).max(1);
    let offset = s.sel.saturating_sub(h - 1);
    let key_w = s.items.iter().map(|i| i.key.len()).max().unwrap_or(0) + 2;
    let lines: Vec<Line> = s
        .items
        .iter()
        .enumerate()
        .skip(offset)
        .take(h)
        .map(|(i, it)| {
            let cursor = if i == s.sel { ">" } else { " " };
            let mark = if s.dirty[i] { "*" } else { " " };
            let tag = if it.kind == SettingKind::ReadOnly {
                Span::styled("salt okunur", t.dim())
            } else if it.live {
                Span::styled("anında geçerli", t.success())
            } else {
                Span::styled("yeniden başlatınca geçerli", t.warning())
            };
            let row_style = if i == s.sel {
                Style::new().add_modifier(Modifier::REVERSED)
            } else {
                Style::new()
            };
            Line::from(vec![
                Span::styled(
                    format!("{cursor}{mark}{:<key_w$}{:<18}", it.key, it.value.display()),
                    row_style,
                ),
                Span::raw(" "),
                tag,
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), list);
    frame.render_widget(
        Paragraph::new(Line::styled(
            "j/k gez · Enter/Boşluk değiştir · +/- (←/→) sayı/seçenek · s kaydet · Esc kapat",
            t.dim(),
        )),
        help,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> State {
        State::new(vec![
            SettingItem::new("a.flag", SettingValue::Bool(true), SettingKind::Bool, true),
            SettingItem::new(
                "a.mode",
                SettingValue::Str("off".into()),
                SettingKind::Choice(vec![
                    SettingValue::Str("off".into()),
                    SettingValue::Str("full".into()),
                ]),
                false,
            ),
            SettingItem::new(
                "a.rows",
                SettingValue::Int(10),
                SettingKind::Int {
                    min: 1,
                    max: 12,
                    step: 1,
                },
                false,
            ),
            SettingItem::new(
                "ui.theme",
                SettingValue::Str("default".into()),
                SettingKind::ReadOnly,
                false,
            ),
        ])
    }

    #[test]
    fn adjust_bool_choice_int_and_clamp() {
        let mut s = items();
        assert_eq!(s.adjust(1, true), Ok(true));
        assert_eq!(s.items[0].value, SettingValue::Bool(false));
        s.sel = 1;
        assert_eq!(s.adjust(1, true), Ok(true));
        assert_eq!(s.items[1].value, SettingValue::Str("full".into()));
        assert_eq!(s.adjust(1, false), Ok(true)); // başa sarar
        assert_eq!(s.items[1].value, SettingValue::Str("off".into()));
        assert_eq!(s.adjust(-1, false), Ok(true)); // geri sarar
        assert_eq!(s.items[1].value, SettingValue::Str("full".into()));
        s.sel = 2;
        assert!(s.adjust(1, true).is_err(), "Enter sayıyı değiştirmez");
        assert_eq!(s.adjust(1, false), Ok(true));
        assert_eq!(s.adjust(1, false), Ok(true));
        assert_eq!(s.items[2].value, SettingValue::Int(12));
        assert_eq!(s.adjust(1, false), Ok(false), "üst sınırda değişmez");
        s.sel = 3;
        assert!(s.adjust(1, true).is_err());
    }

    #[test]
    fn take_dirty_returns_changed_items_once() {
        let mut s = items();
        assert!(s.take_dirty().is_empty());
        let _ = s.adjust(1, true);
        s.set_external("ui.theme", SettingValue::Str("light".into()));
        let d = s.take_dirty();
        assert_eq!(
            d,
            vec![
                ("a.flag".to_owned(), SettingValue::Bool(false)),
                ("ui.theme".to_owned(), SettingValue::Str("light".into())),
            ]
        );
        assert!(s.take_dirty().is_empty());
    }
}
