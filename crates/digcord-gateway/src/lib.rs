//! WebSocket gateway: identify, heartbeat, resume, zlib-stream.

mod connection;
pub mod protocol;
pub mod zlib;

/// Teşhis olaylarının `tracing` hedefi (token içermeyen, alanları açıkça seçilmiş olaylar);
/// uygulama bunları `diag.log`'a yazar. Bkz. PROTOCOL_NOTES §8.
pub const DIAG: &str = "diag";

pub use connection::{Command, Config, Update, run};
pub use protocol::GatewayEvent;
