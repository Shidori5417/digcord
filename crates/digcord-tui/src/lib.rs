//! ratatui arayüzü, girdi ve kısayollar.

use std::io;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::widgets::{Block, Paragraph};
use ratatui::{DefaultTerminal, Frame};

/// Terminali hazırlar, arayüzü çalıştırır ve çıkışta terminali geri yükler.
///
/// `ratatui::run` panic olduğunda da terminali geri yükleyen bir hook kurar; bu yüzden
/// çağıran kendi panic hook'larını bundan **önce** kurmalı.
pub fn run() -> io::Result<()> {
    ratatui::run(|terminal| App::default().run(terminal))
}

/// Uygulama durumu.
#[derive(Debug, Default)]
pub struct App {
    should_quit: bool,
}

impl App {
    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| self.draw(frame))?;
            if let Event::Key(key) = event::read()? {
                self.handle_key(key);
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), KeyModifiers::NONE)
            | (KeyCode::Char('c'), KeyModifiers::CONTROL) => self.should_quit = true,
            _ => {}
        }
    }

    /// Boş üç panelli düzen: kanallar | mesajlar | üyeler + durum çubuğu.
    pub fn draw(&self, frame: &mut Frame) {
        let [main, status] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
        let [channels, messages, members] = Layout::horizontal([
            Constraint::Length(22),
            Constraint::Fill(1),
            Constraint::Length(20),
        ])
        .areas(main);

        frame.render_widget(Block::bordered().title(" Kanallar "), channels);
        frame.render_widget(Block::bordered().title(" Mesajlar "), messages);
        frame.render_widget(Block::bordered().title(" Üyeler "), members);
        frame.render_widget(
            Paragraph::new(" [NORMAL]  Digcord · bağlı değil · q: çıkış"),
            status,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn empty_layout_snapshot() {
        let mut terminal = Terminal::new(TestBackend::new(80, 12)).expect("test terminali");
        let app = App::default();
        terminal.draw(|f| app.draw(f)).expect("çizim");
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn q_and_ctrl_c_quit() {
        for key in [
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            let mut app = App::default();
            app.handle_key(key);
            assert!(app.should_quit, "{key:?}");
        }
    }

    #[test]
    fn other_keys_do_not_quit() {
        let mut app = App::default();
        app.handle_key(KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::SHIFT));
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        assert!(!app.should_quit);
    }
}
