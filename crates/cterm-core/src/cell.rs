//! Terminal cell types
//!
//! A cell represents a single character position in the terminal grid,
//! including its character, colors, and attributes.

use std::sync::Arc;
use bitflags::bitflags;
use crate::color::Color;

bitflags! {
    /// Cell rendering attributes
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct CellAttrs: u16 {
        /// Bold/bright text
        const BOLD = 1 << 0;
        /// Italic text
        const ITALIC = 1 << 1;
        /// Underlined text
        const UNDERLINE = 1 << 2;
        /// Double underline
        const DOUBLE_UNDERLINE = 1 << 3;
        /// Curly underline (undercurl)
        const CURLY_UNDERLINE = 1 << 4;
        /// Dotted underline
        const DOTTED_UNDERLINE = 1 << 5;
        /// Dashed underline
        const DASHED_UNDERLINE = 1 << 6;
        /// Blinking text
        const BLINK = 1 << 7;
        /// Reverse video (swap fg/bg)
        const INVERSE = 1 << 8;
        /// Hidden/invisible text
        const HIDDEN = 1 << 9;
        /// Strikethrough text
        const STRIKETHROUGH = 1 << 10;
        /// Dim/faint text
        const DIM = 1 << 11;
        /// Overline
        const OVERLINE = 1 << 12;
        /// Wide character (takes 2 cells)
        const WIDE = 1 << 13;
        /// Placeholder for second cell of wide char
        const WIDE_SPACER = 1 << 14;
    }
}

impl CellAttrs {
    /// Check if any underline style is set
    pub fn has_underline(&self) -> bool {
        self.intersects(
            Self::UNDERLINE
                | Self::DOUBLE_UNDERLINE
                | Self::CURLY_UNDERLINE
                | Self::DOTTED_UNDERLINE
                | Self::DASHED_UNDERLINE,
        )
    }

    /// Clear all underline styles
    pub fn clear_underline(&mut self) {
        self.remove(
            Self::UNDERLINE
                | Self::DOUBLE_UNDERLINE
                | Self::CURLY_UNDERLINE
                | Self::DOTTED_UNDERLINE
                | Self::DASHED_UNDERLINE,
        );
    }
}

/// Hyperlink information (OSC 8)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hyperlink {
    /// Unique ID for the hyperlink (optional)
    pub id: Option<String>,
    /// The URI target
    pub uri: String,
}

impl Hyperlink {
    pub fn new(uri: String) -> Self {
        Self { id: None, uri }
    }

    pub fn with_id(id: String, uri: String) -> Self {
        Self { id: Some(id), uri }
    }
}

/// A single terminal cell
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// The character in this cell
    pub c: char,
    /// Foreground color
    pub fg: Color,
    /// Background color
    pub bg: Color,
    /// Underline color (if different from fg)
    pub underline_color: Option<Color>,
    /// Cell attributes (bold, italic, etc.)
    pub attrs: CellAttrs,
    /// Hyperlink if present (shared via Arc for efficiency)
    pub hyperlink: Option<Arc<Hyperlink>>,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            c: ' ',
            fg: Color::Default,
            bg: Color::Default,
            underline_color: None,
            attrs: CellAttrs::empty(),
            hyperlink: None,
        }
    }
}

impl Cell {
    /// Create a new cell with the given character
    pub fn new(c: char) -> Self {
        Self {
            c,
            ..Default::default()
        }
    }

    /// Create an empty (space) cell
    pub fn empty() -> Self {
        Self::default()
    }

    /// Check if this cell is empty (space with default colors and no attrs)
    pub fn is_empty(&self) -> bool {
        self.c == ' '
            && self.fg == Color::Default
            && self.bg == Color::Default
            && self.attrs.is_empty()
            && self.hyperlink.is_none()
    }

    /// Check if this cell is a wide character
    pub fn is_wide(&self) -> bool {
        self.attrs.contains(CellAttrs::WIDE)
    }

    /// Check if this cell is a spacer for a wide character
    pub fn is_wide_spacer(&self) -> bool {
        self.attrs.contains(CellAttrs::WIDE_SPACER)
    }

    /// Reset cell to empty state
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Copy attributes from another cell (colors and attrs, not character)
    pub fn copy_style_from(&mut self, other: &Cell) {
        self.fg = other.fg;
        self.bg = other.bg;
        self.underline_color = other.underline_color.clone();
        self.attrs = other.attrs;
        self.hyperlink = other.hyperlink.clone();
    }
}

/// Current terminal styling state (used when writing new characters)
#[derive(Debug, Clone, Default)]
pub struct CellStyle {
    pub fg: Color,
    pub bg: Color,
    pub underline_color: Option<Color>,
    pub attrs: CellAttrs,
    pub hyperlink: Option<Arc<Hyperlink>>,
}

impl CellStyle {
    /// Apply this style to a cell
    pub fn apply_to(&self, cell: &mut Cell) {
        cell.fg = self.fg;
        cell.bg = self.bg;
        cell.underline_color = self.underline_color.clone();
        cell.attrs = self.attrs;
        cell.hyperlink = self.hyperlink.clone();
    }

    /// Create a cell with this style and the given character
    pub fn create_cell(&self, c: char) -> Cell {
        Cell {
            c,
            fg: self.fg,
            bg: self.bg,
            underline_color: self.underline_color.clone(),
            attrs: self.attrs,
            hyperlink: self.hyperlink.clone(),
        }
    }

    /// Reset to default style
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cell_default() {
        let cell = Cell::default();
        assert_eq!(cell.c, ' ');
        assert!(cell.is_empty());
    }

    #[test]
    fn test_cell_not_empty() {
        let mut cell = Cell::new('A');
        assert!(!cell.is_empty());

        cell = Cell::default();
        cell.fg = Color::Ansi(crate::color::AnsiColor::Red);
        assert!(!cell.is_empty());
    }

    #[test]
    fn test_cell_attrs() {
        let mut attrs = CellAttrs::BOLD | CellAttrs::UNDERLINE;
        assert!(attrs.contains(CellAttrs::BOLD));
        assert!(attrs.has_underline());

        attrs.clear_underline();
        assert!(!attrs.has_underline());
        assert!(attrs.contains(CellAttrs::BOLD));
    }

    #[test]
    fn test_cell_style_apply() {
        let style = CellStyle {
            fg: Color::Ansi(crate::color::AnsiColor::Red),
            attrs: CellAttrs::BOLD,
            ..Default::default()
        };

        let cell = style.create_cell('X');
        assert_eq!(cell.c, 'X');
        assert_eq!(cell.fg, Color::Ansi(crate::color::AnsiColor::Red));
        assert!(cell.attrs.contains(CellAttrs::BOLD));
    }
}
