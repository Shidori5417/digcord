//! `:diag` tam ekran teşhis özeti: denetleyicinin hazırladığı satırlar (özet sayaçlar + son
//! olaylar) kaydırılabilir gösterilir. Disk okuması denetleyicide yapılır; burada ağ/disk yok.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::Modifier;
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Paragraph};

use crate::compose::Mode;
use crate::{Action, App};

/// Ekranda gösterilecek son olay sayısı (`Action::OpenDiag`).
pub(crate) const RECENT_EVENTS: usize = 200;

impl App {
    /// Ekranı açar; satırlar denetleyiciden gelene kadar "yükleniyor" görünür.
    pub(crate) fn open_diag(&mut self, from: Mode) -> Action {
        self.diag_from = from;
        self.diag_lines = None;
        self.diag_scroll = usize::MAX;
        self.mode = Mode::Diag;
        Action::OpenDiag {
            recent: RECENT_EVENTS,
        }
    }

    /// Denetleyiciden gelen satırlar (kapatılmışsa yok sayılır). En yeni olaylar altta olduğundan
    /// liste sona kaydırılmış açılır.
    pub(crate) fn diag_loaded(&mut self, lines: Vec<String>) {
        if self.mode == Mode::Diag {
            self.diag_lines = Some(lines);
            self.diag_scroll = usize::MAX;
            self.diag_scroll = self.diag_scroll.min(self.diag_max_scroll());
        }
    }

    pub(crate) fn diag_key(&mut self, key: KeyEvent) -> Option<Action> {
        let page = usize::from(self.diag_height.get().max(2) - 1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.mode = self.diag_from,
            KeyCode::Char('j') | KeyCode::Down => {
                self.diag_scroll = self.diag_scroll.saturating_add(1)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.diag_scroll = self.diag_scroll.saturating_sub(1)
            }
            KeyCode::PageDown | KeyCode::Char(' ') => {
                self.diag_scroll = self.diag_scroll.saturating_add(page);
            }
            KeyCode::PageUp => self.diag_scroll = self.diag_scroll.saturating_sub(page),
            KeyCode::Home | KeyCode::Char('g') => self.diag_scroll = 0,
            KeyCode::End | KeyCode::Char('G') => self.diag_scroll = usize::MAX,
            KeyCode::Char('r') => return Some(self.open_diag(self.diag_from)),
            _ => {}
        }
        self.diag_scroll = self.diag_scroll.min(self.diag_max_scroll());
        None
    }

    pub(crate) fn diag_max_scroll(&self) -> usize {
        self.diag_lines
            .as_ref()
            .map_or(0, Vec::len)
            .saturating_sub(usize::from(self.diag_height.get()))
    }
}

pub(crate) fn draw(app: &App, frame: &mut Frame) {
    let t = &app.theme;
    let area = frame.area();
    frame.render_widget(Clear, area);
    frame.buffer_mut().set_style(area, t.base());
    let block = Block::bordered()
        .title(" Teşhis (diag.log) ")
        .border_style(t.border_style(true));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [body, footer] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(inner);
    app.diag_height.set(body.height);
    let all: Vec<Line> = match &app.diag_lines {
        None => vec![Line::styled("Yükleniyor…", t.dim())],
        Some(lines) => lines
            .iter()
            .map(|l| match l.strip_prefix("# ") {
                Some(h) => {
                    Line::styled(h.to_owned(), t.author().add_modifier(Modifier::UNDERLINED))
                }
                None => Line::raw(l.clone()),
            })
            .collect(),
    };
    let max = all.len().saturating_sub(usize::from(body.height));
    let scroll = app.diag_scroll.min(max);
    let visible: Vec<Line> = all
        .into_iter()
        .skip(scroll)
        .take(usize::from(body.height))
        .collect();
    frame.render_widget(Paragraph::new(visible), body);
    frame.render_widget(
        Paragraph::new(Line::styled(
            "j/k kaydır · PgUp/PgDn sayfa · g/G baş/son · r yenile · Esc/q kapat",
            t.dim(),
        )),
        footer,
    );
}
