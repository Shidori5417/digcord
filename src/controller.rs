//! TUI ile ağ/anahtarlık katmanları arasındaki köprü. tokio üzerinde çalışır;
//! TUI'ye sadece [`UiEvent`] gönderir, TUI'den [`Action`] alır.

use std::sync::mpsc::Sender;
use std::time::Duration;

use digcord_gateway::{Config, Update};
use digcord_model::{ClientProperties, DEFAULT_BUILD_NUMBER, DEFAULT_CHROME_MAJOR, Ready, Token};
use digcord_tui::{Action, Status, UiEvent};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::keystore;

struct Gateway {
    stop: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl Gateway {
    fn start(token: Token, locale: &str, updates: mpsc::Sender<Update>) -> Self {
        let properties =
            ClientProperties::chrome_linux(locale, DEFAULT_CHROME_MAJOR, DEFAULT_BUILD_NUMBER);
        let (stop, stop_rx) = watch::channel(false);
        let task = tokio::spawn(digcord_gateway::run(
            Config::new(token, properties),
            updates,
            stop_rx,
        ));
        Self { stop, task }
    }

    async fn stop(self) {
        let _ = self.stop.send(true);
        let _ = tokio::time::timeout(Duration::from_secs(2), self.task).await;
    }
}

fn online(ready: &Ready) -> Status {
    Status::Online {
        user: ready
            .user
            .global_name
            .clone()
            .unwrap_or_else(|| ready.user.username.clone()),
        guilds: ready.guilds.len(),
        channels: ready.channel_count(),
    }
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(f).await?
}

/// `actions` kapanana veya `Action::Quit` gelene kadar çalışır.
pub async fn run(
    mut actions: mpsc::UnboundedReceiver<Action>,
    ui: Sender<UiEvent>,
    locale: String,
) {
    let send = |ev: UiEvent| {
        let _ = ui.send(ev);
    };
    let (updates_tx, mut updates) = mpsc::channel(32);
    let mut gateway: Option<Gateway> = None;
    // Girişten gelen token; READY gelince (geçerli olduğu kanıtlanınca) kaydedilir.
    let mut unsaved: Option<Token> = None;
    let mut last_online: Option<Status> = None;

    let stored = blocking(|| {
        keystore::init()?;
        keystore::load()
    })
    .await;
    match stored {
        Ok(Some(token)) => {
            tracing::info!("kayıtlı token bulundu");
            gateway = Some(Gateway::start(token, &locale, updates_tx.clone()));
        }
        Ok(None) => send(UiEvent::ShowLogin { error: None }),
        Err(e) => {
            tracing::warn!("{e:#}");
            send(UiEvent::ShowLogin {
                error: Some(format!("{e:#} — token kaydedilemeyecek")),
            });
        }
    }

    loop {
        tokio::select! {
            action = actions.recv() => match action {
                Some(Action::SubmitToken(raw)) => {
                    let Some(token) = Token::new(&raw) else {
                        send(UiEvent::ShowLogin { error: Some("Token boş olamaz".into()) });
                        continue;
                    };
                    if let Some(old) = gateway.take() {
                        old.stop().await;
                    }
                    unsaved = Some(token.clone());
                    gateway = Some(Gateway::start(token, &locale, updates_tx.clone()));
                }
                Some(Action::Quit) | None => break,
            },
            Some(update) = updates.recv() => match update {
                Update::Connecting => send(UiEvent::Status(Status::Connecting)),
                Update::Ready(ready) => {
                    let status = online(&ready);
                    last_online = Some(status.clone());
                    send(UiEvent::Status(status));
                    // Kayıt başarısızsa bağlantı sürer; sadece bir sonraki açılışta token tekrar sorulur.
                    if let Some(token) = unsaved.take()
                        && let Err(e) = blocking(move || keystore::save(&token)).await
                    {
                        tracing::warn!("token kaydedilemedi: {e:#}");
                    }
                }
                Update::Resumed => {
                    send(UiEvent::Status(last_online.clone().unwrap_or(Status::Connecting)));
                }
                Update::Reconnecting { after, reason } => send(UiEvent::Status(Status::Reconnecting {
                    secs: after.as_secs().max(1),
                    reason,
                })),
                Update::InvalidToken => {
                    gateway = None;
                    unsaved = None;
                    if let Err(e) = blocking(keystore::delete).await {
                        tracing::warn!("{e:#}");
                    }
                    send(UiEvent::ShowLogin { error: Some("Token geçersiz (4004), tekrar dene".into()) });
                }
                Update::Fatal(reason) => {
                    gateway = None;
                    send(UiEvent::Status(Status::Failed(reason)));
                }
            },
        }
    }

    if let Some(gw) = gateway {
        gw.stop().await;
    }
}

/// `LC_ALL` / `LC_MESSAGES` / `LANG` değerini BCP-47'ye çevirir (`tr_TR.UTF-8` → `tr-TR`).
pub fn system_locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find_map(|v| locale_from_posix(&v))
        .unwrap_or_else(|| "en-US".to_owned())
}

fn locale_from_posix(value: &str) -> Option<String> {
    let base = value.split(['.', '@']).next()?;
    let (lang, region) = base.split_once('_')?;
    let valid = |s: &str, n: usize| s.len() == n && s.chars().all(|c| c.is_ascii_alphabetic());
    (valid(lang, 2) && valid(region, 2)).then(|| {
        format!(
            "{}-{}",
            lang.to_ascii_lowercase(),
            region.to_ascii_uppercase()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_locale_to_bcp47() {
        assert_eq!(locale_from_posix("tr_TR.UTF-8").as_deref(), Some("tr-TR"));
        assert_eq!(locale_from_posix("en_US").as_deref(), Some("en-US"));
        assert_eq!(locale_from_posix("de_DE@euro").as_deref(), Some("de-DE"));
        assert_eq!(locale_from_posix("C.UTF-8"), None);
        assert_eq!(locale_from_posix("POSIX"), None);
        assert_eq!(locale_from_posix(""), None);
    }
}
