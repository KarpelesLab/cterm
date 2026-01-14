//! cterm-cocoa: Native macOS UI for cterm
//!
//! This crate implements the cterm terminal emulator UI using native
//! macOS AppKit and Metal rendering.

pub mod app;
pub mod cg_renderer;
pub mod clipboard;
pub mod dialogs;
pub mod menu;
pub mod metal_renderer;
pub mod tab_bar;
pub mod terminal_view;
pub mod window;

mod keycode;

pub use app::run;
