mod logging;

fn main() -> anyhow::Result<()> {
    let _log_guard = logging::init()?;
    // TUI kendi (terminali geri yükleyen) hook'unu bundan sonra kurar; sıra önemli.
    install_panic_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "digcord başlıyor");

    digcord_tui::run()?;

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
