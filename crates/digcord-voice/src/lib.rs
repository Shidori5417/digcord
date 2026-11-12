//! Ses gateway'i, UDP, RTP, Opus ve DAVE (MLS) şifreleme.

pub mod audio;
pub mod connection;
pub mod dave;
pub mod discovery;
pub mod protocol;
pub mod rtp;
pub mod transport;

/// Teşhis olaylarının `tracing` hedefi (token içermeyen, alanları açıkça seçilmiş olaylar);
/// uygulama bunları `diag.log`'a yazar. Bkz. PROTOCOL_NOTES §8.
pub const DIAG: &str = "diag";
