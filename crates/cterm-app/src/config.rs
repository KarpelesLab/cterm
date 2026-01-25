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
    /// TERM environment variable (default: xterm-256color)
    /// Common values: xterm-256color, xterm-direct, screen-256color
    pub term: Option<String>,
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
            term: None,
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

/// SSH port forwarding configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshPortForward {
    /// Local port to bind
    pub local_port: u16,
    /// Remote host (default: localhost)
    #[serde(default = "default_localhost")]
    pub remote_host: String,
    /// Remote port to forward to
    pub remote_port: u16,
}

impl SshPortForward {
    /// Parse port forwards from a comma-separated string.
    ///
    /// Supports two formats:
    /// - `local_port:remote_port` (assumes remote_host is "localhost")
    /// - `local_port:remote_host:remote_port`
    ///
    /// Example: "8080:80,3000:localhost:3000,5432:db.example.com:5432"
    pub fn parse_list(input: &str) -> Vec<SshPortForward> {
        if input.is_empty() {
            return Vec::new();
        }

        input
            .split(',')
            .filter_map(|part| {
                let parts: Vec<&str> = part.trim().split(':').collect();
                match parts.len() {
                    2 => {
                        // local_port:remote_port (assume localhost)
                        let local_port = parts[0].parse().ok()?;
                        let remote_port = parts[1].parse().ok()?;
                        Some(SshPortForward {
                            local_port,
                            remote_host: "localhost".to_string(),
                            remote_port,
                        })
                    }
                    3 => {
                        // local_port:host:remote_port
                        let local_port = parts[0].parse().ok()?;
                        let remote_host = parts[1].to_string();
                        let remote_port = parts[2].parse().ok()?;
                        Some(SshPortForward {
                            local_port,
                            remote_host,
                            remote_port,
                        })
                    }
                    _ => None,
                }
            })
            .collect()
    }
}

fn default_localhost() -> String {
    "localhost".to_string()
}

/// SSH-specific configuration for a sticky tab
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SshTabConfig {
    /// Remote host (hostname or IP address)
    pub host: String,
    /// SSH port (default: 22)
    pub port: Option<u16>,
    /// Username for SSH connection
    pub username: Option<String>,
    /// Path to identity file (private key)
    pub identity_file: Option<PathBuf>,
    /// Local port forwards (-L)
    #[serde(default)]
    pub local_forwards: Vec<SshPortForward>,
    /// Remote port forwards (-R)
    #[serde(default)]
    pub remote_forwards: Vec<SshPortForward>,
    /// Dynamic port forward / SOCKS proxy (-D)
    pub dynamic_forward: Option<u16>,
    /// Enable X11 forwarding (-X)
    #[serde(default)]
    pub x11_forward: bool,
    /// Enable SSH agent forwarding (-A)
    #[serde(default)]
    pub agent_forward: bool,
    /// Request a pseudo-terminal (default: true for interactive)
    #[serde(default = "default_true")]
    pub request_tty: bool,
    /// Remote command to execute (instead of shell)
    pub remote_command: Option<String>,
    /// Additional SSH options (passed as -o key=value)
    #[serde(default)]
    pub options: std::collections::HashMap<String, String>,
    /// Additional raw SSH arguments
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// Jump host / proxy (-J)
    pub jump_host: Option<String>,
}

impl SshTabConfig {
    /// Build SSH command and arguments
    pub fn build_command(&self) -> (String, Vec<String>) {
        let mut args = Vec::new();

        // Port
        if let Some(port) = self.port {
            if port != 22 {
                args.push("-p".to_string());
                args.push(port.to_string());
            }
        }

        // Identity file
        if let Some(ref identity) = self.identity_file {
            args.push("-i".to_string());
            args.push(identity.to_string_lossy().to_string());
        }

        // Local port forwards
        for fwd in &self.local_forwards {
            args.push("-L".to_string());
            args.push(format!(
                "{}:{}:{}",
                fwd.local_port, fwd.remote_host, fwd.remote_port
            ));
        }

        // Remote port forwards
        for fwd in &self.remote_forwards {
            args.push("-R".to_string());
            args.push(format!(
                "{}:{}:{}",
                fwd.local_port, fwd.remote_host, fwd.remote_port
            ));
        }

        // Dynamic forward (SOCKS proxy)
        if let Some(port) = self.dynamic_forward {
            args.push("-D".to_string());
            args.push(port.to_string());
        }

        // X11 forwarding
        if self.x11_forward {
            args.push("-X".to_string());
        }

        // Agent forwarding
        if self.agent_forward {
            args.push("-A".to_string());
        }

        // TTY allocation
        if self.request_tty {
            args.push("-t".to_string());
        }

        // Jump host
        if let Some(ref jump) = self.jump_host {
            args.push("-J".to_string());
            args.push(jump.clone());
        }

        // SSH options
        for (key, value) in &self.options {
            args.push("-o".to_string());
            args.push(format!("{}={}", key, value));
        }

        // Extra args
        args.extend(self.extra_args.iter().cloned());

        // Build destination: user@host or just host
        let destination = if let Some(ref user) = self.username {
            format!("{}@{}", user, self.host)
        } else {
            self.host.clone()
        };
        args.push(destination);

        // Remote command
        if let Some(ref cmd) = self.remote_command {
            args.push(cmd.clone());
        }

        ("ssh".to_string(), args)
    }
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
    /// Locked background color (hex) - overrides theme background
    pub background_color: Option<String>,
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
    /// SSH-specific configuration (if present, this is an SSH remote tab)
    pub ssh: Option<SshTabConfig>,
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
            background_color: None,
            auto_start: false,
            keep_open: false,
            unique: false,
            env: HashMap::new(),
            docker: None,
            ssh: None,
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

    /// Check if this is an SSH remote tab
    pub fn is_ssh(&self) -> bool {
        self.ssh.is_some()
    }

    /// Get the command and arguments for this sticky tab
    ///
    /// For Docker tabs, this builds the appropriate docker exec/run command.
    /// For SSH tabs, this builds the ssh command with configured options.
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
        } else if let Some(ref ssh) = self.ssh {
            let (cmd, args) = ssh.build_command();
            (Some(cmd), args)
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

    /// Create an Ubuntu container tab configuration
    pub fn ubuntu() -> Self {
        Self {
            name: "Ubuntu".into(),
            color: Some("#E95420".into()), // Ubuntu orange
            keep_open: true,
            docker: Some(DockerTabConfig {
                mode: DockerMode::Run,
                image: Some("ubuntu:latest".into()),
                shell: Some("/bin/bash".into()),
                auto_remove: true,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Create an Alpine container tab configuration
    pub fn alpine() -> Self {
        Self {
            name: "Alpine".into(),
            color: Some("#0D597F".into()), // Alpine blue
            keep_open: true,
            docker: Some(DockerTabConfig {
                mode: DockerMode::Run,
                image: Some("alpine:latest".into()),
                shell: Some("/bin/sh".into()),
                auto_remove: true,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Create a Node.js container tab configuration
    pub fn nodejs() -> Self {
        Self {
            name: "Node.js".into(),
            color: Some("#339933".into()), // Node.js green
            keep_open: true,
            docker: Some(DockerTabConfig {
                mode: DockerMode::Run,
                image: Some("node:20".into()),
                shell: Some("/bin/bash".into()),
                auto_remove: true,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Create a Python container tab configuration
    pub fn python() -> Self {
        Self {
            name: "Python".into(),
            color: Some("#3776AB".into()), // Python blue
            keep_open: true,
            docker: Some(DockerTabConfig {
                mode: DockerMode::Run,
                image: Some("python:3.12".into()),
                shell: Some("/bin/bash".into()),
                auto_remove: true,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Create an SSH remote connection tab configuration
    pub fn ssh(name: &str, host: &str, username: Option<&str>) -> Self {
        Self {
            name: name.to_string(),
            color: Some("#22c55e".into()), // Green for remote connections
            keep_open: true,
            ssh: Some(SshTabConfig {
                host: host.to_string(),
                username: username.map(|s| s.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Create an SSH with agent forwarding tab configuration
    pub fn ssh_with_agent(name: &str, host: &str, username: Option<&str>) -> Self {
        Self {
            name: name.to_string(),
            color: Some("#22c55e".into()), // Green for remote connections
            keep_open: true,
            ssh: Some(SshTabConfig {
                host: host.to_string(),
                username: username.map(|s| s.to_string()),
                agent_forward: true,
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

/// Perform background git pull if config is a git repo.
/// Returns true if config was updated and should be reloaded.
pub fn background_sync() -> bool {
    let Some(dir) = config_dir() else {
        return false;
    };

    if !crate::git_sync::is_git_repo(&dir) {
        return false;
    }

    match crate::git_sync::pull(&dir) {
        Ok(true) => {
            log::info!("Config updated from git remote");
            true
        }
        Ok(false) => {
            log::debug!("Config already up to date with git remote");
            false
        }
        Err(e) => {
            log::warn!("Git pull failed: {}", e);
            false
        }
    }
}

/// Save configuration to file with optional git sync.
/// If sync is true and config dir is a git repo, commits and pushes changes.
pub fn save_config_with_sync(config: &Config, sync: bool) -> Result<(), ConfigError> {
    // Save to file first
    save_config(config)?;

    if sync {
        if let Some(dir) = config_dir() {
            if crate::git_sync::is_git_repo(&dir) {
                if let Err(e) = crate::git_sync::commit_and_push(&dir, "Update configuration") {
                    log::warn!("Failed to sync config to git: {}", e);
                }
            }
        }
    }

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

    #[test]
    fn test_parse_port_forwards_empty() {
        let forwards = SshPortForward::parse_list("");
        assert!(forwards.is_empty());
    }

    #[test]
    fn test_parse_port_forwards_simple() {
        let forwards = SshPortForward::parse_list("8080:80");
        assert_eq!(forwards.len(), 1);
        assert_eq!(forwards[0].local_port, 8080);
        assert_eq!(forwards[0].remote_host, "localhost");
        assert_eq!(forwards[0].remote_port, 80);
    }

    #[test]
    fn test_parse_port_forwards_with_host() {
        let forwards = SshPortForward::parse_list("5432:db.example.com:5432");
        assert_eq!(forwards.len(), 1);
        assert_eq!(forwards[0].local_port, 5432);
        assert_eq!(forwards[0].remote_host, "db.example.com");
        assert_eq!(forwards[0].remote_port, 5432);
    }

    #[test]
    fn test_parse_port_forwards_multiple() {
        let forwards = SshPortForward::parse_list("8080:80, 3000:localhost:3000");
        assert_eq!(forwards.len(), 2);
        assert_eq!(forwards[0].local_port, 8080);
        assert_eq!(forwards[1].local_port, 3000);
        assert_eq!(forwards[1].remote_host, "localhost");
    }

    #[test]
    fn test_parse_port_forwards_invalid() {
        // Invalid formats should be skipped
        let forwards = SshPortForward::parse_list("invalid,8080:80,too:many:parts:here");
        assert_eq!(forwards.len(), 1);
        assert_eq!(forwards[0].local_port, 8080);
    }

    // SSH command building tests
    #[test]
    fn test_ssh_build_command_basic() {
        let ssh = SshTabConfig {
            host: "example.com".to_string(),
            // Note: Default::default() sets request_tty to false,
            // but serde default is true (for TOML deserialization)
            ..Default::default()
        };
        let (cmd, args) = ssh.build_command();
        assert_eq!(cmd, "ssh");
        // request_tty is false with Default::default()
        assert!(!args.contains(&"-t".to_string()));
        assert!(args.contains(&"example.com".to_string()));
    }

    #[test]
    fn test_ssh_build_command_with_tty() {
        let ssh = SshTabConfig {
            host: "example.com".to_string(),
            request_tty: true,
            ..Default::default()
        };
        let (_, args) = ssh.build_command();
        assert!(args.contains(&"-t".to_string()));
    }

    #[test]
    fn test_ssh_build_command_with_username() {
        let ssh = SshTabConfig {
            host: "example.com".to_string(),
            username: Some("admin".to_string()),
            ..Default::default()
        };
        let (cmd, args) = ssh.build_command();
        assert_eq!(cmd, "ssh");
        assert!(args.contains(&"admin@example.com".to_string()));
    }

    #[test]
    fn test_ssh_build_command_with_port() {
        let ssh = SshTabConfig {
            host: "example.com".to_string(),
            port: Some(2222),
            ..Default::default()
        };
        let (_, args) = ssh.build_command();
        assert!(args.contains(&"-p".to_string()));
        assert!(args.contains(&"2222".to_string()));
    }

    #[test]
    fn test_ssh_build_command_default_port_not_added() {
        let ssh = SshTabConfig {
            host: "example.com".to_string(),
            port: Some(22),
            ..Default::default()
        };
        let (cmd, args) = ssh.build_command();
        assert_eq!(cmd, "ssh");
        // Port 22 should not be added explicitly
        assert!(!args.contains(&"-p".to_string()));
    }

    #[test]
    fn test_ssh_build_command_with_identity() {
        let ssh = SshTabConfig {
            host: "example.com".to_string(),
            identity_file: Some(PathBuf::from("/home/user/.ssh/id_rsa")),
            ..Default::default()
        };
        let (_, args) = ssh.build_command();
        assert!(args.contains(&"-i".to_string()));
        assert!(args.contains(&"/home/user/.ssh/id_rsa".to_string()));
    }

    #[test]
    fn test_ssh_build_command_with_local_forward() {
        let ssh = SshTabConfig {
            host: "example.com".to_string(),
            local_forwards: vec![SshPortForward {
                local_port: 8080,
                remote_host: "localhost".to_string(),
                remote_port: 80,
            }],
            ..Default::default()
        };
        let (_, args) = ssh.build_command();
        assert!(args.contains(&"-L".to_string()));
        assert!(args.contains(&"8080:localhost:80".to_string()));
    }

    #[test]
    fn test_ssh_build_command_with_remote_forward() {
        let ssh = SshTabConfig {
            host: "example.com".to_string(),
            remote_forwards: vec![SshPortForward {
                local_port: 3000,
                remote_host: "localhost".to_string(),
                remote_port: 3000,
            }],
            ..Default::default()
        };
        let (_, args) = ssh.build_command();
        assert!(args.contains(&"-R".to_string()));
        assert!(args.contains(&"3000:localhost:3000".to_string()));
    }

    #[test]
    fn test_ssh_build_command_with_dynamic_forward() {
        let ssh = SshTabConfig {
            host: "example.com".to_string(),
            dynamic_forward: Some(1080),
            ..Default::default()
        };
        let (_, args) = ssh.build_command();
        assert!(args.contains(&"-D".to_string()));
        assert!(args.contains(&"1080".to_string()));
    }

    #[test]
    fn test_ssh_build_command_with_x11() {
        let ssh = SshTabConfig {
            host: "example.com".to_string(),
            x11_forward: true,
            ..Default::default()
        };
        let (_, args) = ssh.build_command();
        assert!(args.contains(&"-X".to_string()));
    }

    #[test]
    fn test_ssh_build_command_with_agent_forward() {
        let ssh = SshTabConfig {
            host: "example.com".to_string(),
            agent_forward: true,
            ..Default::default()
        };
        let (_, args) = ssh.build_command();
        assert!(args.contains(&"-A".to_string()));
    }

    #[test]
    fn test_ssh_build_command_with_jump_host() {
        let ssh = SshTabConfig {
            host: "internal.example.com".to_string(),
            jump_host: Some("bastion.example.com".to_string()),
            ..Default::default()
        };
        let (_, args) = ssh.build_command();
        assert!(args.contains(&"-J".to_string()));
        assert!(args.contains(&"bastion.example.com".to_string()));
    }

    #[test]
    fn test_ssh_build_command_with_remote_command() {
        let ssh = SshTabConfig {
            host: "example.com".to_string(),
            remote_command: Some("ls -la".to_string()),
            ..Default::default()
        };
        let (_, args) = ssh.build_command();
        assert!(args.last() == Some(&"ls -la".to_string()));
    }

    #[test]
    fn test_ssh_build_command_no_tty() {
        let ssh = SshTabConfig {
            host: "example.com".to_string(),
            request_tty: false,
            ..Default::default()
        };
        let (_, args) = ssh.build_command();
        assert!(!args.contains(&"-t".to_string()));
    }

    // StickyTabConfig tests
    #[test]
    fn test_sticky_tab_docker_exec() {
        let tab = StickyTabConfig::docker_exec("My Container", "container-name");
        assert_eq!(tab.name, "My Container");
        assert!(tab.is_docker());
        assert!(!tab.is_ssh());
        let docker = tab.docker.unwrap();
        assert_eq!(docker.mode, DockerMode::Exec);
        assert_eq!(docker.container, Some("container-name".to_string()));
    }

    #[test]
    fn test_sticky_tab_docker_run() {
        let tab = StickyTabConfig::docker_run("Ubuntu Test", "ubuntu:22.04");
        assert_eq!(tab.name, "Ubuntu Test");
        assert!(tab.is_docker());
        let docker = tab.docker.unwrap();
        assert_eq!(docker.mode, DockerMode::Run);
        assert_eq!(docker.image, Some("ubuntu:22.04".to_string()));
        assert!(docker.auto_remove);
    }

    #[test]
    fn test_sticky_tab_ssh() {
        let tab = StickyTabConfig::ssh("Production", "prod.example.com", Some("deploy"));
        assert_eq!(tab.name, "Production");
        assert!(tab.is_ssh());
        assert!(!tab.is_docker());
        let ssh = tab.ssh.unwrap();
        assert_eq!(ssh.host, "prod.example.com");
        assert_eq!(ssh.username, Some("deploy".to_string()));
    }

    #[test]
    fn test_sticky_tab_ssh_with_agent() {
        let tab = StickyTabConfig::ssh_with_agent("Server", "server.example.com", None);
        let ssh = tab.ssh.unwrap();
        assert!(ssh.agent_forward);
    }

    #[test]
    fn test_get_command_args_regular() {
        let tab = StickyTabConfig {
            command: Some("/bin/bash".to_string()),
            args: vec!["-c".to_string(), "echo hello".to_string()],
            ..Default::default()
        };
        let (cmd, args) = tab.get_command_args();
        assert_eq!(cmd, Some("/bin/bash".to_string()));
        assert_eq!(args, vec!["-c", "echo hello"]);
    }

    #[test]
    fn test_get_command_args_default_shell() {
        let tab = StickyTabConfig::default();
        let (cmd, args) = tab.get_command_args();
        assert!(cmd.is_none());
        assert!(args.is_empty());
    }

    #[test]
    fn test_get_command_args_ssh() {
        let tab = StickyTabConfig::ssh("Test", "example.com", Some("user"));
        let (cmd, args) = tab.get_command_args();
        assert_eq!(cmd, Some("ssh".to_string()));
        assert!(args.contains(&"user@example.com".to_string()));
    }

    #[test]
    fn test_sticky_tab_claude_continue() {
        let tab = StickyTabConfig::claude_continue();
        assert_eq!(tab.name, "Claude (Continue)");
        assert_eq!(tab.command, Some("claude".to_string()));
        assert_eq!(tab.args, vec!["-c"]);
        assert!(tab.unique);
    }

    #[test]
    fn test_sticky_tab_ubuntu() {
        let tab = StickyTabConfig::ubuntu();
        assert_eq!(tab.name, "Ubuntu");
        let docker = tab.docker.unwrap();
        assert_eq!(docker.image, Some("ubuntu:latest".to_string()));
        assert_eq!(docker.shell, Some("/bin/bash".to_string()));
    }

    #[test]
    fn test_sticky_tab_alpine() {
        let tab = StickyTabConfig::alpine();
        assert_eq!(tab.name, "Alpine");
        let docker = tab.docker.unwrap();
        assert_eq!(docker.image, Some("alpine:latest".to_string()));
        assert_eq!(docker.shell, Some("/bin/sh".to_string()));
    }

    #[test]
    fn test_docker_mode_default() {
        let mode = DockerMode::default();
        assert_eq!(mode, DockerMode::Exec);
    }

    #[test]
    fn test_cursor_style_default() {
        let style = CursorStyleConfig::default();
        assert!(matches!(style, CursorStyleConfig::Block));
    }

    #[test]
    fn test_tab_bar_visibility_default() {
        let visibility = TabBarVisibility::default();
        assert!(matches!(visibility, TabBarVisibility::Always));
    }
}
