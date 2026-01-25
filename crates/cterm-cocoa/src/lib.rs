//! cterm-cocoa: Native macOS UI for cterm
//!
//! This crate implements the cterm terminal emulator UI using native
//! macOS AppKit and CoreGraphics rendering.

// Allow unnecessary unsafe blocks - objc2 API changes frequently
#![allow(unused_unsafe)]
// Allow deprecated items - needed for macOS version compatibility
#![allow(deprecated)]
// Allow dead code - some functions are for future use or debugging
#![allow(dead_code)]
// Allow unused imports - conditional compilation may need them
#![allow(unused_imports)]
// Allow unused variables - some are for future use
#![allow(unused_variables)]
// Allow unused doc comments - some are for documentation purposes
#![allow(unused_doc_comments)]
// Allow complex types - callback types need these
#![allow(clippy::type_complexity)]
// Allow let and return style - sometimes clearer
#![allow(clippy::let_and_return)]
// Allow explicit deref - sometimes clearer/necessary with objc2
#![allow(clippy::explicit_deref_methods)]
#![allow(clippy::borrow_deref_ref)]

pub mod app;
pub mod cg_renderer;
pub mod clipboard;
pub mod dialogs;
pub mod file_transfer;
pub mod log_capture;
pub mod log_viewer;
pub mod menu;
pub mod notification_bar;
pub mod preferences;
pub mod tab_bar;
pub mod tab_templates;
pub mod terminal_view;
pub mod update_dialog;
#[cfg(unix)]
pub mod upgrade_receiver;
pub mod window;

mod keycode;
mod mouse;

pub use app::run;
pub use file_transfer::PendingFileManager;
pub use notification_bar::NotificationBar;
