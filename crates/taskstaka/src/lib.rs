//! The taskstaka binary as a library so integration tests can drive the real
//! router and client in-process. `main.rs` is a one-liner over `cli::main`.

pub mod args;
pub mod backend;
pub mod cli;
pub mod client;
pub mod dates;
pub mod mcp;
pub mod render;
pub mod server;
pub mod setup;
pub mod tui;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_PORT: u16 = 7407;
