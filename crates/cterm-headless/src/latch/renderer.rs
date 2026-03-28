//! ANSI screen renderer for server-side rendering.
//!
//! Converts terminal screen state into ANSI escape sequences for
//! remote clients. Supports both full-screen renders (for initial
//! connection) and differential renders (for ongoing updates).

use cterm_core::cell::{Cell, CellAttrs};
use cterm_core::color::{Color, Rgb};
use cterm_core::screen::Screen;

/// Cached screen state for differential rendering.
#[derive(Clone)]
pub struct ScreenSnapshot {
    cells: Vec<Vec<CellSnapshot>>,
    cursor_row: usize,
    cursor_col: usize,
    cursor_visible: bool,
    cols: usize,
    rows: usize,
}

/// Snapshot of a single cell's visible state.
#[derive(Clone, PartialEq)]
struct CellSnapshot {
    ch: char,
    fg: Color,
    bg: Color,
    attrs: CellAttrs,
}

impl ScreenSnapshot {
    /// Capture the current screen state.
    pub fn capture(screen: &Screen) -> Self {
        let rows = screen.height();
        let cols = screen.width();
        let mut cells = Vec::with_capacity(rows);

        for row in 0..rows {
            let mut row_cells = Vec::with_capacity(cols);
            for col in 0..cols {
                if let Some(cell) = screen.get_cell(row, col) {
                    row_cells.push(CellSnapshot {
                        ch: cell.c,
                        fg: cell.fg,
                        bg: cell.bg,
                        attrs: cell.attrs,
                    });
                } else {
                    row_cells.push(CellSnapshot {
                        ch: ' ',
                        fg: Color::Default,
                        bg: Color::Default,
                        attrs: CellAttrs::empty(),
                    });
                }
            }
            cells.push(row_cells);
        }

        Self {
            cells,
            cursor_row: screen.cursor.row,
            cursor_col: screen.cursor.col,
            cursor_visible: screen.modes.show_cursor,
            cols,
            rows,
        }
    }
}

/// Render the full screen as ANSI escape sequences.
pub fn render_full(screen: &Screen) -> Vec<u8> {
    let mut out = Vec::with_capacity(screen.height() * screen.width() * 4);

    // Reset all attributes and clear screen
    out.extend_from_slice(b"\x1b[0m\x1b[2J\x1b[H");

    let rows = screen.height();
    let cols = screen.width();

    for row in 0..rows {
        if row > 0 {
            out.extend_from_slice(format!("\x1b[{};1H", row + 1).as_bytes());
        }

        let mut last_fg = Color::Default;
        let mut last_bg = Color::Default;
        let mut last_attrs = CellAttrs::empty();

        for col in 0..cols {
            if let Some(cell) = screen.get_cell(row, col) {
                // Skip wide spacers
                if cell.attrs.contains(CellAttrs::WIDE_SPACER) {
                    continue;
                }
                emit_sgr_diff(&mut out, last_fg, last_bg, last_attrs, cell);
                last_fg = cell.fg;
                last_bg = cell.bg;
                last_attrs = cell.attrs;
                emit_char(&mut out, cell.c);
            } else {
                out.push(b' ');
            }
        }
    }

    // Reset attributes
    out.extend_from_slice(b"\x1b[0m");

    // Position cursor
    out.extend_from_slice(
        format!("\x1b[{};{}H", screen.cursor.row + 1, screen.cursor.col + 1).as_bytes(),
    );

    // Cursor visibility
    if screen.modes.show_cursor {
        out.extend_from_slice(b"\x1b[?25h");
    } else {
        out.extend_from_slice(b"\x1b[?25l");
    }

    out
}

/// Render only the differences between old and new screen state.
pub fn render_diff(old: &ScreenSnapshot, screen: &Screen) -> Vec<u8> {
    let mut out = Vec::new();

    // If dimensions changed, do a full render
    if screen.height() != old.rows || screen.width() != old.cols {
        return render_full(screen);
    }

    let rows = screen.height();
    let cols = screen.width();

    for row in 0..rows {
        for col in 0..cols {
            let old_snap = &old.cells[row][col];
            if let Some(cell) = screen.get_cell(row, col) {
                if cell.attrs.contains(CellAttrs::WIDE_SPACER) {
                    continue;
                }
                let new_snap = CellSnapshot {
                    ch: cell.c,
                    fg: cell.fg,
                    bg: cell.bg,
                    attrs: cell.attrs,
                };
                if new_snap != *old_snap {
                    // Move cursor to this position and render cell
                    out.extend_from_slice(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes());
                    emit_sgr_full(&mut out, cell);
                    emit_char(&mut out, cell.c);
                }
            }
        }
    }

    // Reset attributes if we emitted anything
    if !out.is_empty() {
        out.extend_from_slice(b"\x1b[0m");
    }

    // Update cursor position
    let cursor_changed = screen.cursor.row != old.cursor_row
        || screen.cursor.col != old.cursor_col
        || screen.modes.show_cursor != old.cursor_visible
        || !out.is_empty();

    if cursor_changed {
        out.extend_from_slice(
            format!("\x1b[{};{}H", screen.cursor.row + 1, screen.cursor.col + 1).as_bytes(),
        );
        if screen.modes.show_cursor != old.cursor_visible {
            if screen.modes.show_cursor {
                out.extend_from_slice(b"\x1b[?25h");
            } else {
                out.extend_from_slice(b"\x1b[?25l");
            }
        }
    }

    out
}

/// Emit SGR sequences only for changed attributes.
fn emit_sgr_diff(
    out: &mut Vec<u8>,
    old_fg: Color,
    old_bg: Color,
    old_attrs: CellAttrs,
    cell: &Cell,
) {
    if cell.fg == old_fg && cell.bg == old_bg && cell.attrs == old_attrs {
        return;
    }
    emit_sgr_full(out, cell);
}

/// Emit full SGR reset + set for a cell.
fn emit_sgr_full(out: &mut Vec<u8>, cell: &Cell) {
    out.extend_from_slice(b"\x1b[0");

    let attrs = cell.attrs;
    if attrs.contains(CellAttrs::BOLD) {
        out.extend_from_slice(b";1");
    }
    if attrs.contains(CellAttrs::DIM) {
        out.extend_from_slice(b";2");
    }
    if attrs.contains(CellAttrs::ITALIC) {
        out.extend_from_slice(b";3");
    }
    if attrs.contains(CellAttrs::UNDERLINE) {
        out.extend_from_slice(b";4");
    }
    if attrs.contains(CellAttrs::BLINK) {
        out.extend_from_slice(b";5");
    }
    if attrs.contains(CellAttrs::INVERSE) {
        out.extend_from_slice(b";7");
    }
    if attrs.contains(CellAttrs::HIDDEN) {
        out.extend_from_slice(b";8");
    }
    if attrs.contains(CellAttrs::STRIKETHROUGH) {
        out.extend_from_slice(b";9");
    }

    emit_color_fg(out, cell.fg);
    emit_color_bg(out, cell.bg);

    out.push(b'm');
}

fn emit_color_fg(out: &mut Vec<u8>, color: Color) {
    match color {
        Color::Default => {}
        Color::Ansi(ac) => {
            let idx = ac as u8;
            if idx < 8 {
                out.extend_from_slice(format!(";{}", 30 + idx).as_bytes());
            } else {
                out.extend_from_slice(format!(";{}", 90 + idx - 8).as_bytes());
            }
        }
        Color::Indexed(i) => {
            out.extend_from_slice(format!(";38;5;{}", i).as_bytes());
        }
        Color::Rgb(Rgb { r, g, b }) => {
            out.extend_from_slice(format!(";38;2;{};{};{}", r, g, b).as_bytes());
        }
    }
}

fn emit_color_bg(out: &mut Vec<u8>, color: Color) {
    match color {
        Color::Default => {}
        Color::Ansi(ac) => {
            let idx = ac as u8;
            if idx < 8 {
                out.extend_from_slice(format!(";{}", 40 + idx).as_bytes());
            } else {
                out.extend_from_slice(format!(";{}", 100 + idx - 8).as_bytes());
            }
        }
        Color::Indexed(i) => {
            out.extend_from_slice(format!(";48;5;{}", i).as_bytes());
        }
        Color::Rgb(Rgb { r, g, b }) => {
            out.extend_from_slice(format!(";48;2;{};{};{}", r, g, b).as_bytes());
        }
    }
}

/// Emit a character, handling special cases.
fn emit_char(out: &mut Vec<u8>, ch: char) {
    if ch == '\0' || ch == ' ' {
        out.push(b' ');
    } else {
        let mut buf = [0u8; 4];
        let s = ch.encode_utf8(&mut buf);
        out.extend_from_slice(s.as_bytes());
    }
}
