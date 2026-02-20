//! GitHub release update checker and downloader
//!
//! This module provides functionality to check for updates from GitHub releases,
//! download new versions, and verify their integrity.

use futures_util::StreamExt;
use semver::Version;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Errors that can occur during update operations
#[derive(Error, Debug)]
pub enum UpdateError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Failed to parse version: {0}")]
    Version(String),

    #[error("Failed to parse JSON: {0}")]
    Json(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Checksum verification failed")]
    ChecksumMismatch,

    #[error("No suitable release asset found for this platform")]
    NoAssetFound,

    #[error("GitHub API rate limit exceeded")]
    RateLimited,

    #[error("Release not found")]
    NotFound,
}

/// Information about an available update
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    /// Version string (e.g., "1.2.3")
    pub version: String,
    /// Parsed semantic version
    pub semver: Version,
    /// URL to download the binary
    pub download_url: String,
    /// URL to download the SHA256 checksum file (if available)
    pub checksum_url: Option<String>,
    /// Size of the download in bytes
    pub size: u64,
    /// Release notes / changelog
    pub release_notes: String,
    /// Release name/title
    pub name: String,
    /// Whether this is a prerelease
    pub prerelease: bool,
}

/// Update checker for GitHub releases
pub struct Updater {
    /// GitHub repository in "owner/repo" format
    repo: String,
    /// Current version of the application
    current_version: Version,
    /// HTTP client
    client: reqwest::Client,
}

impl Updater {
    /// Create a new updater
    ///
    /// # Arguments
    /// * `repo` - GitHub repository in "owner/repo" format
    /// * `current_version` - Current version string
    pub fn new(repo: &str, current_version: &str) -> Result<Self, UpdateError> {
        let version =
            Version::parse(current_version).map_err(|e| UpdateError::Version(e.to_string()))?;

        let client = reqwest::Client::builder()
            .user_agent(format!("cterm/{}", current_version))
            .build()?;

        Ok(Self {
            repo: repo.to_string(),
            current_version: version,
            client,
        })
    }

    /// Check for available updates
    ///
    /// Returns `Some(UpdateInfo)` if a newer version is available,
    /// `None` if already on the latest version.
    pub async fn check_for_update(&self) -> Result<Option<UpdateInfo>, UpdateError> {
        let url = format!("https://api.github.com/repos/{}/releases/latest", self.repo);

        let response = self
            .client
            .get(&url)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(UpdateError::NotFound);
        }

        if response.status() == reqwest::StatusCode::FORBIDDEN {
            // Check if it's rate limiting
            if response
                .headers()
                .get("X-RateLimit-Remaining")
                .and_then(|v| v.to_str().ok())
                .map(|v| v == "0")
                .unwrap_or(false)
            {
                return Err(UpdateError::RateLimited);
            }
        }

        let release: Value = response
            .json()
            .await
            .map_err(|e| UpdateError::Json(e.to_string()))?;

        self.parse_release(&release)
    }

    /// Parse a GitHub release response
    fn parse_release(&self, release: &Value) -> Result<Option<UpdateInfo>, UpdateError> {
        let tag_name = release["tag_name"]
            .as_str()
            .ok_or_else(|| UpdateError::Json("Missing tag_name".to_string()))?;

        // Strip 'v' prefix if present
        let version_str = tag_name.strip_prefix('v').unwrap_or(tag_name);

        let version =
            Version::parse(version_str).map_err(|e| UpdateError::Version(e.to_string()))?;

        // Check if this is newer than current
        if version <= self.current_version {
            return Ok(None);
        }

        // Find the appropriate asset for this platform
        let (download_url, size) = self.find_asset(release)?;

        // Look for checksum file
        let checksum_url = self.find_checksum_asset(release);

        let release_notes = release["body"].as_str().unwrap_or("").to_string();

        let name = release["name"].as_str().unwrap_or(tag_name).to_string();

        let prerelease = release["prerelease"].as_bool().unwrap_or(false);

        Ok(Some(UpdateInfo {
            version: version_str.to_string(),
            semver: version,
            download_url,
            checksum_url,
            size,
            release_notes,
            name,
            prerelease,
        }))
    }

    /// Find the appropriate release asset for the current platform
    fn find_asset(&self, release: &Value) -> Result<(String, u64), UpdateError> {
        let assets = release["assets"]
            .as_array()
            .ok_or(UpdateError::NoAssetFound)?;

        let target = Self::platform_target();

        for asset in assets {
            let name = asset["name"].as_str().unwrap_or("");

            // Look for asset matching our platform
            if name.contains(&target) && !name.ends_with(".sha256") {
                let url = asset["browser_download_url"]
                    .as_str()
                    .ok_or(UpdateError::NoAssetFound)?
                    .to_string();

                let size = asset["size"].as_u64().unwrap_or(0);

                return Ok((url, size));
            }
        }

        Err(UpdateError::NoAssetFound)
    }

    /// Find the checksum asset if available
    fn find_checksum_asset(&self, release: &Value) -> Option<String> {
        let assets = release["assets"].as_array()?;
        let target = Self::platform_target();

        for asset in assets {
            let name = asset["name"].as_str().unwrap_or("");

            // Look for checksum file matching our platform
            if name.contains(&target) && name.ends_with(".sha256") {
                return asset["browser_download_url"]
                    .as_str()
                    .map(|s| s.to_string());
            }
        }

        None
    }

    /// Get the platform target string used in release asset names
    fn platform_target() -> String {
        let os = if cfg!(target_os = "linux") {
            "linux"
        } else if cfg!(target_os = "macos") {
            "darwin"
        } else if cfg!(target_os = "windows") {
            "windows"
        } else {
            "unknown"
        };

        let arch = if cfg!(target_arch = "x86_64") {
            "x86_64"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else {
            "unknown"
        };

        format!("{}-{}", os, arch)
    }

    /// Download the update to a temporary file
    ///
    /// # Arguments
    /// * `info` - Update information from `check_for_update`
    /// * `on_progress` - Callback for progress updates (bytes_downloaded, total_bytes)
    ///
    /// # Returns
    /// Path to the downloaded file
    pub async fn download<F>(
        &self,
        info: &UpdateInfo,
        mut on_progress: F,
    ) -> Result<PathBuf, UpdateError>
    where
        F: FnMut(u64, u64),
    {
        let response = self.client.get(&info.download_url).send().await?;

        let total_size = response.content_length().unwrap_or(info.size);

        // Create temp file
        let temp_dir = std::env::temp_dir();
        let file_name = format!("cterm-update-{}", info.version);
        let file_path = temp_dir.join(&file_name);

        let mut file = std::fs::File::create(&file_path)?;
        let mut downloaded: u64 = 0;

        // Stream the download
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk)?;
            downloaded += chunk.len() as u64;
            on_progress(downloaded, total_size);
        }

        file.flush()?;

        // Make executable on Unix (for non-tar.gz downloads)
        #[cfg(unix)]
        if !file_path.to_string_lossy().ends_with(".tar.gz") {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&file_path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&file_path, perms)?;
        }

        Ok(file_path)
    }

    /// Extract a downloaded tar.gz archive
    ///
    /// # Arguments
    /// * `archive_path` - Path to the downloaded tar.gz file
    ///
    /// # Returns
    /// Path to the extracted directory
    pub fn extract_archive(archive_path: &Path) -> Result<PathBuf, UpdateError> {
        use flate2::read::GzDecoder;
        use std::fs::File;
        use tar::Archive;

        let file = File::open(archive_path)?;
        let decoder = GzDecoder::new(file);
        let mut archive = Archive::new(decoder);

        // Extract to a temp directory next to the archive
        let extract_dir = archive_path.with_extension("extracted");
        if extract_dir.exists() {
            std::fs::remove_dir_all(&extract_dir)?;
        }
        std::fs::create_dir_all(&extract_dir)?;

        archive.unpack(&extract_dir)?;

        Ok(extract_dir)
    }

    /// Install the update on macOS by replacing the app bundle
    ///
    /// # Arguments
    /// * `extracted_dir` - Path to the extracted update directory containing cterm.app
    ///
    /// # Returns
    /// Path to the installed app bundle's binary
    #[cfg(target_os = "macos")]
    pub fn install_macos_update(extracted_dir: &Path) -> Result<PathBuf, UpdateError> {
        let new_app = extracted_dir.join("cterm.app");
        if !new_app.exists() {
            return Err(UpdateError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "cterm.app not found in extracted archive",
            )));
        }

        // Get the current app bundle location
        let current_exe = std::env::current_exe()?;

        // Check if we're running from an app bundle
        // The path would be like: /Applications/cterm.app/Contents/MacOS/cterm
        let current_app = if let Some(macos_dir) = current_exe.parent() {
            if macos_dir.ends_with("MacOS") {
                if let Some(contents_dir) = macos_dir.parent() {
                    if contents_dir.ends_with("Contents") {
                        contents_dir.parent().map(|p| p.to_path_buf())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let target_app = match current_app {
            Some(app) => app,
            None => {
                // Not running from an app bundle, install to /Applications
                PathBuf::from("/Applications/cterm.app")
            }
        };

        log::info!(
            "Installing update: {} -> {}",
            new_app.display(),
            target_app.display()
        );

        // Remove old app bundle if it exists
        if target_app.exists() {
            // Move to trash or backup instead of deleting
            let backup_path = target_app.with_extension("app.backup");
            if backup_path.exists() {
                // Check that backup path is not a symlink to prevent symlink attacks
                let meta = std::fs::symlink_metadata(&backup_path)?;
                if meta.is_symlink() {
                    std::fs::remove_file(&backup_path)?;
                } else {
                    std::fs::remove_dir_all(&backup_path)?;
                }
            }
            std::fs::rename(&target_app, &backup_path)?;
        }

        // Move new app bundle into place
        std::fs::rename(&new_app, &target_app)?;

        // Return path to the new binary
        let new_binary = target_app.join("Contents/MacOS/cterm");
        Ok(new_binary)
    }

    /// Verify the downloaded file against its SHA256 checksum
    ///
    /// # Arguments
    /// * `file_path` - Path to the downloaded file
    /// * `info` - Update information containing checksum URL
    ///
    /// # Returns
    /// `Ok(true)` if verification passed, `Ok(false)` if no checksum available,
    /// `Err` on verification failure
    pub async fn verify(&self, file_path: &Path, info: &UpdateInfo) -> Result<bool, UpdateError> {
        let checksum_url = match &info.checksum_url {
            Some(url) => url,
            None => return Ok(false), // No checksum available
        };

        // Download checksum file
        let response = self.client.get(checksum_url).send().await?;

        let checksum_text = response.text().await?;

        // Parse expected checksum (format: "hash  filename" or just "hash")
        let expected_hash = checksum_text
            .split_whitespace()
            .next()
            .ok_or_else(|| UpdateError::Json("Invalid checksum format".to_string()))?
            .to_lowercase();

        // Calculate actual hash
        let file_data = std::fs::read(file_path)?;
        let mut hasher = Sha256::new();
        hasher.update(&file_data);
        let actual_hash = format!("{:x}", hasher.finalize());

        if actual_hash != expected_hash {
            return Err(UpdateError::ChecksumMismatch);
        }

        Ok(true)
    }

    /// Verify a file against an expected SHA256 hash
    pub fn verify_hash(file_path: &Path, expected_hash: &str) -> Result<bool, UpdateError> {
        let file_data = std::fs::read(file_path)?;
        let mut hasher = Sha256::new();
        hasher.update(&file_data);
        let actual_hash = format!("{:x}", hasher.finalize());

        Ok(actual_hash.to_lowercase() == expected_hash.to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_target() {
        let target = Updater::platform_target();
        assert!(target.contains('-'));

        #[cfg(target_os = "linux")]
        assert!(target.starts_with("linux"));

        #[cfg(target_os = "macos")]
        assert!(target.starts_with("darwin"));

        #[cfg(target_arch = "x86_64")]
        assert!(target.ends_with("x86_64"));

        #[cfg(target_arch = "aarch64")]
        assert!(target.ends_with("aarch64"));
    }

    #[test]
    fn test_version_comparison() {
        let v1 = Version::parse("1.0.0").unwrap();
        let v2 = Version::parse("1.0.1").unwrap();
        let v3 = Version::parse("2.0.0").unwrap();

        assert!(v2 > v1);
        assert!(v3 > v2);
        assert!(v3 > v1);
    }

    #[test]
    fn test_verify_hash() {
        use std::io::Write;

        // Create a temp file with known content
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join("test_hash_file");
        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(b"test content").unwrap();
        file.flush().unwrap();

        // Known SHA256 of "test content"
        let expected_hash = "6ae8a75555209fd6c44157c0aed8016e763ff435a19cf186f76863140143ff72";

        let result = Updater::verify_hash(&file_path, expected_hash).unwrap();
        assert!(result);

        // Wrong hash should return false
        let result = Updater::verify_hash(&file_path, "wronghash").unwrap();
        assert!(!result);

        // Cleanup
        std::fs::remove_file(&file_path).unwrap();
    }
}
