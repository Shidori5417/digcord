//! DAVE: Discord'un ses için uçtan uca şifrelemesi (MLS grup anahtarı + kare şifreleme).
//! Ayrıntılar: docs/PROTOCOL_NOTES.md §7.

pub mod cryptor;
pub mod frame;
pub mod manager;
pub mod ratchet;
pub mod session;
