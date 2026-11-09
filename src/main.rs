mod controller;
mod keystore;
mod logging;

use std::time::Duration;

fn main() -> anyhow::Result<()> {
    let _log_guard = logging::init()?;
    // TUI kendi (terminali geri yükleyen) hook'unu bundan sonra kurar; sıra önemli.
    install_panic_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "digcord başlıyor");

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
        controller::Env::detect(),
    ));

    let result = digcord_tui::run(ui_tx, ui_rx, shared, |action| {
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
