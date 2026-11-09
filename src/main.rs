mod logging;

fn main() -> anyhow::Result<()> {
    let _log_guard = logging::init()?;
    install_panic_logging();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "digcord başlıyor");
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
