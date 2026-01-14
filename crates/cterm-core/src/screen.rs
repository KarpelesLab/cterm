//! Screen - Terminal screen with scrollback buffer
//!
//! Manages the visible grid and scrollback history, handling resize
//! and scroll operations.

use crate::cell::{Cell, CellStyle};
use crate::grid::{Grid, Row};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Configuration for the screen
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CursorStyle {
    #[default]
    Block,
    Underline,
    Bar,
}

/// Scroll region bounds
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
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
    /// Active charset (true = G1, false = G0) - controlled by SO/SI
    pub charset_g1_active: bool,
}

/// Character set designations
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Charset {
    /// ASCII (USASCII)
    #[default]
    Ascii,
    /// DEC Special Graphics (line drawing)
    DecSpecialGraphics,
    /// UK character set
    Uk,
}

/// Mouse reporting modes
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// Clipboard selection type for OSC 52
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardSelection {
    /// System clipboard (c)
    Clipboard,
    /// Primary selection (p)
    Primary,
    /// Both clipboard and primary (s)
    Select,
}

/// Clipboard operation from OSC 52
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClipboardOperation {
    /// Set clipboard content (base64 decoded data)
    Set {
        selection: ClipboardSelection,
        data: Vec<u8>,
    },
    /// Query clipboard content
    Query { selection: ClipboardSelection },
}

/// Color query type (OSC 10-12)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorQuery {
    /// Query foreground color (OSC 10)
    Foreground,
    /// Query background color (OSC 11)
    Background,
    /// Query cursor color (OSC 12)
    Cursor,
}

/// A point in the terminal buffer (absolute line index + column)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionPoint {
    /// Absolute line index (0 = oldest scrollback line)
    pub line: usize,
    /// Column position
    pub col: usize,
}

impl SelectionPoint {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }

    /// Returns true if self comes before other in reading order
    pub fn is_before(&self, other: &SelectionPoint) -> bool {
        self.line < other.line || (self.line == other.line && self.col < other.col)
    }
}

impl PartialOrd for SelectionPoint {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SelectionPoint {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.line.cmp(&other.line) {
            std::cmp::Ordering::Equal => self.col.cmp(&other.col),
            ord => ord,
        }
    }
}

/// Text selection state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Selection {
    /// Starting point of selection (where mouse was pressed)
    pub anchor: SelectionPoint,
    /// Current end point of selection (where mouse is now)
    pub end: SelectionPoint,
    /// Selection type (char, word, line)
    pub mode: SelectionMode,
}

/// Selection granularity mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SelectionMode {
    /// Character-by-character selection (single click drag)
    #[default]
    Char,
    /// Word selection (double-click)
    Word,
    /// Line selection (triple-click)
    Line,
}

impl Selection {
    /// Create a new selection starting at a point
    pub fn new(point: SelectionPoint, mode: SelectionMode) -> Self {
        Self {
            anchor: point,
            end: point,
            mode,
        }
    }

    /// Get the start and end points in reading order (start <= end)
    pub fn ordered(&self) -> (SelectionPoint, SelectionPoint) {
        if self.anchor.is_before(&self.end) {
            (self.anchor, self.end)
        } else {
            (self.end, self.anchor)
        }
    }

    /// Check if a cell at (line, col) is within the selection
    pub fn contains(&self, line: usize, col: usize) -> bool {
        let (start, end) = self.ordered();

        if line < start.line || line > end.line {
            return false;
        }

        if start.line == end.line {
            // Single line selection
            col >= start.col && col <= end.col
        } else if line == start.line {
            // First line of multi-line selection
            col >= start.col
        } else if line == end.line {
            // Last line of multi-line selection
            col <= end.col
        } else {
            // Middle lines are fully selected
            true
        }
    }

    /// Update the end point of the selection
    pub fn extend_to(&mut self, point: SelectionPoint) {
        self.end = point;
    }
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
    /// Bell was triggered (should be cleared after notification)
    pub bell: bool,
    /// Tab stop positions (columns where tabs stop)
    tab_stops: Vec<bool>,
    /// Pending responses to send back to the PTY (for DSR etc)
    pending_responses: Vec<Vec<u8>>,
    /// Pending clipboard operations from OSC 52
    pending_clipboard_ops: Vec<ClipboardOperation>,
    /// Pending color queries (OSC 10-12)
    pending_color_queries: Vec<ColorQuery>,
    /// Current text selection (if any)
    pub selection: Option<Selection>,
}

impl Screen {
    /// Create a screen restored from upgrade state
    ///
    /// This is used during seamless upgrades to restore the terminal state
    /// from the old process.
    #[allow(clippy::too_many_arguments)]
    pub fn from_upgrade_state(
        grid: crate::grid::Grid,
        scrollback: Vec<crate::grid::Row>,
        alternate_grid: Option<crate::grid::Grid>,
        cursor: Cursor,
        saved_cursor: Option<Cursor>,
        alt_saved_cursor: Option<Cursor>,
        scroll_region: ScrollRegion,
        style: crate::cell::CellStyle,
        modes: TerminalModes,
        title: String,
        scroll_offset: usize,
        tab_stops: Vec<bool>,
        config: ScreenConfig,
    ) -> Self {
        Self {
            grid,
            scrollback: scrollback.into(),
            alternate_grid,
            config,
            cursor,
            saved_cursor,
            alt_saved_cursor,
            scroll_region,
            style,
            modes,
            title,
            icon_name: String::new(),
            dirty: true,
            scroll_offset,
            bell: false,
            tab_stops,
            pending_responses: Vec::new(),
            pending_clipboard_ops: Vec::new(),
            pending_color_queries: Vec::new(),
            selection: None,
        }
    }

    /// Create a new screen with the given dimensions
    pub fn new(width: usize, height: usize, config: ScreenConfig) -> Self {
        let modes = TerminalModes {
            auto_wrap: true,
            show_cursor: true,
            ..Default::default()
        };

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
            bell: false,
            tab_stops: Self::default_tab_stops(width),
            pending_responses: Vec::new(),
            pending_clipboard_ops: Vec::new(),
            pending_color_queries: Vec::new(),
            selection: None,
        }
    }

    /// Queue a response to be sent back through the PTY
    pub fn queue_response(&mut self, response: Vec<u8>) {
        self.pending_responses.push(response);
    }

    /// Queue a clipboard operation (from OSC 52)
    pub fn queue_clipboard_op(&mut self, op: ClipboardOperation) {
        self.pending_clipboard_ops.push(op);
    }

    /// Take all pending clipboard operations (drains the queue)
    pub fn take_clipboard_ops(&mut self) -> Vec<ClipboardOperation> {
        std::mem::take(&mut self.pending_clipboard_ops)
    }

    /// Check if there are pending clipboard operations
    pub fn has_clipboard_ops(&self) -> bool {
        !self.pending_clipboard_ops.is_empty()
    }

    /// Queue a color query (from OSC 10-12)
    pub fn queue_color_query(&mut self, osc_code: u8) {
        let query = match osc_code {
            10 => ColorQuery::Foreground,
            11 => ColorQuery::Background,
            12 => ColorQuery::Cursor,
            _ => return,
        };
        self.pending_color_queries.push(query);
    }

    /// Take all pending color queries (drains the queue)
    pub fn take_color_queries(&mut self) -> Vec<ColorQuery> {
        std::mem::take(&mut self.pending_color_queries)
    }

    /// Check if there are pending color queries
    pub fn has_color_queries(&self) -> bool {
        !self.pending_color_queries.is_empty()
    }

    /// Take all pending responses (drains the queue)
    pub fn take_pending_responses(&mut self) -> Vec<Vec<u8>> {
        std::mem::take(&mut self.pending_responses)
    }

    /// Check if there are pending responses
    pub fn has_pending_responses(&self) -> bool {
        !self.pending_responses.is_empty()
    }

    /// Create default tab stops (every 8 columns)
    fn default_tab_stops(width: usize) -> Vec<bool> {
        (0..width).map(|i| i % 8 == 0 && i > 0).collect()
    }

    /// Set a tab stop at the current cursor position
    pub fn set_tab_stop(&mut self) {
        let col = self.cursor.col;
        if col < self.tab_stops.len() {
            self.tab_stops[col] = true;
        }
    }

    /// Clear tab stop at current cursor position
    pub fn clear_tab_stop(&mut self) {
        let col = self.cursor.col;
        if col < self.tab_stops.len() {
            self.tab_stops[col] = false;
        }
    }

    /// Clear all tab stops
    pub fn clear_all_tab_stops(&mut self) {
        self.tab_stops.fill(false);
    }

    /// Move cursor to the next tab stop
    pub fn tab_forward(&mut self, count: usize) {
        let width = self.width();
        for _ in 0..count {
            // Find next tab stop
            let mut next_col = self.cursor.col + 1;
            while next_col < width && !self.tab_stops.get(next_col).copied().unwrap_or(false) {
                next_col += 1;
            }
            // If no tab stop found, go to the last column
            self.cursor.col = next_col.min(width.saturating_sub(1));
        }
        self.dirty = true;
    }

    /// Move cursor to the previous tab stop
    pub fn tab_backward(&mut self, count: usize) {
        for _ in 0..count {
            // Find previous tab stop
            if self.cursor.col == 0 {
                break;
            }
            let mut prev_col = self.cursor.col - 1;
            while prev_col > 0 && !self.tab_stops.get(prev_col).copied().unwrap_or(false) {
                prev_col -= 1;
            }
            // If no tab stop found, go to column 0
            self.cursor.col = prev_col;
        }
        self.dirty = true;
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

    /// Get alternate grid if active
    pub fn alternate_grid(&self) -> Option<&Grid> {
        self.alternate_grid.as_ref()
    }

    /// Get saved cursor
    pub fn saved_cursor(&self) -> Option<&Cursor> {
        self.saved_cursor.as_ref()
    }

    /// Get alternate saved cursor
    pub fn alt_saved_cursor(&self) -> Option<&Cursor> {
        self.alt_saved_cursor.as_ref()
    }

    /// Get tab stops
    pub fn tab_stops(&self) -> &[bool] {
        &self.tab_stops
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
        let scrolled =
            self.grid
                .scroll_up(count, self.scroll_region.top, self.scroll_region.bottom);

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
        self.grid
            .scroll_down(count, self.scroll_region.top, self.scroll_region.bottom);
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

    /// Search for text in scrollback and visible buffer
    ///
    /// Returns all matches found, starting from the oldest scrollback line.
    /// Line index 0 is the oldest scrollback line, and increases toward
    /// the most recent visible line.
    pub fn find(&self, pattern: &str, case_sensitive: bool, regex: bool) -> Vec<SearchResult> {
        let mut results = Vec::new();

        if pattern.is_empty() {
            return results;
        }

        // Build the regex or prepare for simple search
        let regex_pattern = if regex {
            match regex::RegexBuilder::new(pattern)
                .case_insensitive(!case_sensitive)
                .build()
            {
                Ok(re) => Some(re),
                Err(_) => return results, // Invalid regex
            }
        } else {
            None
        };

        let search_pattern = if !case_sensitive && !regex {
            pattern.to_lowercase()
        } else {
            pattern.to_string()
        };

        // Search scrollback
        for (line_idx, row) in self.scrollback.iter().enumerate() {
            self.search_row(
                row,
                line_idx,
                &search_pattern,
                case_sensitive,
                &regex_pattern,
                &mut results,
            );
        }

        // Search visible grid
        let scrollback_len = self.scrollback.len();
        for row_idx in 0..self.grid.height() {
            if let Some(row) = self.grid.row(row_idx) {
                self.search_row(
                    row,
                    scrollback_len + row_idx,
                    &search_pattern,
                    case_sensitive,
                    &regex_pattern,
                    &mut results,
                );
            }
        }

        results
    }

    /// Search a single row for matches
    fn search_row(
        &self,
        row: &crate::grid::Row,
        line_idx: usize,
        pattern: &str,
        case_sensitive: bool,
        regex_pattern: &Option<regex::Regex>,
        results: &mut Vec<SearchResult>,
    ) {
        let line_text = row.text();

        if let Some(re) = regex_pattern {
            for m in re.find_iter(&line_text) {
                results.push(SearchResult {
                    line: line_idx,
                    col: m.start(),
                    len: m.len(),
                });
            }
        } else {
            // Simple string search
            let search_text = if case_sensitive {
                line_text.clone()
            } else {
                line_text.to_lowercase()
            };

            let mut start = 0;
            while let Some(pos) = search_text[start..].find(pattern) {
                let col = start + pos;
                results.push(SearchResult {
                    line: line_idx,
                    col,
                    len: pattern.len(),
                });
                start = col + 1;
            }
        }
    }

    /// Convert a line index from find() to scroll offset
    ///
    /// Returns the scroll offset needed to show the given line at the top of the visible area.
    pub fn line_to_scroll_offset(&self, line_idx: usize) -> usize {
        let scrollback_len = self.scrollback.len();
        // If line is in scrollback, return offset; otherwise 0 for visible area
        scrollback_len.saturating_sub(line_idx)
    }

    // ========== Selection Methods ==========

    /// Check if a character is a word character (for word selection)
    fn is_word_char(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }

    /// Find word boundaries around a column position in a row
    fn find_word_bounds(&self, line: usize, col: usize) -> (usize, usize) {
        let row = match self.get_row_by_absolute_line(line) {
            Some(r) => r,
            None => return (col, col),
        };

        let row_len = row.len();
        if row_len == 0 || col >= row_len {
            return (col, col);
        }

        let center_char = row.get(col).map(|c| c.c).unwrap_or(' ');

        // If we clicked on a non-word character, just select that character
        if !Self::is_word_char(center_char) {
            return (col, col);
        }

        // Find start of word
        let mut start = col;
        while start > 0 {
            if let Some(cell) = row.get(start - 1) {
                if Self::is_word_char(cell.c) {
                    start -= 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        // Find end of word
        let mut end = col;
        while end < row_len - 1 {
            if let Some(cell) = row.get(end + 1) {
                if Self::is_word_char(cell.c) {
                    end += 1;
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        (start, end)
    }

    /// Start a new selection at the given absolute line and column
    pub fn start_selection(&mut self, line: usize, col: usize, mode: SelectionMode) {
        match mode {
            SelectionMode::Char => {
                let point = SelectionPoint::new(line, col);
                self.selection = Some(Selection::new(point, mode));
            }
            SelectionMode::Word => {
                let (start_col, end_col) = self.find_word_bounds(line, col);
                let anchor = SelectionPoint::new(line, start_col);
                let end = SelectionPoint::new(line, end_col);
                self.selection = Some(Selection {
                    anchor,
                    end,
                    mode,
                });
            }
            SelectionMode::Line => {
                // Select entire line (use large end column to select to end of line)
                let anchor = SelectionPoint::new(line, 0);
                let end = SelectionPoint::new(line, usize::MAX);
                self.selection = Some(Selection {
                    anchor,
                    end,
                    mode,
                });
            }
        }
        self.dirty = true;
    }

    /// Extend the current selection to the given absolute line and column
    pub fn extend_selection(&mut self, line: usize, col: usize) {
        // Extract mode and anchor info before mutating
        let (mode, anchor_line, anchor_col) = match &self.selection {
            Some(s) => (s.mode, s.anchor.line, s.anchor.col),
            None => return,
        };

        match mode {
            SelectionMode::Char => {
                if let Some(ref mut selection) = self.selection {
                    selection.extend_to(SelectionPoint::new(line, col));
                }
            }
            SelectionMode::Word => {
                // Find word bounds before mutating selection
                let (word_start, word_end) = self.find_word_bounds(line, col);
                if let Some(ref mut selection) = self.selection {
                    if line < anchor_line || (line == anchor_line && col < anchor_col) {
                        // Extending backwards - use start of word
                        selection.extend_to(SelectionPoint::new(line, word_start));
                    } else {
                        // Extending forwards - use end of word
                        selection.extend_to(SelectionPoint::new(line, word_end));
                    }
                }
            }
            SelectionMode::Line => {
                if let Some(ref mut selection) = self.selection {
                    // Line mode extends to full lines
                    if line < anchor_line {
                        // Extending upward - start of new line to end of anchor line
                        selection.end = SelectionPoint::new(line, 0);
                    } else {
                        // Extending downward - end of new line
                        selection.end = SelectionPoint::new(line, usize::MAX);
                    }
                }
            }
        }
        self.dirty = true;
    }

    /// Clear the current selection
    pub fn clear_selection(&mut self) {
        if self.selection.is_some() {
            self.selection = None;
            self.dirty = true;
        }
    }

    /// Check if a cell at the given absolute line and column is selected
    pub fn is_selected(&self, line: usize, col: usize) -> bool {
        self.selection
            .as_ref()
            .map(|s| s.contains(line, col))
            .unwrap_or(false)
    }

    /// Convert visible row (accounting for scroll offset) to absolute line index
    pub fn visible_row_to_absolute_line(&self, visible_row: usize) -> usize {
        let scrollback_len = self.scrollback.len();
        // When scroll_offset is 0, we see the most recent scrollback + current grid
        // visible_row 0 = oldest visible line
        // scrollback_len - scroll_offset = first visible scrollback line index
        // After scrollback, grid rows start
        scrollback_len.saturating_sub(self.scroll_offset) + visible_row
    }

    /// Get the selected text as a string
    ///
    /// Returns None if there's no selection or it's empty
    pub fn get_selected_text(&self) -> Option<String> {
        let selection = self.selection.as_ref()?;
        let (start, end) = selection.ordered();

        // Clamp to valid range
        let total = self.total_lines();
        if start.line >= total {
            return None;
        }

        let mut result = String::new();
        let end_line = end.line.min(total - 1);

        for line_idx in start.line..=end_line {
            let row = self.get_row_by_absolute_line(line_idx)?;

            let start_col = if line_idx == start.line {
                start.col
            } else {
                0
            };
            let end_col = if line_idx == end.line {
                end.col.min(row.len().saturating_sub(1))
            } else {
                row.len().saturating_sub(1)
            };

            // Extract characters from this row
            for col in start_col..=end_col {
                if let Some(cell) = row.get(col) {
                    // Skip wide character spacers
                    if !cell.attrs.contains(crate::cell::CellAttrs::WIDE_SPACER) {
                        result.push(cell.c);
                    }
                }
            }

            // Add newline between lines (but not after wrapped lines or at the very end)
            if line_idx < end_line && !row.wrapped {
                result.push('\n');
            }
        }

        // Trim trailing whitespace from each line but keep newlines
        let trimmed: String = result
            .lines()
            .map(|l| l.trim_end())
            .collect::<Vec<_>>()
            .join("\n");

        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }

    /// Get a row by absolute line index (0 = oldest scrollback line)
    fn get_row_by_absolute_line(&self, line: usize) -> Option<&Row> {
        let scrollback_len = self.scrollback.len();
        if line < scrollback_len {
            self.scrollback.get(line)
        } else {
            let grid_row = line - scrollback_len;
            self.grid.row(grid_row)
        }
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

/// Search result in terminal buffer
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Line index (0 = oldest scrollback line)
    pub line: usize,
    /// Column where match starts
    pub col: usize,
    /// Length of match
    pub len: usize,
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
