//! WebSocket gateway: identify, heartbeat, resume, zlib-stream.

mod connection;
pub mod protocol;
pub mod zlib;

pub use connection::{Config, Update, run};
