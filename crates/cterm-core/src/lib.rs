//! cterm-core: Core terminal emulation library
//!
//! This crate provides the fundamental building blocks for terminal emulation:
//! - Color and cell attribute types
//! - Screen buffer management (grid, scrollback)
//! - ANSI/VT sequence parsing
//! - Cross-platform PTY handling

pub mod color;
pub mod cell;
pub mod grid;
pub mod screen;
pub mod parser;
pub mod pty;
pub mod term;

pub use color::{Color, AnsiColor, Rgb};
pub use cell::{Cell, CellAttrs};
pub use grid::Grid;
pub use screen::Screen;
pub use parser::Parser;
pub use pty::Pty;
pub use term::Terminal;
