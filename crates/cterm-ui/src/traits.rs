//! UI abstraction traits
//!
//! These traits define the interface that any UI backend must implement
//! to work with cterm.

use crate::events::InputEvent;
use crate::theme::Theme;
use cterm_core::color::ColorPalette;
use cterm_core::screen::Screen;

/// Terminal view trait - renders a terminal screen
pub trait TerminalView {
    /// Set the screen to render
    fn set_screen(&mut self, screen: &Screen);

    /// Set the color palette
    fn set_palette(&mut self, palette: &ColorPalette);

    /// Set the font
    fn set_font(&mut self, family: &str, size: f64);

    /// Request a redraw
    fn queue_redraw(&mut self);

    /// Get the cell size in pixels
    fn cell_size(&self) -> (f64, f64);

    /// Calculate grid dimensions for a given pixel size
    fn grid_dimensions(&self, width: f64, height: f64) -> (usize, usize) {
        let (cw, ch) = self.cell_size();
        let cols = (width / cw).floor() as usize;
        let rows = (height / ch).floor() as usize;
        (cols.max(1), rows.max(1))
    }
}

/// Tab information
#[derive(Debug, Clone)]
pub struct TabInfo {
    /// Unique tab ID
    pub id: u64,
    /// Tab title
    pub title: String,
    /// Whether this is a sticky tab
    pub sticky: bool,
    /// Tab color (optional)
    pub color: Option<String>,
    /// Whether the tab is active
    pub active: bool,
    /// Whether there's unread output
    pub has_unread: bool,
}

/// Tab bar trait - manages tabs
pub trait TabBar {
    /// Add a new tab
    fn add_tab(&mut self, info: TabInfo);

    /// Remove a tab
    fn remove_tab(&mut self, id: u64);

    /// Update tab info
    fn update_tab(&mut self, info: TabInfo);

    /// Set the active tab
    fn set_active(&mut self, id: u64);

    /// Get the active tab ID
    fn active_tab(&self) -> Option<u64>;

    /// Get all tab IDs in order
    fn tab_ids(&self) -> Vec<u64>;

    /// Reorder tabs
    fn reorder(&mut self, from: usize, to: usize);
}

/// Window trait - main application window
pub trait Window {
    type TerminalView: TerminalView;
    type TabBar: TabBar;

    /// Get the terminal view
    fn terminal_view(&self) -> &Self::TerminalView;

    /// Get a mutable reference to the terminal view
    fn terminal_view_mut(&mut self) -> &mut Self::TerminalView;

    /// Get the tab bar
    fn tab_bar(&self) -> &Self::TabBar;

    /// Get a mutable reference to the tab bar
    fn tab_bar_mut(&mut self) -> &mut Self::TabBar;

    /// Set the window title
    fn set_title(&mut self, title: &str);

    /// Get the window size in pixels
    fn size(&self) -> (f64, f64);

    /// Set the window size
    fn set_size(&mut self, width: f64, height: f64);

    /// Show the window
    fn show(&mut self);

    /// Hide the window
    fn hide(&mut self);

    /// Close the window
    fn close(&mut self);

    /// Check if the window is focused
    fn is_focused(&self) -> bool;

    /// Present the window (bring to front)
    fn present(&mut self);
}

/// Application trait - main application controller
pub trait Application {
    type Window: Window;

    /// Create a new window
    fn create_window(&mut self) -> Self::Window;

    /// Get all windows
    fn windows(&self) -> Vec<&Self::Window>;

    /// Set the theme
    fn set_theme(&mut self, theme: &Theme);

    /// Show the preferences dialog
    fn show_preferences(&mut self);

    /// Quit the application
    fn quit(&mut self);

    /// Run the main loop
    fn run(&mut self);
}

/// Clipboard operations
pub trait Clipboard {
    /// Get text from clipboard
    fn get_text(&self) -> Option<String>;

    /// Set text to clipboard
    fn set_text(&mut self, text: &str);

    /// Get text from primary selection (X11)
    fn get_primary(&self) -> Option<String>;

    /// Set text to primary selection (X11)
    fn set_primary(&mut self, text: &str);
}

/// Dialog operations
pub trait Dialogs {
    /// Show an error dialog
    fn show_error(&self, title: &str, message: &str);

    /// Show a confirmation dialog
    fn show_confirm(&self, title: &str, message: &str) -> bool;

    /// Show an input dialog
    fn show_input(&self, title: &str, message: &str, default: &str) -> Option<String>;
}
