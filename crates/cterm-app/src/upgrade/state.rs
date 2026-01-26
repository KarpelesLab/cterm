//! Upgrade state types for seamless process upgrade
//!
//! These types capture all the state needed to reconstruct terminal windows
//! after a seamless upgrade. They are serialized and passed to the new process.

use serde::{Deserialize, Serialize};

use cterm_core::cell::CellStyle;
use cterm_core::grid::{Grid, Row};
use cterm_core::screen::{Cursor, CursorStyle, MouseMode, ScrollRegion, TerminalModes};

/// Complete upgrade state for all windows
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeState {
    /// Version of the serialization format
    pub format_version: u32,
    /// Version of cterm that created this state
    pub cterm_version: String,
    /// All windows to restore
    pub windows: Vec<WindowUpgradeState>,
}

impl UpgradeState {
    /// Current format version
    /// Increment this when making incompatible changes to serialized types
    pub const FORMAT_VERSION: u32 = 2;

    /// Create a new upgrade state
    pub fn new(cterm_version: &str) -> Self {
        Self {
            format_version: Self::FORMAT_VERSION,
            cterm_version: cterm_version.to_string(),
            windows: Vec::new(),
        }
    }
}

/// State for a single window
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowUpgradeState {
    /// Window X position
    pub x: i32,
    /// Window Y position
    pub y: i32,
    /// Window width
    pub width: i32,
    /// Window height
    pub height: i32,
    /// Whether the window is maximized
    pub maximized: bool,
    /// Whether the window is fullscreen
    pub fullscreen: bool,
    /// All tabs in this window
    pub tabs: Vec<TabUpgradeState>,
    /// Index of the currently active tab
    pub active_tab: usize,
}

impl WindowUpgradeState {
    /// Create a new window upgrade state
    pub fn new() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
            maximized: false,
            fullscreen: false,
            tabs: Vec::new(),
            active_tab: 0,
        }
    }
}

impl Default for WindowUpgradeState {
    fn default() -> Self {
        Self::new()
    }
}

/// State for a single tab
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabUpgradeState {
    /// Unique tab ID
    pub id: u64,
    /// Tab title
    pub title: String,
    /// Tab color (if sticky tab)
    pub color: Option<String>,
    /// Template name (for sticky/unique tabs)
    pub template_name: Option<String>,
    /// Terminal state
    pub terminal: TerminalUpgradeState,
    /// Index into the FD array for this tab's PTY (Unix)
    /// Index into the handle array for this tab's PTY (Windows)
    pub pty_fd_index: usize,
    /// Child process ID (Unix)
    #[cfg(unix)]
    pub child_pid: i32,
    /// Child process ID (Windows)
    #[cfg(windows)]
    pub process_id: u32,
    /// Working directory of the shell
    pub cwd: Option<String>,
    /// Watchdog FD ID (for crash recovery matching)
    #[cfg(unix)]
    pub watchdog_fd_id: u64,
}

impl TabUpgradeState {
    /// Create a new tab upgrade state (Unix)
    #[cfg(unix)]
    pub fn new(id: u64, pty_fd_index: usize, child_pid: i32) -> Self {
        Self {
            id,
            title: String::new(),
            color: None,
            template_name: None,
            terminal: TerminalUpgradeState::default(),
            pty_fd_index,
            child_pid,
            cwd: None,
            watchdog_fd_id: 0,
        }
    }

    /// Create a new tab upgrade state (Windows)
    #[cfg(windows)]
    pub fn new(id: u64, pty_fd_index: usize, process_id: u32) -> Self {
        Self {
            id,
            title: String::new(),
            color: None,
            template_name: None,
            terminal: TerminalUpgradeState::default(),
            pty_fd_index,
            process_id,
            cwd: None,
        }
    }

    /// Create a new tab upgrade state with watchdog FD ID
    #[cfg(unix)]
    pub fn with_watchdog_fd_id(
        id: u64,
        pty_fd_index: usize,
        child_pid: i32,
        watchdog_fd_id: u64,
    ) -> Self {
        Self {
            id,
            title: String::new(),
            color: None,
            template_name: None,
            terminal: TerminalUpgradeState::default(),
            pty_fd_index,
            child_pid,
            cwd: None,
            watchdog_fd_id,
        }
    }
}

/// Terminal emulator state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalUpgradeState {
    /// Terminal width in columns
    pub cols: usize,
    /// Terminal height in rows
    pub rows: usize,
    /// Main screen grid
    pub grid: Grid,
    /// Scrollback buffer
    pub scrollback: Vec<Row>,
    /// Alternate screen grid (for vim, less, etc.)
    pub alternate_grid: Option<Grid>,
    /// Current cursor position and style
    pub cursor: Cursor,
    /// Saved cursor (for DECSC/DECRC)
    pub saved_cursor: Option<Cursor>,
    /// Alternate screen saved cursor
    pub alt_saved_cursor: Option<Cursor>,
    /// Scroll region
    pub scroll_region: ScrollRegion,
    /// Current cell style
    pub style: CellStyle,
    /// Terminal modes
    pub modes: TerminalModes,
    /// Terminal title
    pub title: String,
    /// Current scroll offset (for viewing scrollback)
    pub scroll_offset: usize,
    /// Tab stops
    pub tab_stops: Vec<bool>,
    /// Whether alternate screen is active
    pub alternate_active: bool,
    /// Cursor style
    pub cursor_style: CursorStyle,
    /// Mouse mode
    pub mouse_mode: MouseMode,
}

impl Default for TerminalUpgradeState {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            grid: Grid::new(80, 24),
            scrollback: Vec::new(),
            alternate_grid: None,
            cursor: Cursor::default(),
            saved_cursor: None,
            alt_saved_cursor: None,
            scroll_region: ScrollRegion { top: 0, bottom: 24 },
            style: CellStyle::default(),
            modes: TerminalModes::default(),
            title: String::new(),
            scroll_offset: 0,
            tab_stops: vec![false; 80],
            alternate_active: false,
            cursor_style: CursorStyle::default(),
            mouse_mode: MouseMode::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upgrade_state_serialization() {
        let state = UpgradeState::new("0.1.0");

        // Serialize with bincode
        let bytes = bincode::serialize(&state).expect("Failed to serialize");

        // Deserialize
        let restored: UpgradeState = bincode::deserialize(&bytes).expect("Failed to deserialize");

        assert_eq!(restored.format_version, UpgradeState::FORMAT_VERSION);
        assert_eq!(restored.cterm_version, "0.1.0");
        assert!(restored.windows.is_empty());
    }

    #[test]
    fn test_window_state_serialization() {
        let mut state = UpgradeState::new("0.1.0");

        let mut window = WindowUpgradeState::new();
        window.x = 100;
        window.y = 200;
        window.width = 1024;
        window.height = 768;
        window.maximized = true;

        state.windows.push(window);

        let bytes = bincode::serialize(&state).expect("Failed to serialize");
        let restored: UpgradeState = bincode::deserialize(&bytes).expect("Failed to deserialize");

        assert_eq!(restored.windows.len(), 1);
        assert_eq!(restored.windows[0].x, 100);
        assert!(restored.windows[0].maximized);
    }

    #[test]
    fn test_terminal_state_serialization() {
        let terminal = TerminalUpgradeState {
            cols: 120,
            rows: 40,
            ..Default::default()
        };

        let bytes = bincode::serialize(&terminal).expect("Failed to serialize");
        let restored: TerminalUpgradeState =
            bincode::deserialize(&bytes).expect("Failed to deserialize");

        assert_eq!(restored.cols, 120);
        assert_eq!(restored.rows, 40);
    }
}
