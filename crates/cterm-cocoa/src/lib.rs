//! cterm-cocoa: Native macOS UI for cterm
//!
//! This crate implements the cterm terminal emulator UI using native
//! macOS AppKit and CoreGraphics rendering.

pub mod app;
pub mod cg_renderer;
pub mod clipboard;
pub mod dialogs;
pub mod file_transfer;
pub mod menu;
pub mod notification_bar;
pub mod preferences;
pub mod tab_bar;
pub mod tab_templates;
pub mod terminal_view;
#[cfg(unix)]
pub mod upgrade_receiver;
pub mod window;

mod keycode;
mod mouse;

pub use app::run;
pub use file_transfer::PendingFileManager;
pub use notification_bar::NotificationBar;
