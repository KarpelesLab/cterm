//! SSH host key generation and management.

use cterm_app::config::LatchConfig;
use russh::keys::PrivateKey;
use std::path::PathBuf;

/// Resolve the host key path from config (or use default).
fn host_key_path(config: &LatchConfig) -> PathBuf {
    if let Some(ref p) = config.host_key_path {
        PathBuf::from(p)
    } else {
        crate::cli::config_dir().join("host_key")
    }
}

/// Load an existing host key or generate a new Ed25519 key.
pub fn load_or_generate_host_key(config: &LatchConfig) -> anyhow::Result<PrivateKey> {
    let path = host_key_path(config);

    if path.exists() {
        let key = PrivateKey::read_openssh_file(&path)
            .map_err(|e| anyhow::anyhow!("Failed to load host key {}: {}", path.display(), e))?;
        log::info!("Loaded SSH host key from {}", path.display());
        return Ok(key);
    }

    // Generate new Ed25519 key
    let key = PrivateKey::random(
        &mut ssh_key::rand_core::OsRng,
        russh::keys::Algorithm::Ed25519,
    )
    .map_err(|e| anyhow::anyhow!("Failed to generate host key: {}", e))?;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Write key file
    key.write_openssh_file(&path, ssh_key::LineEnding::LF)
        .map_err(|e| anyhow::anyhow!("Failed to write host key to {}: {}", path.display(), e))?;

    // Set permissions to 0600
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }

    log::info!("Generated new SSH host key at {}", path.display());
    Ok(key)
}
