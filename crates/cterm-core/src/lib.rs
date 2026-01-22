//! cterm-core: Core terminal emulation library
//!
//! This crate provides the fundamental building blocks for terminal emulation:
//! - Color and cell attribute types
//! - Screen buffer management (grid, scrollback)
//! - ANSI/VT sequence parsing
//! - Cross-platform PTY handling

pub mod cell;
pub mod color;
pub mod drcs;
#[cfg(unix)]
pub mod fd_passing;
pub mod grid;
pub mod parser;
pub mod pty;
pub mod screen;
pub mod sixel;
pub mod term;

pub use cell::{Cell, CellAttrs};
pub use color::{AnsiColor, Color, Rgb};
pub use grid::Grid;
pub use parser::Parser;
pub use pty::{Pty, PtyConfig, PtyError, PtySize};
pub use screen::{
    ClipboardOperation, ClipboardSelection, ColorQuery, Screen, SearchResult, Selection,
    SelectionMode, SelectionPoint, TerminalImage,
};
pub use sixel::{SixelDecoder, SixelImage};
pub use drcs::{DecdldDecoder, DrcsFont, DrcsGlyph};
pub use term::Terminal;
