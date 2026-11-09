mod config;
mod controller;
mod keystore;
mod logging;

use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let _log_guard = logging::init()?;
    // TUI kendi (terminali geri yükleyen) hook'unu bundan sonra kurar; sıra önemli.
    install_panic_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "digcord başlıyor");

    // Ayarlar: dosya yoksa varsayılanlar; bozuksa açılmadan hata ver (sessizce yok saymak yerine).
    let (config, warnings) = match config::Config::default_path() {
        Some(path) => config::Config::load(&path)?,
        None => (config::Config::default(), Vec::new()),
    };
    for w in &warnings {
        tracing::warn!("{w}");
    }

    // TLS: süreç genelinde tek kripto sağlayıcı (ring); gateway ve HTTP aynısını kullanır.
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Ağ ve anahtarlık tokio'da; TUI ana thread'de.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("digcord-rt")
        .enable_all()
        .build()?;
    let (ui_tx, ui_rx) = std::sync::mpsc::channel();
    let (action_tx, action_rx) = tokio::sync::mpsc::unbounded_channel();
    let shared = std::sync::Arc::new(digcord_tui::Shared::default());
    let controller = runtime.spawn(controller::run(
        action_rx,
        ui_tx.clone(),
        shared.clone(),
        controller::Env::detect(controller::Settings::from_config(&config)),
    ));

    if let Some(w) = warnings.first() {
        let more = warnings.len() - 1;
        let text = if more > 0 {
            format!("config: {w} (+{more})")
        } else {
            format!("config: {w}")
        };
        let _ = ui_tx.send(digcord_tui::UiEvent::Notice(text));
    }
    let result = digcord_tui::run(ui_tx, ui_rx, shared, config.tui_options(), |action| {
        let _ = action_tx.send(action);
    });

    // Terminal geri yüklendi; arka planı en fazla birkaç saniye bekle.
    drop(action_tx);
    runtime.block_on(async {
        let _ = tokio::time::timeout(Duration::from_secs(3), controller).await;
    });
    runtime.shutdown_timeout(Duration::from_secs(1));

    result?;
    tracing::info!("digcord kapandı");
    Ok(())
}

/// Panic mesajını log dosyasına da yazar, sonra önceki hook'u çağırır.
fn install_panic_logging() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        previous(info);
    }));
}
