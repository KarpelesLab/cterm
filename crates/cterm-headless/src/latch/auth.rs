//! Authorized SSH keys management.

use cterm_app::config::LatchConfig;
use russh::keys::PublicKey;
use std::path::PathBuf;

/// A set of authorized SSH public keys.
pub struct AuthorizedKeys {
    keys: Vec<PublicKey>,
    path: PathBuf,
}

impl AuthorizedKeys {
    /// Load authorized keys from the configured path.
    ///
    /// If the file does not exist, creates an empty file and returns
    /// an empty key set (all connections will be rejected).
    pub fn load(config: &LatchConfig) -> anyhow::Result<Self> {
        let path = if let Some(ref p) = config.authorized_keys_path {
            PathBuf::from(p)
        } else {
            crate::cli::config_dir().join("authorized_keys")
        };

        let keys = if path.exists() {
            parse_authorized_keys_file(&path)?
        } else {
            // Create empty file
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, "")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            }
            log::warn!(
                "No authorized_keys file found, created empty file at {}. \
                 Add SSH public keys to allow remote connections.",
                path.display()
            );
            Vec::new()
        };

        log::info!(
            "Loaded {} authorized key(s) from {}",
            keys.len(),
            path.display()
        );

        Ok(Self { keys, path })
    }

    /// Check if a public key is authorized.
    pub fn contains(&self, key: &PublicKey) -> bool {
        self.keys.iter().any(|k| k == key)
    }

    /// Reload keys from disk. Called on each connection attempt.
    pub fn reload(&mut self) -> anyhow::Result<()> {
        if self.path.exists() {
            self.keys = parse_authorized_keys_file(&self.path)?;
        }
        Ok(())
    }
}

/// Parse an OpenSSH authorized_keys file into public keys.
fn parse_authorized_keys_file(path: &std::path::Path) -> anyhow::Result<Vec<PublicKey>> {
    let content = std::fs::read_to_string(path)?;
    let mut keys = Vec::new();

    for (line_num, line) in content.lines().enumerate() {
        let line = line.trim();
        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Parse the key line. Format: [options] key-type base64-key [comment]
        // We need to find the key-type + base64 part.
        match parse_key_line(line) {
            Ok(key) => keys.push(key),
            Err(e) => {
                log::warn!(
                    "{}:{}: failed to parse key: {}",
                    path.display(),
                    line_num + 1,
                    e
                );
            }
        }
    }

    Ok(keys)
}

/// Parse a single authorized_keys line into a PublicKey.
fn parse_key_line(line: &str) -> anyhow::Result<PublicKey> {
    let parts: Vec<&str> = line.split_whitespace().collect();

    // Try to find the key type + base64 data
    for (i, part) in parts.iter().enumerate() {
        if is_key_type(part) {
            if let Some(b64) = parts.get(i + 1) {
                let key_str = format!("{} {}", part, b64);
                let key = PublicKey::from_openssh(&key_str)
                    .map_err(|e| anyhow::anyhow!("invalid key: {}", e))?;
                return Ok(key);
            }
        }
    }

    // If no recognized key type, try parsing the whole line
    let key =
        PublicKey::from_openssh(line).map_err(|e| anyhow::anyhow!("invalid key line: {}", e))?;
    Ok(key)
}

/// Check if a string is a recognized SSH key type.
fn is_key_type(s: &str) -> bool {
    matches!(
        s,
        "ssh-ed25519"
            | "ssh-rsa"
            | "ecdsa-sha2-nistp256"
            | "ecdsa-sha2-nistp384"
            | "ecdsa-sha2-nistp521"
            | "ssh-dss"
            | "sk-ssh-ed25519@openssh.com"
            | "sk-ecdsa-sha2-nistp256@openssh.com"
    )
}
