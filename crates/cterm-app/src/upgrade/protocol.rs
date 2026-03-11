//! Upgrade protocol - handles sending and receiving upgrade state
//!
//! Since all terminal sessions live in the ctermd daemon, upgrading cterm
//! only requires preserving UI layout state (window positions, tab arrangement,
//! session IDs). The daemon keeps sessions alive across cterm restarts.
//!
//! Protocol: serialize state as JSON to a temp file, spawn new binary
//! with `--upgrade-state /path/to/file`, exit. New process reads the file,
//! connects to ctermd, and reconstructs windows.

use std::io;
use std::path::{Path, PathBuf};

use super::state::UpgradeState;

/// Errors that can occur during upgrade
#[derive(Debug, thiserror::Error)]
pub enum UpgradeError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),

    #[error("Process spawn error: {0}")]
    Spawn(String),
}

/// Get the path for the upgrade state file
fn upgrade_state_path() -> PathBuf {
    let mut path = std::env::temp_dir();
    #[cfg(unix)]
    {
        let uid = unsafe { libc::getuid() };
        path.push(format!("cterm_upgrade_{}.json", uid));
    }
    #[cfg(not(unix))]
    {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".to_string());
        path.push(format!("cterm_upgrade_{}.json", user));
    }
    path
}

/// Execute an upgrade by saving state and spawning the new process.
///
/// 1. Serializes upgrade state (window layout + session IDs) to a temp file
/// 2. Spawns the new binary with `--upgrade-state /path/to/file`
/// 3. Returns Ok(()) — caller should exit after this
pub fn execute_upgrade(new_binary: &Path, state: &UpgradeState) -> Result<(), UpgradeError> {
    let state_path = upgrade_state_path();

    // Serialize state as JSON
    let json =
        serde_json::to_vec_pretty(state).map_err(|e| UpgradeError::Serialization(e.to_string()))?;

    log::info!(
        "Saving upgrade state ({} bytes) to {}",
        json.len(),
        state_path.display()
    );

    // Write to temp file
    std::fs::write(&state_path, &json)?;

    // Set restrictive permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&state_path, std::fs::Permissions::from_mode(0o600)).ok();
    }

    // Spawn new process
    std::process::Command::new(new_binary)
        .arg("--upgrade-state")
        .arg(&state_path)
        .spawn()
        .map_err(|e| UpgradeError::Spawn(e.to_string()))?;

    log::info!("New process spawned, upgrade state written");

    Ok(())
}

/// Receive upgrade state from a file (new process side).
///
/// Reads the state file, deserializes it, and deletes the file.
/// The caller should then connect to ctermd and reconstruct windows
/// using the session IDs in the state.
pub fn receive_upgrade(state_path: &Path) -> Result<UpgradeState, UpgradeError> {
    log::info!("Reading upgrade state from {}", state_path.display());

    let json = std::fs::read_to_string(state_path)?;

    let state: UpgradeState =
        serde_json::from_str(&json).map_err(|e| UpgradeError::Deserialization(e.to_string()))?;

    // Clean up state file
    if let Err(e) = std::fs::remove_file(state_path) {
        log::warn!("Failed to remove upgrade state file: {}", e);
    }

    log::info!(
        "Upgrade state loaded: format_version={}, {} window(s)",
        state.format_version,
        state.windows.len()
    );

    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upgrade::state::*;

    #[test]
    fn test_state_serialization_roundtrip() {
        let mut state = UpgradeState::new();

        let mut window = WindowUpgradeState::new();
        window.width = 1024;
        window.height = 768;

        let mut tab = TabUpgradeState::new(1);
        tab.title = "bash".to_string();
        tab.session_id = Some("sess-123".to_string());
        window.tabs.push(tab);

        state.windows.push(window);

        let json = serde_json::to_vec(&state).expect("Serialize failed");
        let restored: UpgradeState = serde_json::from_slice(&json).expect("Deserialize failed");

        assert_eq!(restored.windows.len(), 1);
        assert_eq!(restored.windows[0].width, 1024);
        assert_eq!(
            restored.windows[0].tabs[0].session_id.as_deref(),
            Some("sess-123")
        );
    }

    #[test]
    fn test_upgrade_state_file_roundtrip() {
        let mut state = UpgradeState::new();
        let mut window = WindowUpgradeState::new();
        window.x = 100;
        window.y = 200;
        state.windows.push(window);

        // Write to temp file
        let path = std::env::temp_dir().join("cterm_test_upgrade.json");
        let json = serde_json::to_vec(&state).unwrap();
        std::fs::write(&path, &json).unwrap();

        // Read back
        let restored = receive_upgrade(&path).unwrap();
        assert_eq!(restored.windows[0].x, 100);
        assert_eq!(restored.windows[0].y, 200);

        // File should be deleted
        assert!(!path.exists());
    }
}
