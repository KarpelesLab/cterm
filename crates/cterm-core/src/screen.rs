//! Screen - Terminal screen with scrollback buffer
//!
//! Manages the visible grid and scrollback history, handling resize
//! and scroll operations.

use std::collections::VecDeque;
use crate::grid::{Grid, Row};
use crate::cell::{Cell, CellStyle};

/// Configuration for the screen
#[derive(Debug, Clone)]
pub struct ScreenConfig {
    /// Maximum scrollback lines (0 = no scrollback)
    pub scrollback_lines: usize,
}

impl Default for ScreenConfig {
    fn default() -> Self {
        Self {
            scrollback_lines: 10000,
        }
    }
}

/// Cursor position and state
#[derive(Debug, Clone, Default)]
pub struct Cursor {
    /// Column position (0-indexed)
    pub col: usize,
    /// Row position (0-indexed)
    pub row: usize,
    /// Whether cursor is visible
    pub visible: bool,
    /// Cursor style
    pub style: CursorStyle,
    /// Whether cursor should blink
    pub blink: bool,
}

/// Cursor shape style
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CursorStyle {
    #[default]
    Block,
    Underline,
    Bar,
}

/// Scroll region bounds
#[derive(Debug, Clone, Copy)]
pub struct ScrollRegion {
    pub top: usize,
    pub bottom: usize,
}

impl ScrollRegion {
    pub fn contains(&self, row: usize) -> bool {
        row >= self.top && row < self.bottom
    }
}

/// Terminal modes that affect behavior
#[derive(Debug, Clone, Default)]
pub struct TerminalModes {
    /// Application cursor keys mode (DECCKM)
    pub application_cursor: bool,
    /// Application keypad mode (DECKPAM)
    pub application_keypad: bool,
    /// Auto-wrap mode (DECAWM)
    pub auto_wrap: bool,
    /// Origin mode (DECOM)
    pub origin_mode: bool,
    /// Insert mode (IRM)
    pub insert_mode: bool,
    /// Line feed/new line mode (LNM)
    pub line_feed_mode: bool,
    /// Show cursor (DECTCEM)
    pub show_cursor: bool,
    /// Mouse reporting mode
    pub mouse_mode: MouseMode,
    /// Bracketed paste mode
    pub bracketed_paste: bool,
    /// Focus events reporting
    pub focus_events: bool,
    /// Alternate screen buffer active
    pub alternate_screen: bool,
}

/// Mouse reporting modes
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MouseMode {
    #[default]
    None,
    /// X10 mouse reporting
    X10,
    /// Normal tracking mode
    Normal,
    /// Button event tracking
    ButtonEvent,
    /// Any event tracking
    AnyEvent,
}

/// Terminal screen state
#[derive(Debug)]
pub struct Screen {
    /// Active display grid
    grid: Grid,
    /// Scrollback buffer (oldest lines first)
    scrollback: VecDeque<Row>,
    /// Alternate screen buffer (for vim, less, etc.)
    alternate_grid: Option<Grid>,
    /// Screen configuration
    config: ScreenConfig,
    /// Cursor state
    pub cursor: Cursor,
    /// Saved cursor state (for save/restore)
    saved_cursor: Option<Cursor>,
    /// Alternate saved cursor (for alternate screen)
    alt_saved_cursor: Option<Cursor>,
    /// Scroll region
    scroll_region: ScrollRegion,
    /// Current cell styling
    pub style: CellStyle,
    /// Terminal modes
    pub modes: TerminalModes,
    /// Window title
    pub title: String,
    /// Icon name
    pub icon_name: String,
    /// Whether content has changed since last render
    pub dirty: bool,
    /// Current scroll offset (for viewing scrollback)
    pub scroll_offset: usize,
}

impl Screen {
    /// Create a new screen with the given dimensions
    pub fn new(width: usize, height: usize, config: ScreenConfig) -> Self {
        let mut modes = TerminalModes::default();
        modes.auto_wrap = true;
        modes.show_cursor = true;

        Self {
            grid: Grid::new(width, height),
            scrollback: VecDeque::with_capacity(config.scrollback_lines.min(1000)),
            alternate_grid: None,
            config,
            cursor: Cursor {
                visible: true,
                blink: true,
                ..Default::default()
            },
            saved_cursor: None,
            alt_saved_cursor: None,
            scroll_region: ScrollRegion {
                top: 0,
                bottom: height,
            },
            style: CellStyle::default(),
            modes,
            title: String::new(),
            icon_name: String::new(),
            dirty: true,
            scroll_offset: 0,
        }
    }

    /// Get screen width
    pub fn width(&self) -> usize {
        self.grid.width()
    }

    /// Get screen height
    pub fn height(&self) -> usize {
        self.grid.height()
    }

    /// Get the active grid
    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    /// Get a mutable reference to the active grid
    pub fn grid_mut(&mut self) -> &mut Grid {
        &mut self.grid
    }

    /// Get scroll region
    pub fn scroll_region(&self) -> &ScrollRegion {
        &self.scroll_region
    }

    /// Set scroll region
    pub fn set_scroll_region(&mut self, top: usize, bottom: usize) {
        let top = top.min(self.height().saturating_sub(1));
        let bottom = bottom.min(self.height()).max(top + 1);
        self.scroll_region = ScrollRegion { top, bottom };
    }

    /// Reset scroll region to full screen
    pub fn reset_scroll_region(&mut self) {
        self.scroll_region = ScrollRegion {
            top: 0,
            bottom: self.height(),
        };
    }

    /// Get scrollback buffer
    pub fn scrollback(&self) -> &VecDeque<Row> {
        &self.scrollback
    }

    /// Total lines (scrollback + visible)
    pub fn total_lines(&self) -> usize {
        self.scrollback.len() + self.height()
    }

    /// Resize the screen
    pub fn resize(&mut self, width: usize, height: usize) {
        if width == self.width() && height == self.height() {
            return;
        }

        self.grid.resize(width, height);

        if let Some(ref mut alt) = self.alternate_grid {
            alt.resize(width, height);
        }

        // Update scroll region
        if self.scroll_region.bottom == self.height() {
            self.scroll_region.bottom = height;
        } else {
            self.scroll_region.bottom = self.scroll_region.bottom.min(height);
        }
        self.scroll_region.top = self.scroll_region.top.min(height.saturating_sub(1));

        // Clamp cursor position
        self.cursor.col = self.cursor.col.min(width.saturating_sub(1));
        self.cursor.row = self.cursor.row.min(height.saturating_sub(1));

        self.dirty = true;
    }

    /// Get a cell at the given position
    pub fn get_cell(&self, row: usize, col: usize) -> Option<&Cell> {
        self.grid.get(row, col)
    }

    /// Get a cell from scrollback + visible area
    pub fn get_cell_with_scrollback(&self, line: usize, col: usize) -> Option<&Cell> {
        if line < self.scrollback.len() {
            self.scrollback.get(line)?.get(col)
        } else {
            let row = line - self.scrollback.len();
            self.grid.get(row, col)
        }
    }

    /// Put a character at the current cursor position
    pub fn put_char(&mut self, c: char) {
        let width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);

        // Handle auto-wrap
        if self.cursor.col >= self.width() {
            if self.modes.auto_wrap {
                self.carriage_return();
                self.line_feed();
                if let Some(row) = self.grid.row_mut(self.cursor.row) {
                    row.wrapped = true;
                }
            } else {
                self.cursor.col = self.width() - 1;
            }
        }

        // Insert mode: shift characters right
        if self.modes.insert_mode && self.cursor.col < self.width() {
            self.insert_cells(width);
        }

        // Write the character
        if let Some(cell) = self.grid.get_mut(self.cursor.row, self.cursor.col) {
            cell.c = c;
            self.style.apply_to(cell);

            if width > 1 {
                cell.attrs.insert(crate::cell::CellAttrs::WIDE);
            }
        }

        // Handle wide characters (write spacer in next cell)
        if width > 1 && self.cursor.col + 1 < self.width() {
            if let Some(cell) = self.grid.get_mut(self.cursor.row, self.cursor.col + 1) {
                cell.c = ' ';
                cell.attrs = crate::cell::CellAttrs::WIDE_SPACER;
            }
        }

        // Advance cursor
        self.cursor.col += width;
        self.dirty = true;
    }

    /// Insert blank cells at cursor, shifting existing cells right
    fn insert_cells(&mut self, count: usize) {
        let cursor_row = self.cursor.row;
        let cursor_col = self.cursor.col;
        let width = self.width();

        if let Some(row) = self.grid.row_mut(cursor_row) {
            for i in (cursor_col + count..width).rev() {
                let src_col = i - count;
                let src_cell = row[src_col].clone();
                row[i] = src_cell;
            }
            for i in cursor_col..cursor_col + count {
                if i < width {
                    row[i].reset();
                }
            }
        }
    }

    /// Move cursor to start of line
    pub fn carriage_return(&mut self) {
        self.cursor.col = 0;
    }

    /// Move cursor down, scrolling if needed
    pub fn line_feed(&mut self) {
        if self.cursor.row + 1 >= self.scroll_region.bottom {
            self.scroll_up(1);
        } else {
            self.cursor.row += 1;
        }
        self.dirty = true;
    }

    /// Scroll up within scroll region
    pub fn scroll_up(&mut self, count: usize) {
        let scrolled = self.grid.scroll_up(
            count,
            self.scroll_region.top,
            self.scroll_region.bottom,
        );

        // Add to scrollback if not in alternate screen and scrolling from top
        if !self.modes.alternate_screen && self.scroll_region.top == 0 {
            for row in scrolled {
                if self.scrollback.len() >= self.config.scrollback_lines {
                    self.scrollback.pop_front();
                }
                self.scrollback.push_back(row);
            }
        }

        self.dirty = true;
    }

    /// Scroll down within scroll region
    pub fn scroll_down(&mut self, count: usize) {
        self.grid.scroll_down(
            count,
            self.scroll_region.top,
            self.scroll_region.bottom,
        );
        self.dirty = true;
    }

    /// Move cursor to position
    pub fn move_cursor(&mut self, row: usize, col: usize) {
        let (base_row, max_row) = if self.modes.origin_mode {
            (self.scroll_region.top, self.scroll_region.bottom)
        } else {
            (0, self.height())
        };

        self.cursor.row = (base_row + row).min(max_row.saturating_sub(1));
        self.cursor.col = col.min(self.width().saturating_sub(1));
    }

    /// Move cursor relative to current position
    pub fn move_cursor_relative(&mut self, row_delta: i32, col_delta: i32) {
        let new_row = (self.cursor.row as i32 + row_delta)
            .max(0)
            .min(self.height() as i32 - 1) as usize;
        let new_col = (self.cursor.col as i32 + col_delta)
            .max(0)
            .min(self.width() as i32 - 1) as usize;

        self.cursor.row = new_row;
        self.cursor.col = new_col;
    }

    /// Save cursor state
    pub fn save_cursor(&mut self) {
        self.saved_cursor = Some(self.cursor.clone());
    }

    /// Restore cursor state
    pub fn restore_cursor(&mut self) {
        if let Some(saved) = self.saved_cursor.take() {
            self.cursor = saved;
        }
    }

    /// Switch to alternate screen buffer
    pub fn enter_alternate_screen(&mut self) {
        if self.modes.alternate_screen {
            return;
        }

        self.modes.alternate_screen = true;
        self.alt_saved_cursor = Some(self.cursor.clone());

        let alt = Grid::new(self.width(), self.height());
        self.alternate_grid = Some(std::mem::replace(&mut self.grid, alt));

        self.cursor = Cursor::default();
        self.cursor.visible = true;
        self.dirty = true;
    }

    /// Switch back to primary screen buffer
    pub fn exit_alternate_screen(&mut self) {
        if !self.modes.alternate_screen {
            return;
        }

        self.modes.alternate_screen = false;

        if let Some(primary) = self.alternate_grid.take() {
            self.grid = primary;
        }

        if let Some(saved) = self.alt_saved_cursor.take() {
            self.cursor = saved;
        }

        self.dirty = true;
    }

    /// Clear screen (or parts of it)
    pub fn clear(&mut self, mode: ClearMode) {
        let cursor_row = self.cursor.row;
        let cursor_col = self.cursor.col;
        let width = self.width();
        let height = self.height();

        match mode {
            ClearMode::Below => {
                // Clear from cursor to end of line
                if let Some(row) = self.grid.row_mut(cursor_row) {
                    for col in cursor_col..width {
                        row[col].reset();
                    }
                }
                // Clear all lines below
                for row_idx in cursor_row + 1..height {
                    if let Some(row) = self.grid.row_mut(row_idx) {
                        row.clear();
                    }
                }
            }
            ClearMode::Above => {
                // Clear all lines above
                for row_idx in 0..cursor_row {
                    if let Some(row) = self.grid.row_mut(row_idx) {
                        row.clear();
                    }
                }
                // Clear from start of line to cursor
                if let Some(row) = self.grid.row_mut(cursor_row) {
                    for col in 0..=cursor_col.min(width.saturating_sub(1)) {
                        row[col].reset();
                    }
                }
            }
            ClearMode::All => {
                self.grid.clear();
            }
            ClearMode::Scrollback => {
                self.scrollback.clear();
            }
        }
        self.dirty = true;
    }

    /// Clear line (or parts of it)
    pub fn clear_line(&mut self, mode: LineClearMode) {
        let cursor_row = self.cursor.row;
        let cursor_col = self.cursor.col;
        let width = self.width();

        let (start, end) = match mode {
            LineClearMode::Right => (cursor_col, width),
            LineClearMode::Left => (0, cursor_col + 1),
            LineClearMode::All => (0, width),
        };

        if let Some(row) = self.grid.row_mut(cursor_row) {
            for col in start..end.min(width) {
                row[col].reset();
            }
        }
        self.dirty = true;
    }

    /// Delete characters at cursor position
    pub fn delete_chars(&mut self, count: usize) {
        let cursor_row = self.cursor.row;
        let cursor_col = self.cursor.col;
        let width = self.width();
        let count = count.min(width.saturating_sub(cursor_col));

        if let Some(row) = self.grid.row_mut(cursor_row) {
            // Shift characters left
            for col in cursor_col..width.saturating_sub(count) {
                row[col] = row[col + count].clone();
            }

            // Clear the rightmost cells
            for col in width.saturating_sub(count)..width {
                row[col].reset();
            }
        }
        self.dirty = true;
    }

    /// Insert blank lines at cursor position
    pub fn insert_lines(&mut self, count: usize) {
        if !self.scroll_region.contains(self.cursor.row) {
            return;
        }

        // Scroll the region below cursor down
        let region_bottom = self.scroll_region.bottom;
        self.grid.scroll_down(count, self.cursor.row, region_bottom);
        self.cursor.col = 0;
        self.dirty = true;
    }

    /// Delete lines at cursor position
    pub fn delete_lines(&mut self, count: usize) {
        if !self.scroll_region.contains(self.cursor.row) {
            return;
        }

        // Scroll the region from cursor up
        let region_bottom = self.scroll_region.bottom;
        self.grid.scroll_up(count, self.cursor.row, region_bottom);
        self.cursor.col = 0;
        self.dirty = true;
    }

    /// Reset terminal state
    pub fn reset(&mut self) {
        self.grid.clear();
        self.scrollback.clear();
        self.alternate_grid = None;
        self.cursor = Cursor {
            visible: true,
            blink: true,
            ..Default::default()
        };
        self.saved_cursor = None;
        self.alt_saved_cursor = None;
        self.scroll_region = ScrollRegion {
            top: 0,
            bottom: self.height(),
        };
        self.style = CellStyle::default();
        self.modes = TerminalModes {
            auto_wrap: true,
            show_cursor: true,
            ..Default::default()
        };
        self.title.clear();
        self.icon_name.clear();
        self.dirty = true;
        self.scroll_offset = 0;
    }
}

/// Screen clear mode
#[derive(Debug, Clone, Copy)]
pub enum ClearMode {
    /// Clear from cursor to end of screen
    Below,
    /// Clear from start of screen to cursor
    Above,
    /// Clear entire screen
    All,
    /// Clear scrollback buffer
    Scrollback,
}

/// Line clear mode
#[derive(Debug, Clone, Copy)]
pub enum LineClearMode {
    /// Clear from cursor to end of line
    Right,
    /// Clear from start of line to cursor
    Left,
    /// Clear entire line
    All,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_screen_new() {
        let screen = Screen::new(80, 24, ScreenConfig::default());
        assert_eq!(screen.width(), 80);
        assert_eq!(screen.height(), 24);
        assert_eq!(screen.cursor.row, 0);
        assert_eq!(screen.cursor.col, 0);
    }

    #[test]
    fn test_put_char() {
        let mut screen = Screen::new(80, 24, ScreenConfig::default());

        screen.put_char('H');
        screen.put_char('i');

        assert_eq!(screen.get_cell(0, 0).unwrap().c, 'H');
        assert_eq!(screen.get_cell(0, 1).unwrap().c, 'i');
        assert_eq!(screen.cursor.col, 2);
    }

    #[test]
    fn test_auto_wrap() {
        let mut screen = Screen::new(5, 3, ScreenConfig::default());

        for c in "Hello World".chars() {
            screen.put_char(c);
        }

        assert_eq!(screen.grid().row(0).unwrap().text(), "Hello");
        assert_eq!(screen.grid().row(1).unwrap().text(), " Worl");
        assert_eq!(screen.grid().row(2).unwrap().text(), "d");
    }

    #[test]
    fn test_scroll_up() {
        let mut screen = Screen::new(80, 3, ScreenConfig::default());

        // Fill screen
        screen.put_char('1');
        screen.line_feed();
        screen.carriage_return();
        screen.put_char('2');
        screen.line_feed();
        screen.carriage_return();
        screen.put_char('3');
        screen.line_feed(); // This should scroll

        assert_eq!(screen.scrollback.len(), 1);
        assert_eq!(screen.scrollback[0][0].c, '1');
        assert_eq!(screen.grid()[0][0].c, '2');
        assert_eq!(screen.grid()[1][0].c, '3');
    }

    #[test]
    fn test_alternate_screen() {
        let mut screen = Screen::new(80, 24, ScreenConfig::default());

        screen.put_char('A');
        screen.enter_alternate_screen();

        // Alternate screen should be empty
        assert_eq!(screen.get_cell(0, 0).unwrap().c, ' ');

        screen.put_char('B');
        screen.exit_alternate_screen();

        // Should restore primary with 'A'
        assert_eq!(screen.get_cell(0, 0).unwrap().c, 'A');
    }

    #[test]
    fn test_clear_screen() {
        let mut screen = Screen::new(80, 24, ScreenConfig::default());

        screen.put_char('X');
        screen.clear(ClearMode::All);

        assert_eq!(screen.get_cell(0, 0).unwrap().c, ' ');
    }
}
