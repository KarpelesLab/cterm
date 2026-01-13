//! Session management
//!
//! Handles terminal sessions, tabs, and window state.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use cterm_core::pty::{Pty, PtyConfig, PtyError};
use cterm_core::screen::ScreenConfig;
use cterm_core::term::Terminal;

use crate::config::StickyTabConfig;

/// Global tab ID counter
static TAB_ID_COUNTER: AtomicU64 = AtomicU64::new(1);
/// Global window ID counter
static WINDOW_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Generate a unique tab ID
pub fn next_tab_id() -> u64 {
    TAB_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// Generate a unique window ID
pub fn next_window_id() -> u64 {
    WINDOW_ID_COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// Tab state
pub struct TabState {
    /// Unique tab ID
    pub id: u64,
    /// Terminal instance
    pub terminal: Terminal,
    /// Tab title (custom or from terminal)
    pub title: String,
    /// Custom title set by user
    pub custom_title: Option<String>,
    /// Sticky tab config (if this is a sticky tab)
    pub sticky_config: Option<StickyTabConfig>,
    /// Tab color override
    pub color: Option<String>,
    /// Whether there's unread output
    pub has_unread: bool,
    /// Working directory
    pub cwd: Option<PathBuf>,
}

impl TabState {
    /// Create a new tab with default shell
    pub fn new(cols: usize, rows: usize) -> Result<Self, PtyError> {
        let screen_config = ScreenConfig::default();
        let pty_config = PtyConfig::default();

        let terminal = Terminal::with_shell(cols, rows, screen_config, &pty_config)?;

        Ok(Self {
            id: next_tab_id(),
            terminal,
            title: "Terminal".into(),
            custom_title: None,
            sticky_config: None,
            color: None,
            has_unread: false,
            cwd: None,
        })
    }

    /// Create a new tab from a sticky tab configuration
    pub fn from_sticky(
        config: &StickyTabConfig,
        cols: usize,
        rows: usize,
    ) -> Result<Self, PtyError> {
        let screen_config = ScreenConfig::default();

        let pty_config = PtyConfig {
            shell: config.command.clone(),
            args: config.args.clone(),
            cwd: config.working_directory.clone(),
            env: config.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            ..Default::default()
        };

        let terminal = Terminal::with_shell(cols, rows, screen_config, &pty_config)?;

        Ok(Self {
            id: next_tab_id(),
            terminal,
            title: config.name.clone(),
            custom_title: Some(config.name.clone()),
            sticky_config: Some(config.clone()),
            color: config.color.clone(),
            has_unread: false,
            cwd: config.working_directory.clone(),
        })
    }

    /// Create a tab with a custom command
    pub fn with_command(
        command: &str,
        args: &[String],
        cwd: Option<PathBuf>,
        cols: usize,
        rows: usize,
    ) -> Result<Self, PtyError> {
        let screen_config = ScreenConfig::default();

        let pty_config = PtyConfig {
            shell: Some(command.to_string()),
            args: args.to_vec(),
            cwd: cwd.clone(),
            ..Default::default()
        };

        let terminal = Terminal::with_shell(cols, rows, screen_config, &pty_config)?;

        Ok(Self {
            id: next_tab_id(),
            terminal,
            title: command.to_string(),
            custom_title: None,
            sticky_config: None,
            color: None,
            has_unread: false,
            cwd,
        })
    }

    /// Get the display title
    pub fn display_title(&self) -> &str {
        self.custom_title.as_ref().unwrap_or(&self.title)
    }

    /// Update title from terminal
    pub fn update_title_from_terminal(&mut self) {
        if self.custom_title.is_none() {
            let term_title = self.terminal.title();
            if !term_title.is_empty() {
                self.title = term_title.to_string();
            }
        }
    }

    /// Check if this is a sticky tab
    pub fn is_sticky(&self) -> bool {
        self.sticky_config.is_some()
    }

    /// Check if process is running
    pub fn is_running(&self) -> bool {
        self.terminal.is_running()
    }
}

/// Window state
pub struct WindowState {
    /// Unique window ID
    pub id: u64,
    /// Tabs in this window
    pub tabs: Vec<TabState>,
    /// Active tab index
    pub active_tab: usize,
    /// Window geometry
    pub geometry: WindowGeometry,
}

impl WindowState {
    /// Create a new window with one tab
    pub fn new(cols: usize, rows: usize) -> Result<Self, PtyError> {
        let tab = TabState::new(cols, rows)?;

        Ok(Self {
            id: next_window_id(),
            tabs: vec![tab],
            active_tab: 0,
            geometry: WindowGeometry::default(),
        })
    }

    /// Get the active tab
    pub fn active_tab(&self) -> Option<&TabState> {
        self.tabs.get(self.active_tab)
    }

    /// Get the active tab mutably
    pub fn active_tab_mut(&mut self) -> Option<&mut TabState> {
        self.tabs.get_mut(self.active_tab)
    }

    /// Add a new tab
    pub fn add_tab(&mut self, tab: TabState, position: TabPosition) {
        match position {
            TabPosition::End => {
                self.tabs.push(tab);
                self.active_tab = self.tabs.len() - 1;
            }
            TabPosition::AfterCurrent => {
                let idx = (self.active_tab + 1).min(self.tabs.len());
                self.tabs.insert(idx, tab);
                self.active_tab = idx;
            }
        }
    }

    /// Close a tab by index
    pub fn close_tab(&mut self, index: usize) -> Option<TabState> {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return None;
        }

        let tab = self.tabs.remove(index);

        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len() - 1;
        } else if self.active_tab > index {
            self.active_tab -= 1;
        }

        Some(tab)
    }

    /// Switch to a tab by index
    pub fn switch_to_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_tab = index;
            if let Some(tab) = self.tabs.get_mut(index) {
                tab.has_unread = false;
            }
        }
    }

    /// Switch to next tab
    pub fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active_tab = (self.active_tab + 1) % self.tabs.len();
            self.tabs[self.active_tab].has_unread = false;
        }
    }

    /// Switch to previous tab
    pub fn prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active_tab = if self.active_tab == 0 {
                self.tabs.len() - 1
            } else {
                self.active_tab - 1
            };
            self.tabs[self.active_tab].has_unread = false;
        }
    }

    /// Move tab from one position to another
    pub fn move_tab(&mut self, from: usize, to: usize) {
        if from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }

        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);

        // Update active tab index
        if self.active_tab == from {
            self.active_tab = to;
        } else if from < self.active_tab && to >= self.active_tab {
            self.active_tab -= 1;
        } else if from > self.active_tab && to <= self.active_tab {
            self.active_tab += 1;
        }
    }

    /// Find tab by ID
    pub fn find_tab(&self, id: u64) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// Resize all terminals in this window
    pub fn resize(&mut self, cols: usize, rows: usize) {
        for tab in &mut self.tabs {
            tab.terminal.resize(cols, rows);
        }
    }
}

/// Where to insert new tabs
#[derive(Debug, Clone, Copy)]
pub enum TabPosition {
    End,
    AfterCurrent,
}

/// Window geometry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
            maximized: false,
        }
    }
}

/// Session state (serializable for persistence)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    /// Window states
    pub windows: Vec<WindowSessionState>,
}

/// Window session state (serializable)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowSessionState {
    /// Window geometry
    pub geometry: WindowGeometry,
    /// Tabs (only sticky tabs are saved)
    pub tabs: Vec<TabSessionState>,
    /// Active tab index
    pub active_tab: usize,
}

/// Tab session state (serializable)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabSessionState {
    /// Sticky tab name (for matching)
    pub sticky_name: Option<String>,
    /// Working directory
    pub cwd: Option<PathBuf>,
}

impl SessionState {
    /// Save session state
    pub fn save(&self) -> Result<(), std::io::Error> {
        let path = crate::config::config_dir()
            .map(|p| p.join("session.toml"))
            .ok_or_else(|| std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Config directory not found",
            ))?;

        let content = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        std::fs::write(path, content)
    }

    /// Load session state
    pub fn load() -> Result<Self, std::io::Error> {
        let path = crate::config::config_dir()
            .map(|p| p.join("session.toml"))
            .ok_or_else(|| std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Config directory not found",
            ))?;

        if !path.exists() {
            return Ok(Self { windows: Vec::new() });
        }

        let content = std::fs::read_to_string(path)?;
        toml::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

/// Application session managing all windows
pub struct Session {
    /// All windows
    pub windows: Vec<WindowState>,
    /// Active window index
    pub active_window: usize,
}

impl Session {
    /// Create a new session with one window
    pub fn new(cols: usize, rows: usize) -> Result<Self, PtyError> {
        let window = WindowState::new(cols, rows)?;

        Ok(Self {
            windows: vec![window],
            active_window: 0,
        })
    }

    /// Get the active window
    pub fn active_window(&self) -> Option<&WindowState> {
        self.windows.get(self.active_window)
    }

    /// Get the active window mutably
    pub fn active_window_mut(&mut self) -> Option<&mut WindowState> {
        self.windows.get_mut(self.active_window)
    }

    /// Create a new window
    pub fn new_window(&mut self, cols: usize, rows: usize) -> Result<u64, PtyError> {
        let window = WindowState::new(cols, rows)?;
        let id = window.id;
        self.windows.push(window);
        self.active_window = self.windows.len() - 1;
        Ok(id)
    }

    /// Close a window
    pub fn close_window(&mut self, index: usize) -> bool {
        if self.windows.len() <= 1 || index >= self.windows.len() {
            return false;
        }

        self.windows.remove(index);

        if self.active_window >= self.windows.len() {
            self.active_window = self.windows.len() - 1;
        }

        true
    }

    /// Find window by ID
    pub fn find_window(&self, id: u64) -> Option<usize> {
        self.windows.iter().position(|w| w.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_id_generation() {
        let id1 = next_tab_id();
        let id2 = next_tab_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_window_geometry_default() {
        let geo = WindowGeometry::default();
        assert_eq!(geo.width, 800);
        assert_eq!(geo.height, 600);
    }
}
