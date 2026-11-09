//! WebSocket gateway: identify, heartbeat, resume, zlib-stream.

mod connection;
pub mod protocol;
pub mod zlib;

pub use connection::{Command, Config, Update, run};
pub use protocol::GatewayEvent;
