//! Upgrade state types for seamless process upgrade
//!
//! These types capture the window/tab layout needed to reconstruct terminal
//! windows after a seamless upgrade. Terminal session state lives in the
//! ctermd daemon and is referenced by session ID.

use serde::{Deserialize, Serialize};

/// Complete upgrade state for all windows
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeState {
    /// Version of the serialization format
    pub format_version: u32,
    /// All windows to restore
    pub windows: Vec<WindowUpgradeState>,
}

impl UpgradeState {
    /// Current format version
    /// Increment this when making incompatible changes to serialized types
    pub const FORMAT_VERSION: u32 = 4;

    /// Create a new upgrade state
    pub fn new() -> Self {
        Self {
            format_version: Self::FORMAT_VERSION,
            windows: Vec::new(),
        }
    }
}

impl Default for UpgradeState {
    fn default() -> Self {
        Self::new()
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
    /// Custom title set by user (locks out OSC title updates when Some)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_title: Option<String>,
    /// Tab color (if sticky tab)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Template name (for sticky/unique tabs)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_name: Option<String>,
    /// Daemon session ID for reconnecting to the running session
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Working directory of the shell
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Keep the tab open after the process exits
    #[serde(default)]
    pub keep_open: bool,
}

impl TabUpgradeState {
    /// Create a new tab upgrade state
    pub fn new(id: u64) -> Self {
        Self {
            id,
            title: String::new(),
            custom_title: None,
            color: None,
            template_name: None,
            session_id: None,
            cwd: None,
            keep_open: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upgrade_state_serialization() {
        let state = UpgradeState::new();

        let json = serde_json::to_vec(&state).expect("Failed to serialize");
        let restored: UpgradeState = serde_json::from_slice(&json).expect("Failed to deserialize");

        assert_eq!(restored.format_version, UpgradeState::FORMAT_VERSION);
        assert!(restored.windows.is_empty());
    }

    #[test]
    fn test_window_state_serialization() {
        let mut state = UpgradeState::new();

        let mut window = WindowUpgradeState::new();
        window.x = 100;
        window.y = 200;
        window.width = 1024;
        window.height = 768;
        window.maximized = true;

        state.windows.push(window);

        let json = serde_json::to_vec(&state).expect("Failed to serialize");
        let restored: UpgradeState = serde_json::from_slice(&json).expect("Failed to deserialize");

        assert_eq!(restored.windows.len(), 1);
        assert_eq!(restored.windows[0].x, 100);
        assert!(restored.windows[0].maximized);
    }

    #[test]
    fn test_tab_state_serialization() {
        let mut tab = TabUpgradeState::new(42);
        tab.title = "My Tab".to_string();
        tab.session_id = Some("sess-abc123".to_string());
        tab.cwd = Some("/home/user".to_string());
        tab.keep_open = true;

        let json = serde_json::to_vec(&tab).expect("Failed to serialize");
        let restored: TabUpgradeState =
            serde_json::from_slice(&json).expect("Failed to deserialize");

        assert_eq!(restored.id, 42);
        assert_eq!(restored.title, "My Tab");
        assert_eq!(restored.session_id.as_deref(), Some("sess-abc123"));
        assert_eq!(restored.cwd.as_deref(), Some("/home/user"));
        assert!(restored.keep_open);
    }

    #[test]
    fn test_tab_state_optional_fields_omitted() {
        let tab = TabUpgradeState::new(1);

        let json = serde_json::to_string(&tab).expect("Failed to serialize");

        // Optional None fields should be omitted from JSON
        assert!(!json.contains("custom_title"));
        assert!(!json.contains("color"));
        assert!(!json.contains("template_name"));
        assert!(!json.contains("session_id"));
        assert!(!json.contains("cwd"));
    }

    #[test]
    fn test_full_roundtrip() {
        let mut state = UpgradeState::new();

        let mut window = WindowUpgradeState::new();
        window.x = 50;
        window.y = 100;
        window.width = 1920;
        window.height = 1080;
        window.fullscreen = true;
        window.active_tab = 1;

        let mut tab0 = TabUpgradeState::new(1);
        tab0.title = "bash".to_string();
        tab0.session_id = Some("sess-001".to_string());

        let mut tab1 = TabUpgradeState::new(2);
        tab1.title = "vim".to_string();
        tab1.custom_title = Some("Editor".to_string());
        tab1.session_id = Some("sess-002".to_string());
        tab1.color = Some("#ff0000".to_string());
        tab1.template_name = Some("dev".to_string());
        tab1.keep_open = true;

        window.tabs.push(tab0);
        window.tabs.push(tab1);
        state.windows.push(window);

        let json = serde_json::to_vec_pretty(&state).expect("Failed to serialize");
        let restored: UpgradeState = serde_json::from_slice(&json).expect("Failed to deserialize");

        assert_eq!(restored.format_version, 4);
        assert_eq!(restored.windows.len(), 1);

        let w = &restored.windows[0];
        assert_eq!(w.width, 1920);
        assert!(w.fullscreen);
        assert_eq!(w.active_tab, 1);
        assert_eq!(w.tabs.len(), 2);

        assert_eq!(w.tabs[0].title, "bash");
        assert_eq!(w.tabs[0].session_id.as_deref(), Some("sess-001"));
        assert!(!w.tabs[0].keep_open);

        assert_eq!(w.tabs[1].title, "vim");
        assert_eq!(w.tabs[1].custom_title.as_deref(), Some("Editor"));
        assert_eq!(w.tabs[1].color.as_deref(), Some("#ff0000"));
        assert!(w.tabs[1].keep_open);
    }
}
