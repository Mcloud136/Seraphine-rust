//! Shared application services for the Rust Seraphine app.
//!
//! The sidecar binary and the Tauri backend both build on this crate so the
//! LCU session, configuration and data services have a single implementation.

pub mod aram;
pub mod career;
pub mod catalog;
pub mod config;
pub mod error;
pub mod gameinfo;
pub mod live;
pub mod opgg;
pub mod paths;
pub mod session;
pub mod tft;
pub mod writes;

pub use config::{AppConfig, ConfigStore, DEFAULT_WORKSPACE};
pub use error::CoreError;
pub use paths::AppPaths;
pub use session::{SessionEvent, SessionManager, SessionStatus};
