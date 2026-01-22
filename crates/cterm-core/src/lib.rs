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
pub mod image_decode;
pub mod iterm2;
pub mod parser;
pub mod pty;
pub mod screen;
pub mod sixel;
pub mod streaming_file;
pub mod term;

pub use cell::{Cell, CellAttrs};
pub use color::{AnsiColor, Color, Rgb};
pub use grid::Grid;
pub use parser::Parser;
pub use pty::{Pty, PtyConfig, PtyError, PtySize};
pub use screen::{
    ClipboardOperation, ClipboardSelection, ColorQuery, FileTransferOperation, Screen,
    SearchResult, Selection, SelectionMode, SelectionPoint, TerminalImage,
};
pub use image_decode::{decode_image, DecodedImage, ImageDecodeError};
pub use iterm2::{Iterm2Dimension, Iterm2FileParams};
pub use sixel::{SixelDecoder, SixelImage};
pub use streaming_file::{StreamingFileData, StreamingFileReceiver, StreamingFileResult};
pub use drcs::{DecdldDecoder, DrcsFont, DrcsGlyph};
pub use term::Terminal;
