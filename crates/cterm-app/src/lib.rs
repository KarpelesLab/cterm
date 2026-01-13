//! cterm-app: Application logic for cterm
//!
//! This crate contains the application logic that is independent of the UI,
//! including configuration management, session handling, and sticky tabs.

pub mod config;
pub mod session;
pub mod shortcuts;

pub use config::{Config, load_config, save_config};
pub use session::{Session, TabState, WindowState};
pub use shortcuts::ShortcutManager;
