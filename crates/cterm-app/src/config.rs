//! Configuration management
//!
//! Handles loading, saving, and managing configuration files.

use std::collections::HashMap;
use std::path::PathBuf;

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use cterm_ui::theme::{FontConfig, Theme};

/// Configuration errors
#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    Read(#[from] std::io::Error),

    #[error("Failed to parse config file: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("Failed to serialize config: {0}")]
    Serialize(#[from] toml::ser::Error),

    #[error("Config directory not found")]
    NoConfigDir,
}

/// Main configuration struct
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// General settings
    pub general: GeneralConfig,
    /// Appearance settings
    pub appearance: AppearanceConfig,
    /// Tab settings
    pub tabs: TabsConfig,
    /// Shortcut bindings
    pub shortcuts: ShortcutsConfig,
    /// Sticky tabs configuration
    pub sticky_tabs: Vec<StickyTabConfig>,
}

/// General settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// Default shell to use (None = system default)
    pub default_shell: Option<String>,
    /// Shell arguments
    pub shell_args: Vec<String>,
    /// Scrollback buffer size
    pub scrollback_lines: usize,
    /// Confirm before closing with running process
    pub confirm_close_with_running: bool,
    /// Copy on select
    pub copy_on_select: bool,
    /// Working directory for new tabs
    pub working_directory: Option<PathBuf>,
    /// Environment variables to set
    pub env: HashMap<String, String>,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            default_shell: None,
            shell_args: Vec::new(),
            scrollback_lines: 10000,
            confirm_close_with_running: true,
            copy_on_select: false,
            working_directory: None,
            env: HashMap::new(),
        }
    }
}

/// Appearance settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    /// Theme name or "custom"
    pub theme: String,
    /// Custom theme (if theme = "custom")
    pub custom_theme: Option<Theme>,
    /// Font configuration
    pub font: FontConfig,
    /// Cursor style
    pub cursor_style: CursorStyleConfig,
    /// Cursor blink
    pub cursor_blink: bool,
    /// Opacity (0.0 - 1.0)
    pub opacity: f64,
    /// Padding around terminal content
    pub padding: u32,
    /// Enable bold text
    pub bold_is_bright: bool,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: "Default Dark".into(),
            custom_theme: None,
            font: FontConfig::default(),
            cursor_style: CursorStyleConfig::Block,
            cursor_blink: true,
            opacity: 1.0,
            padding: 4,
            bold_is_bright: false,
        }
    }
}

/// Cursor style options
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CursorStyleConfig {
    #[default]
    Block,
    Underline,
    Bar,
}

/// Tab settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TabsConfig {
    /// When to show tab bar
    pub show_tab_bar: TabBarVisibility,
    /// Tab bar position
    pub tab_bar_position: TabBarPosition,
    /// Where to insert new tabs
    pub new_tab_position: NewTabPosition,
    /// Show tab close button
    pub show_close_button: bool,
    /// Tab title format
    pub title_format: String,
}

impl Default for TabsConfig {
    fn default() -> Self {
        Self {
            show_tab_bar: TabBarVisibility::Always,
            tab_bar_position: TabBarPosition::Top,
            new_tab_position: NewTabPosition::End,
            show_close_button: true,
            title_format: "{title}".into(),
        }
    }
}

/// Tab bar visibility options
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TabBarVisibility {
    #[default]
    Always,
    Multiple,
    Never,
}

/// Tab bar position
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TabBarPosition {
    #[default]
    Top,
    Bottom,
}

/// Position for new tabs
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NewTabPosition {
    #[default]
    End,
    AfterCurrent,
}

/// Docker mode for sticky tabs
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DockerMode {
    /// Connect to a running container with `docker exec`
    #[default]
    Exec,
    /// Start a new container with `docker run`
    Run,
    /// Start a devcontainer with project/config mounts (like Claude Code/Cursor)
    DevContainer,
}

/// Docker-specific configuration for a sticky tab
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DockerTabConfig {
    /// Docker mode: exec (connect to running container), run (start new container),
    /// or devcontainer (start with project/config mounts)
    pub mode: DockerMode,
    /// Container name or ID (for exec mode)
    pub container: Option<String>,
    /// Image name with optional tag (for run/devcontainer mode)
    pub image: Option<String>,
    /// Shell to use inside the container (default: /bin/sh, or /bin/zsh for devcontainer)
    pub shell: Option<String>,
    /// Additional docker exec/run arguments (e.g., -v, --env)
    #[serde(default)]
    pub docker_args: Vec<String>,
    /// Auto-remove container on exit (run/devcontainer mode, default: true)
    #[serde(default = "default_true")]
    pub auto_remove: bool,
    /// Project directory to mount (devcontainer mode, default: current directory)
    pub project_dir: Option<PathBuf>,
    /// Mount ~/.claude config directory (devcontainer mode, default: true)
    #[serde(default = "default_true")]
    pub mount_claude_config: bool,
    /// Mount ~/.ssh directory for git operations (devcontainer mode, default: false)
    #[serde(default)]
    pub mount_ssh: bool,
    /// Mount ~/.gitconfig (devcontainer mode, default: true)
    #[serde(default = "default_true")]
    pub mount_gitconfig: bool,
    /// Working directory inside the container (default: /workspace)
    pub workdir: Option<String>,
    /// Container name (for devcontainer mode, to allow reconnecting)
    pub container_name: Option<String>,
    /// Run post-start command (e.g., firewall init)
    pub post_start_command: Option<String>,
}

fn default_true() -> bool {
    true
}

/// Keyboard shortcuts configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShortcutsConfig {
    pub new_tab: String,
    pub close_tab: String,
    pub next_tab: String,
    pub prev_tab: String,
    pub new_window: String,
    pub close_window: String,
    pub copy: String,
    pub paste: String,
    pub select_all: String,
    pub zoom_in: String,
    pub zoom_out: String,
    pub zoom_reset: String,
    pub scroll_up: String,
    pub scroll_down: String,
    pub scroll_page_up: String,
    pub scroll_page_down: String,
    pub preferences: String,
    pub find: String,
    pub reset: String,
}

impl Default for ShortcutsConfig {
    fn default() -> Self {
        Self {
            new_tab: "Ctrl+Shift+T".into(),
            close_tab: "Ctrl+Shift+W".into(),
            next_tab: "Ctrl+Tab".into(),
            prev_tab: "Ctrl+Shift+Tab".into(),
            new_window: "Ctrl+Shift+N".into(),
            close_window: "Ctrl+Shift+Q".into(),
            copy: "Ctrl+Shift+C".into(),
            paste: "Ctrl+Shift+V".into(),
            select_all: "Ctrl+Shift+A".into(),
            zoom_in: "Ctrl+Plus".into(),
            zoom_out: "Ctrl+Minus".into(),
            zoom_reset: "Ctrl+0".into(),
            scroll_up: "Shift+PageUp".into(),
            scroll_down: "Shift+PageDown".into(),
            scroll_page_up: "PageUp".into(),
            scroll_page_down: "PageDown".into(),
            preferences: "Ctrl+Comma".into(),
            find: "Ctrl+Shift+F".into(),
            reset: "Ctrl+Shift+R".into(),
        }
    }
}

/// Sticky tab configuration (tab template)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StickyTabConfig {
    /// Tab name (also used as unique identifier for the template)
    pub name: String,
    /// Command to run (None = default shell)
    pub command: Option<String>,
    /// Command arguments
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory
    pub working_directory: Option<PathBuf>,
    /// Tab color (hex)
    pub color: Option<String>,
    /// Theme override for this tab (None = use default theme)
    pub theme: Option<String>,
    /// Whether to auto-start this tab on launch
    #[serde(default)]
    pub auto_start: bool,
    /// Keep tab open after process exits
    #[serde(default)]
    pub keep_open: bool,
    /// Unique tab - if true, opening this template focuses existing tab instead of creating new one
    #[serde(default)]
    pub unique: bool,
    /// Environment variables
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Docker-specific configuration (if present, this is a Docker tab)
    pub docker: Option<DockerTabConfig>,
}

impl Default for StickyTabConfig {
    fn default() -> Self {
        Self {
            name: "New Tab".into(),
            command: None,
            args: Vec::new(),
            working_directory: None,
            color: None,
            theme: None,
            auto_start: false,
            keep_open: false,
            unique: false,
            env: HashMap::new(),
            docker: None,
        }
    }
}

impl StickyTabConfig {
    /// Create a Claude tab configuration
    pub fn claude() -> Self {
        Self {
            name: "Claude".into(),
            command: Some("claude".into()),
            args: Vec::new(),
            color: Some("#7c3aed".into()),
            auto_start: false,
            keep_open: true,
            unique: true, // Claude tabs are unique by default
            ..Default::default()
        }
    }

    /// Create a Claude continue session tab configuration
    pub fn claude_continue() -> Self {
        Self {
            name: "Claude (Continue)".into(),
            command: Some("claude".into()),
            args: vec!["-c".into()],
            color: Some("#7c3aed".into()),
            auto_start: false,
            keep_open: true,
            unique: true, // Claude tabs are unique by default
            ..Default::default()
        }
    }

    /// Create a Docker exec tab configuration (connect to running container)
    pub fn docker_exec(name: &str, container: &str) -> Self {
        Self {
            name: name.to_string(),
            color: Some("#0db7ed".to_string()), // Docker blue
            keep_open: true,
            docker: Some(DockerTabConfig {
                mode: DockerMode::Exec,
                container: Some(container.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Create a Docker run tab configuration (start new container from image)
    pub fn docker_run(name: &str, image: &str) -> Self {
        Self {
            name: name.to_string(),
            color: Some("#0db7ed".to_string()), // Docker blue
            docker: Some(DockerTabConfig {
                mode: DockerMode::Run,
                image: Some(image.to_string()),
                auto_remove: true,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Check if this is a Docker tab
    pub fn is_docker(&self) -> bool {
        self.docker.is_some()
    }

    /// Get the command and arguments for this sticky tab
    ///
    /// For Docker tabs, this builds the appropriate docker exec/run command.
    /// For regular tabs, this returns the configured command and args.
    pub fn get_command_args(&self) -> (Option<String>, Vec<String>) {
        if let Some(ref docker) = self.docker {
            match docker.mode {
                DockerMode::Exec => {
                    let container = docker.container.as_deref().unwrap_or("");
                    let shell = docker.shell.as_deref();
                    let (cmd, args) = crate::docker::build_exec_command(container, shell);
                    (Some(cmd), args)
                }
                DockerMode::Run => {
                    let image = docker.image.as_deref().unwrap_or("");
                    let shell = docker.shell.as_deref();
                    let (cmd, args) = crate::docker::build_run_command(
                        image,
                        shell,
                        docker.auto_remove,
                        &docker.docker_args,
                    );
                    (Some(cmd), args)
                }
                DockerMode::DevContainer => {
                    let (cmd, args) = crate::docker::build_devcontainer_command(docker);
                    (Some(cmd), args)
                }
            }
        } else {
            (self.command.clone(), self.args.clone())
        }
    }

    /// Create a Claude devcontainer tab configuration
    ///
    /// This creates a container with:
    /// - Project directory mounted to /workspace
    /// - ~/.claude mounted for credentials
    /// - ~/.gitconfig mounted for git configuration
    /// - Claude Code pre-installed (using anthropic's devcontainer image)
    pub fn claude_devcontainer(project_dir: Option<PathBuf>) -> Self {
        Self {
            name: "Claude Container".into(),
            color: Some("#7c3aed".into()), // Claude purple
            keep_open: true,
            docker: Some(DockerTabConfig {
                mode: DockerMode::DevContainer,
                image: Some("node:20".into()), // Base image, Claude Code installed via npm
                shell: Some("/bin/bash".into()),
                auto_remove: true,
                project_dir,
                mount_claude_config: true,
                mount_ssh: false,
                mount_gitconfig: true,
                workdir: Some("/workspace".into()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}

/// Get the config directory path
pub fn config_dir() -> Option<PathBuf> {
    ProjectDirs::from("com", "cterm", "cterm").map(|p| p.config_dir().to_path_buf())
}

/// Get the config file path
pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|p| p.join("config.toml"))
}

/// Get the sticky tabs file path
pub fn sticky_tabs_path() -> Option<PathBuf> {
    config_dir().map(|p| p.join("sticky_tabs.toml"))
}

/// Load configuration from file
pub fn load_config() -> Result<Config, ConfigError> {
    let path = config_path().ok_or(ConfigError::NoConfigDir)?;

    if !path.exists() {
        return Ok(Config::default());
    }

    let content = std::fs::read_to_string(&path)?;
    let config: Config = toml::from_str(&content)?;

    Ok(config)
}

/// Save configuration to file
pub fn save_config(config: &Config) -> Result<(), ConfigError> {
    let dir = config_dir().ok_or(ConfigError::NoConfigDir)?;
    std::fs::create_dir_all(&dir)?;

    let path = dir.join("config.toml");
    let content = toml::to_string_pretty(config)?;
    std::fs::write(&path, content)?;

    Ok(())
}

/// Load sticky tabs configuration
pub fn load_sticky_tabs() -> Result<Vec<StickyTabConfig>, ConfigError> {
    let path = sticky_tabs_path().ok_or(ConfigError::NoConfigDir)?;

    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = std::fs::read_to_string(&path)?;

    #[derive(Deserialize)]
    struct StickyTabsFile {
        tabs: Vec<StickyTabConfig>,
    }

    let file: StickyTabsFile = toml::from_str(&content)?;
    Ok(file.tabs)
}

/// Save sticky tabs configuration
pub fn save_sticky_tabs(tabs: &[StickyTabConfig]) -> Result<(), ConfigError> {
    let dir = config_dir().ok_or(ConfigError::NoConfigDir)?;
    std::fs::create_dir_all(&dir)?;

    let path = dir.join("sticky_tabs.toml");

    #[derive(Serialize)]
    struct StickyTabsFile<'a> {
        tabs: &'a [StickyTabConfig],
    }

    let file = StickyTabsFile { tabs };
    let content = toml::to_string_pretty(&file)?;
    std::fs::write(&path, content)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.general.scrollback_lines, 10000);
        assert!(config.general.confirm_close_with_running);
    }

    #[test]
    fn test_config_serialize() {
        let config = Config::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        assert!(serialized.contains("[general]"));
        assert!(serialized.contains("[appearance]"));
    }

    #[test]
    fn test_sticky_tab_claude() {
        let tab = StickyTabConfig::claude();
        assert_eq!(tab.name, "Claude");
        assert_eq!(tab.command, Some("claude".into()));
        assert!(tab.keep_open);
    }
}
