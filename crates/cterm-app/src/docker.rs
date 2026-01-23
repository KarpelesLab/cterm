//! Docker utility functions for container/image management

use std::fmt;
use std::path::Path;
use std::process::Command;

use serde::Deserialize;

/// Error type for Docker operations
#[derive(Debug)]
pub enum DockerError {
    /// Docker binary not found
    NotInstalled,
    /// Docker daemon is not running
    DaemonNotRunning,
    /// Docker command failed with error message
    CommandFailed(String),
    /// Failed to parse Docker output
    ParseError(String),
}

impl fmt::Display for DockerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DockerError::NotInstalled => write!(f, "Docker is not installed"),
            DockerError::DaemonNotRunning => write!(f, "Docker daemon is not running"),
            DockerError::CommandFailed(msg) => write!(f, "Docker command failed: {}", msg),
            DockerError::ParseError(msg) => write!(f, "Failed to parse Docker output: {}", msg),
        }
    }
}

impl std::error::Error for DockerError {}

/// Information about a running container from `docker ps`
#[derive(Debug, Clone)]
pub struct ContainerInfo {
    /// Container ID (short form)
    pub id: String,
    /// Container name
    pub name: String,
    /// Image used to create the container
    pub image: String,
    /// Container status (e.g., "Up 2 hours")
    pub status: String,
}

/// Information about a Docker image from `docker images`
#[derive(Debug, Clone)]
pub struct ImageInfo {
    /// Image ID (short form)
    pub id: String,
    /// Repository name
    pub repository: String,
    /// Image tag
    pub tag: String,
    /// Human-readable size (e.g., "128MB")
    pub size: String,
}

/// Check if Docker is available and the daemon is running
pub fn check_docker_available() -> Result<(), DockerError> {
    let output = Command::new("docker")
        .arg("version")
        .output()
        .map_err(|_| DockerError::NotInstalled)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("Cannot connect")
            || stderr.contains("daemon")
            || stderr.contains("Is the docker daemon running")
        {
            return Err(DockerError::DaemonNotRunning);
        }
        return Err(DockerError::CommandFailed(stderr.to_string()));
    }

    Ok(())
}

/// List running containers
pub fn list_containers() -> Result<Vec<ContainerInfo>, DockerError> {
    let output = Command::new("docker")
        .args([
            "ps",
            "--format",
            "{{.ID}}\t{{.Names}}\t{{.Image}}\t{{.Status}}",
        ])
        .output()
        .map_err(|e| DockerError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        return Err(DockerError::CommandFailed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let containers = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 4 {
                Some(ContainerInfo {
                    id: parts[0].to_string(),
                    name: parts[1].to_string(),
                    image: parts[2].to_string(),
                    status: parts[3].to_string(),
                })
            } else {
                None
            }
        })
        .collect();

    Ok(containers)
}

/// List available Docker images
pub fn list_images() -> Result<Vec<ImageInfo>, DockerError> {
    let output = Command::new("docker")
        .args([
            "images",
            "--format",
            "{{.ID}}\t{{.Repository}}\t{{.Tag}}\t{{.Size}}",
        ])
        .output()
        .map_err(|e| DockerError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        return Err(DockerError::CommandFailed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let images = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 4 && parts[1] != "<none>" {
                Some(ImageInfo {
                    id: parts[0].to_string(),
                    repository: parts[1].to_string(),
                    tag: parts[2].to_string(),
                    size: parts[3].to_string(),
                })
            } else {
                None
            }
        })
        .collect();

    Ok(images)
}

/// Build command and arguments for `docker exec`
///
/// Returns (command, args) tuple suitable for PtyConfig
pub fn build_exec_command(container: &str, shell: Option<&str>) -> (String, Vec<String>) {
    let shell = shell.unwrap_or("/bin/sh");
    (
        "docker".to_string(),
        vec![
            "exec".to_string(),
            "-it".to_string(),
            container.to_string(),
            shell.to_string(),
        ],
    )
}

/// Build command and arguments for `docker run`
///
/// Returns (command, args) tuple suitable for PtyConfig
pub fn build_run_command(
    image: &str,
    shell: Option<&str>,
    auto_remove: bool,
    extra_args: &[String],
) -> (String, Vec<String>) {
    let shell = shell.unwrap_or("/bin/sh");
    let mut args = vec!["run".to_string(), "-it".to_string()];

    if auto_remove {
        args.push("--rm".to_string());
    }

    args.extend(extra_args.iter().cloned());
    args.push(image.to_string());
    args.push(shell.to_string());

    ("docker".to_string(), args)
}

/// Devcontainer.json build configuration
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DevcontainerBuild {
    /// Dockerfile to build (relative to .devcontainer)
    pub dockerfile: Option<String>,
    /// Build context (relative to .devcontainer)
    pub context: Option<String>,
    /// Build arguments
    #[serde(default)]
    pub args: std::collections::HashMap<String, String>,
}

/// Devcontainer.json configuration (subset of fields we support)
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DevcontainerConfig {
    /// Container name
    pub name: Option<String>,
    /// Docker image to use
    pub image: Option<String>,
    /// Build configuration (alternative to image)
    pub build: Option<DevcontainerBuild>,
    /// Working directory inside container
    pub workspace_folder: Option<String>,
    /// Workspace mount specification
    pub workspace_mount: Option<String>,
    /// Container user
    pub container_user: Option<String>,
    /// Remote user (user to run as)
    pub remote_user: Option<String>,
    /// Mounts to add
    #[serde(default)]
    pub mounts: Vec<String>,
    /// Docker run arguments
    #[serde(default)]
    pub run_args: Vec<String>,
    /// Environment variables
    #[serde(default)]
    pub container_env: std::collections::HashMap<String, String>,
    /// Features to install
    #[serde(default)]
    pub features: serde_json::Value,
    /// Post-create command
    pub post_create_command: Option<serde_json::Value>,
    /// Post-start command
    pub post_start_command: Option<serde_json::Value>,
}

/// Load devcontainer.json from a project directory
pub fn load_devcontainer_config(project_dir: &Path) -> Option<DevcontainerConfig> {
    // Try .devcontainer/devcontainer.json first
    let devcontainer_path = project_dir.join(".devcontainer/devcontainer.json");
    if devcontainer_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&devcontainer_path) {
            // Strip JSON comments (// and /* */) before parsing
            let content = strip_json_comments(&content);
            if let Ok(config) = serde_json::from_str(&content) {
                log::info!("Loaded devcontainer.json from {:?}", devcontainer_path);
                return Some(config);
            } else {
                log::warn!(
                    "Failed to parse devcontainer.json at {:?}",
                    devcontainer_path
                );
            }
        }
    }

    // Try .devcontainer.json in project root
    let devcontainer_path = project_dir.join(".devcontainer.json");
    if devcontainer_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&devcontainer_path) {
            let content = strip_json_comments(&content);
            if let Ok(config) = serde_json::from_str(&content) {
                log::info!("Loaded .devcontainer.json from {:?}", devcontainer_path);
                return Some(config);
            }
        }
    }

    None
}

/// Strip JSON comments (// and /* */) from content
fn strip_json_comments(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut chars = content.chars().peekable();
    let mut in_string = false;
    let mut escape_next = false;

    while let Some(c) = chars.next() {
        if escape_next {
            result.push(c);
            escape_next = false;
            continue;
        }

        if c == '\\' && in_string {
            result.push(c);
            escape_next = true;
            continue;
        }

        if c == '"' {
            in_string = !in_string;
            result.push(c);
            continue;
        }

        if !in_string && c == '/' {
            if let Some(&next) = chars.peek() {
                if next == '/' {
                    // Line comment - skip until newline
                    chars.next();
                    while let Some(&ch) = chars.peek() {
                        if ch == '\n' {
                            break;
                        }
                        chars.next();
                    }
                    continue;
                } else if next == '*' {
                    // Block comment - skip until */
                    chars.next();
                    while let Some(ch) = chars.next() {
                        if ch == '*' {
                            if let Some(&'/') = chars.peek() {
                                chars.next();
                                break;
                            }
                        }
                    }
                    continue;
                }
            }
        }

        result.push(c);
    }

    result
}

/// Generate a unique container ID for devcontainer variable substitution
fn generate_devcontainer_id(project_dir: &Path) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    project_dir.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

/// Expand devcontainer variables in a string
fn expand_devcontainer_vars(s: &str, project_dir: &Path, container_id: &str) -> String {
    s.replace("${localWorkspaceFolder}", &project_dir.to_string_lossy())
        .replace("${devcontainerId}", container_id)
        // Remove unsupported ${localEnv:...} variables (use empty string)
        .split("${localEnv:")
        .enumerate()
        .map(|(i, part)| {
            if i == 0 {
                part.to_string()
            } else if let Some(end) = part.find('}') {
                // Extract default value if present (e.g., ${localEnv:VAR:default})
                let var_spec = &part[..end];
                let default = var_spec.split(':').nth(1).unwrap_or("");
                format!("{}{}", default, &part[end + 1..])
            } else {
                part.to_string()
            }
        })
        .collect()
}

/// Build command and arguments for a devcontainer-style `docker run`
///
/// This reads .devcontainer/devcontainer.json if present, otherwise uses defaults.
/// The devcontainer.json can specify:
/// - image: Docker image to use
/// - build.dockerfile: Dockerfile to build
/// - workspaceFolder: Working directory inside container
/// - workspaceMount: How to mount the workspace
/// - mounts: Additional mounts
/// - runArgs: Additional docker run arguments
/// - containerEnv: Environment variables
///
/// Returns (command, args) tuple suitable for PtyConfig
pub fn build_devcontainer_command(
    config: &crate::config::DockerTabConfig,
) -> (String, Vec<String>) {
    // Get project directory
    let project_dir = config
        .project_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    // Try to load devcontainer.json
    let devcontainer = load_devcontainer_config(&project_dir);

    // Generate a unique container ID for this project
    let container_id = generate_devcontainer_id(&project_dir);

    // Check if we need to build an image first
    let needs_build = devcontainer
        .as_ref()
        .map(|d| d.build.is_some() && d.image.is_none())
        .unwrap_or(false);

    if needs_build {
        // Build the image first
        if let Some(ref dc) = devcontainer {
            if let Some(ref build) = dc.build {
                let image_tag = format!("devcontainer-{}", container_id);
                let dockerfile = build.dockerfile.as_deref().unwrap_or("Dockerfile");
                let context = build.context.as_deref().unwrap_or(".");
                let devcontainer_dir = project_dir.join(".devcontainer");

                log::info!("Building devcontainer image: {}", image_tag);

                let mut build_cmd = Command::new("docker");
                build_cmd
                    .arg("build")
                    .arg("-t")
                    .arg(&image_tag)
                    .arg("-f")
                    .arg(devcontainer_dir.join(dockerfile));

                // Add build args
                for (key, value) in &build.args {
                    let expanded = expand_devcontainer_vars(value, &project_dir, &container_id);
                    build_cmd
                        .arg("--build-arg")
                        .arg(format!("{}={}", key, expanded));
                }

                build_cmd.arg(devcontainer_dir.join(context));

                match build_cmd.status() {
                    Ok(status) if status.success() => {
                        log::info!("Successfully built devcontainer image: {}", image_tag);
                    }
                    Ok(status) => {
                        log::error!(
                            "Failed to build devcontainer image, exit code: {:?}",
                            status.code()
                        );
                    }
                    Err(e) => {
                        log::error!("Failed to run docker build: {}", e);
                    }
                }

                // Return command using the built image
                return build_run_command_internal(
                    config,
                    &devcontainer,
                    &project_dir,
                    &container_id,
                    &image_tag,
                );
            }
        }
    }

    // Determine image from devcontainer.json or fallback to config/defaults
    let image = config
        .image
        .as_deref()
        .or(devcontainer.as_ref().and_then(|d| d.image.as_deref()))
        .unwrap_or("node:20");

    build_run_command_internal(config, &devcontainer, &project_dir, &container_id, image)
}

/// Internal helper to build the docker run command
fn build_run_command_internal(
    config: &crate::config::DockerTabConfig,
    devcontainer: &Option<DevcontainerConfig>,
    project_dir: &Path,
    container_id: &str,
    image: &str,
) -> (String, Vec<String>) {
    let shell = config.shell.as_deref().unwrap_or("/bin/bash");

    let workdir = config
        .workdir
        .as_deref()
        .or(devcontainer
            .as_ref()
            .and_then(|d| d.workspace_folder.as_deref()))
        .unwrap_or("/workspace");

    let remote_user = devcontainer
        .as_ref()
        .and_then(|d| d.remote_user.as_deref().or(d.container_user.as_deref()));

    let mut args = vec!["run".to_string(), "-it".to_string()];

    if config.auto_remove {
        args.push("--rm".to_string());
    }

    // Set container name if specified (allows reconnecting)
    if let Some(ref name) = config.container_name {
        args.push("--name".to_string());
        args.push(name.clone());
    }

    // Handle workspace mount - use workspaceMount if specified, otherwise default
    let has_workspace_mount = devcontainer
        .as_ref()
        .map(|d| d.workspace_mount.is_some())
        .unwrap_or(false);

    if let Some(ref dc) = devcontainer {
        if let Some(ref workspace_mount) = dc.workspace_mount {
            let mount = expand_devcontainer_vars(workspace_mount, project_dir, container_id);
            args.push("--mount".to_string());
            args.push(mount);
        }
    }

    // If no workspaceMount specified, add default project directory mount
    if !has_workspace_mount && project_dir.exists() {
        args.push("-v".to_string());
        args.push(format!("{}:{}:delegated", project_dir.display(), workdir));
    }

    // Add mounts from devcontainer.json
    if let Some(ref dc) = devcontainer {
        for mount in &dc.mounts {
            let mount = expand_devcontainer_vars(mount, project_dir, container_id);
            args.push("--mount".to_string());
            args.push(mount);
        }

        // Add run args from devcontainer.json
        for arg in &dc.run_args {
            let expanded = expand_devcontainer_vars(arg, project_dir, container_id);
            args.push(expanded);
        }

        // Add environment variables
        for (key, value) in &dc.container_env {
            let expanded = expand_devcontainer_vars(value, project_dir, container_id);
            args.push("-e".to_string());
            args.push(format!("{}={}", key, expanded));
        }
    }

    // Set user if specified
    if let Some(user) = remote_user {
        args.push("-u".to_string());
        args.push(user.to_string());
    }

    // Set working directory inside container
    args.push("-w".to_string());
    args.push(workdir.to_string());

    // Add any extra docker args from config
    args.extend(config.docker_args.iter().cloned());

    // Add the image
    args.push(image.to_string());

    // Determine post-start command (runs every time container starts)
    let post_start = devcontainer
        .as_ref()
        .and_then(|d| d.post_start_command.as_ref())
        .and_then(json_value_to_command);

    // Determine post-create command (runs once on first creation)
    let post_create = devcontainer
        .as_ref()
        .and_then(|d| d.post_create_command.as_ref())
        .and_then(json_value_to_command);

    // Build the shell command
    let startup_cmd = match (post_start, post_create) {
        (Some(start), Some(create)) => Some(format!("{}; {}", create, start)),
        (Some(start), None) => Some(start),
        (None, Some(create)) => Some(create),
        (None, None) => None,
    };

    if let Some(cmd) = startup_cmd {
        args.push(shell.to_string());
        args.push("-c".to_string());
        args.push(format!("{}; exec {}", cmd, shell));
    } else if image.starts_with("node:") && devcontainer.is_none() {
        // For node images without devcontainer.json, install claude-code
        args.push(shell.to_string());
        args.push("-c".to_string());
        args.push(format!(
            "which claude >/dev/null 2>&1 || npm install -g @anthropic-ai/claude-code; exec {}",
            shell
        ));
    } else {
        args.push(shell.to_string());
    }

    ("docker".to_string(), args)
}

/// Convert a JSON value (string or array) to a command string
fn json_value_to_command(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(arr) => {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(" "))
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_exec_command_default_shell() {
        let (cmd, args) = build_exec_command("my-container", None);
        assert_eq!(cmd, "docker");
        assert_eq!(args, vec!["exec", "-it", "my-container", "/bin/sh"]);
    }

    #[test]
    fn test_build_exec_command_custom_shell() {
        let (cmd, args) = build_exec_command("my-container", Some("/bin/bash"));
        assert_eq!(cmd, "docker");
        assert_eq!(args, vec!["exec", "-it", "my-container", "/bin/bash"]);
    }

    #[test]
    fn test_build_run_command_default() {
        let (cmd, args) = build_run_command("ubuntu:latest", None, true, &[]);
        assert_eq!(cmd, "docker");
        assert_eq!(args, vec!["run", "-it", "--rm", "ubuntu:latest", "/bin/sh"]);
    }

    #[test]
    fn test_build_run_command_no_auto_remove() {
        let (cmd, args) = build_run_command("ubuntu:latest", Some("/bin/bash"), false, &[]);
        assert_eq!(cmd, "docker");
        assert_eq!(args, vec!["run", "-it", "ubuntu:latest", "/bin/bash"]);
    }

    #[test]
    fn test_build_run_command_with_extra_args() {
        let extra = vec!["-v".to_string(), "/host:/container".to_string()];
        let (cmd, args) = build_run_command("ubuntu:latest", None, true, &extra);
        assert_eq!(cmd, "docker");
        assert_eq!(
            args,
            vec![
                "run",
                "-it",
                "--rm",
                "-v",
                "/host:/container",
                "ubuntu:latest",
                "/bin/sh"
            ]
        );
    }
}
