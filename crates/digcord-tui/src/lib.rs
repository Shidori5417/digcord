//! ratatui arayüzü, girdi ve kısayollar.
//!
//! TUI ağ katmanını bilmez: dışarıdan [`UiEvent`] alır, dışarıya [`Action`] verir.

use std::io;
use std::sync::mpsc::{Receiver, Sender};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, Clear, Paragraph};
use ratatui::{DefaultTerminal, Frame};

/// TUI'ye gelen olaylar.
#[derive(Debug)]
pub enum UiEvent {
    Input(Event),
    Status(Status),
    /// Token iste; `error` bir önceki denemenin neden başarısız olduğu.
    ShowLogin {
        error: Option<String>,
    },
}

/// Bağlantı durumu (durum çubuğunda gösterilir).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Starting,
    Connecting,
    Online {
        user: String,
        guilds: usize,
        channels: usize,
    },
    Reconnecting {
        secs: u64,
        reason: String,
    },
    Failed(String),
}

/// TUI'den uygulamaya giden eylemler.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    SubmitToken(String),
    Quit,
}

/// Terminali hazırlar, `events` kanalı kapanana veya kullanıcı çıkana kadar çizer.
/// Girdi ayrı bir thread'de okunup `tx` ile aynı kanala gönderilir; döngü boşta CPU harcamaz.
///
/// `ratatui::run` panic olduğunda da terminali geri yükleyen bir hook kurar; bu yüzden
/// çağıran kendi panic hook'larını bundan **önce** kurmalı.
pub fn run(
    tx: Sender<UiEvent>,
    events: Receiver<UiEvent>,
    mut actions: impl FnMut(Action),
) -> io::Result<()> {
    std::thread::Builder::new()
        .name("digcord-input".into())
        .spawn(move || {
            while let Ok(ev) = event::read() {
                if tx.send(UiEvent::Input(ev)).is_err() {
                    break;
                }
            }
        })?;
    ratatui::run(|terminal| App::default().run(terminal, &events, &mut actions))
}

#[derive(Debug, Default)]
enum Screen {
    #[default]
    Main,
    Login {
        input: String,
        error: Option<String>,
    },
}

/// Uygulama durumu.
#[derive(Debug)]
pub struct App {
    screen: Screen,
    status: Status,
    should_quit: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            screen: Screen::Main,
            status: Status::Starting,
            should_quit: false,
        }
    }
}

impl App {
    fn run(
        &mut self,
        terminal: &mut DefaultTerminal,
        events: &Receiver<UiEvent>,
        actions: &mut impl FnMut(Action),
    ) -> io::Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| self.draw(frame))?;
            let Ok(ev) = events.recv() else { break };
            if let Some(action) = self.handle(ev) {
                actions(action);
            }
        }
        actions(Action::Quit);
        Ok(())
    }

    /// Bir olayı işler; gerekiyorsa dışarıya bir eylem döndürür.
    pub fn handle(&mut self, ev: UiEvent) -> Option<Action> {
        match ev {
            UiEvent::Input(Event::Key(key)) => self.handle_key(key),
            UiEvent::Input(Event::Paste(text)) => {
                if let Screen::Login { input, .. } = &mut self.screen {
                    input.extend(text.chars().filter(|c| !c.is_control()));
                }
                None
            }
            UiEvent::Input(_) => None,
            UiEvent::Status(status) => {
                self.status = status;
                None
            }
            UiEvent::ShowLogin { error } => {
                self.screen = Screen::Login {
                    input: String::new(),
                    error,
                };
                None
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.kind != KeyEventKind::Press {
            return None;
        }
        let ctrl_c = key.code == KeyCode::Char('c') && key.modifiers == KeyModifiers::CONTROL;
        match &mut self.screen {
            Screen::Main => {
                if ctrl_c || (key.code == KeyCode::Char('q') && key.modifiers == KeyModifiers::NONE)
                {
                    self.should_quit = true;
                }
                None
            }
            Screen::Login { input, error } => match key.code {
                _ if ctrl_c => {
                    self.should_quit = true;
                    None
                }
                KeyCode::Esc => {
                    self.should_quit = true;
                    None
                }
                KeyCode::Enter if !input.trim().is_empty() => {
                    let token = std::mem::take(input);
                    self.screen = Screen::Main;
                    self.status = Status::Connecting;
                    Some(Action::SubmitToken(token))
                }
                KeyCode::Enter => {
                    *error = Some("Token boş olamaz".into());
                    None
                }
                KeyCode::Backspace => {
                    input.pop();
                    None
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.push(c);
                    None
                }
                _ => None,
            },
        }
    }

    /// Üç panelli düzen + durum çubuğu; giriş ekranı açıksa üstünde pencere.
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
        frame.render_widget(Paragraph::new(self.status_line()), status);

        if let Screen::Login { input, error } = &self.screen {
            draw_login(frame, input, error.as_deref());
        }
    }

    fn status_line(&self) -> Line<'static> {
        let mode = if matches!(self.screen, Screen::Login { .. }) {
            " [GİRİŞ] "
        } else {
            " [NORMAL] "
        };
        let text = match &self.status {
            Status::Starting => "Digcord · başlatılıyor…".to_owned(),
            Status::Connecting => "Digcord · bağlanıyor…".to_owned(),
            Status::Online {
                user,
                guilds,
                channels,
            } => format!("Digcord · {user} · bağlı · {guilds} sunucu · {channels} kanal"),
            Status::Reconnecting { secs, reason } => {
                format!("Digcord · yeniden bağlanılıyor ({secs} sn): {reason}")
            }
            Status::Failed(reason) => format!("Digcord · hata: {reason}"),
        };
        let style = match self.status {
            Status::Online { .. } => Style::new().green(),
            Status::Reconnecting { .. } => Style::new().yellow(),
            Status::Failed(_) => Style::new().red(),
            _ => Style::new(),
        };
        Line::from(vec![mode.bold(), ratatui::text::Span::styled(text, style)])
    }
}

fn draw_login(frame: &mut Frame, input: &str, error: Option<&str>) {
    let area = centered(frame.area(), 60, 9);
    frame.render_widget(Clear, area);
    let block = Block::bordered().title(" Digcord — Giriş ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let field_width = usize::from(inner.width.saturating_sub(4));
    let count = input.chars().count();
    // Token asla gösterilmez; sadece uzunluğu kadar nokta.
    let masked: String = "•".repeat(count.min(field_width));
    let lines = vec![
        Line::from("Discord token'ını yapıştır (yan hesap önerilir):"),
        Line::from(""),
        Line::from(format!("> {masked}")).bold(),
        Line::from(""),
        match error {
            Some(e) => Line::from(e.to_owned()).red(),
            None => Line::from(""),
        },
        Line::from("Enter: giriş · Esc: çıkış").dim(),
    ];
    frame.render_widget(
        Paragraph::new(lines),
        inner.inner(ratatui::layout::Margin::new(1, 0)),
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [v] = Layout::vertical([Constraint::Length(height)])
        .flex(Flex::Center)
        .areas(area);
    let [h] = Layout::horizontal([Constraint::Length(width)])
        .flex(Flex::Center)
        .areas(v);
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode) -> UiEvent {
        UiEvent::Input(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    fn snapshot(app: &App) -> TestBackend {
        let mut terminal = Terminal::new(TestBackend::new(80, 14)).expect("test terminali");
        terminal.draw(|f| app.draw(f)).expect("çizim");
        terminal.backend().clone()
    }

    #[test]
    fn empty_layout_snapshot() {
        insta::assert_snapshot!(snapshot(&App::default()));
    }

    #[test]
    fn online_status_snapshot() {
        let mut app = App::default();
        app.handle(UiEvent::Status(Status::Online {
            user: "kullanici".into(),
            guilds: 9,
            channels: 120,
        }));
        insta::assert_snapshot!(snapshot(&app));
    }

    #[test]
    fn login_masks_token_snapshot() {
        let mut app = App::default();
        app.handle(UiEvent::ShowLogin {
            error: Some("Token geçersiz, tekrar dene".into()),
        });
        for c in "gizli".chars() {
            app.handle(key(KeyCode::Char(c)));
        }
        let screen = snapshot(&app);
        assert!(!format!("{screen}").contains("gizli"));
        insta::assert_snapshot!(screen);
    }

    #[test]
    fn login_submit_and_edit() {
        let mut app = App::default();
        app.handle(UiEvent::ShowLogin { error: None });
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            None,
            "boş token gönderilmez"
        );
        for c in "abcx".chars() {
            app.handle(key(KeyCode::Char(c)));
        }
        app.handle(key(KeyCode::Backspace));
        app.handle(UiEvent::Input(Event::Paste("d\n".into())));
        // Giriş ekranında `q` yazılır, çıkış yapmaz.
        app.handle(key(KeyCode::Char('q')));
        assert!(!app.should_quit);
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            Some(Action::SubmitToken("abcdq".into()))
        );
        assert!(matches!(app.screen, Screen::Main));
        assert_eq!(app.status, Status::Connecting);
    }

    #[test]
    fn login_escape_quits() {
        let mut app = App::default();
        app.handle(UiEvent::ShowLogin { error: None });
        app.handle(key(KeyCode::Esc));
        assert!(app.should_quit);
    }

    #[test]
    fn q_and_ctrl_c_quit_main() {
        for ev in [
            key(KeyCode::Char('q')),
            UiEvent::Input(Event::Key(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL,
            ))),
        ] {
            let mut app = App::default();
            app.handle(ev);
            assert!(app.should_quit);
        }
    }

    #[test]
    fn other_keys_do_not_quit() {
        let mut app = App::default();
        app.handle(UiEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Char('Q'),
            KeyModifiers::SHIFT,
        ))));
        app.handle(UiEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::NONE,
        ))));
        assert!(!app.should_quit);
    }
}
