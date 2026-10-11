//! The taskstaka binary as a library so integration tests can drive the real
//! router and client in-process. `main.rs` is a one-liner over `cli::main`.

pub use taskstaka_client as client;

pub mod args;
pub mod backend;
pub mod cli;
pub mod dates;
pub mod mcp;
pub mod notes_dir;
pub mod quick;
pub mod render;
pub mod server;
pub mod service;
pub mod setup;
pub mod setup_macos;
pub mod tray;
pub mod tui;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_PORT: u16 = taskstaka_client::DEFAULT_PORT;
