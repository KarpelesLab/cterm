//! File transfer manager for iTerm2 protocol
//!
//! Manages pending file transfers received via OSC 1337 with inline=0.

use std::path::PathBuf;

/// A pending file waiting for user action
#[derive(Debug)]
pub struct PendingFile {
    /// Unique ID for this transfer
    pub id: u64,
    /// Filename (if provided)
    pub name: Option<String>,
    /// File data
    pub data: Vec<u8>,
}

/// Manages pending file transfers
#[derive(Debug, Default)]
pub struct PendingFileManager {
    /// Currently pending file (only one at a time)
    pending: Option<PendingFile>,
    /// Last used save directory
    last_save_dir: Option<PathBuf>,
}

impl PendingFileManager {
    /// Create a new file manager
    pub fn new() -> Self {
        Self {
            pending: None,
            last_save_dir: None,
        }
    }

    /// Set a new pending file (discards any existing pending file)
    pub fn set_pending(&mut self, id: u64, name: Option<String>, data: Vec<u8>) {
        if self.pending.is_some() {
            log::debug!("Discarding previous pending file");
        }
        self.pending = Some(PendingFile { id, name, data });
    }

    /// Get the current pending file (if any)
    pub fn pending(&self) -> Option<&PendingFile> {
        self.pending.as_ref()
    }

    /// Take the pending file with the given ID
    pub fn take_pending(&mut self, id: u64) -> Option<PendingFile> {
        if self.pending.as_ref().is_some_and(|p| p.id == id) {
            self.pending.take()
        } else {
            None
        }
    }

    /// Discard the pending file with the given ID
    pub fn discard(&mut self, id: u64) {
        if self.pending.as_ref().is_some_and(|p| p.id == id) {
            self.pending = None;
        }
    }

    /// Check if there's a pending file
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// Get the last used save directory
    pub fn last_save_dir(&self) -> Option<&PathBuf> {
        self.last_save_dir.as_ref()
    }

    /// Set the last used save directory
    pub fn set_last_save_dir(&mut self, dir: PathBuf) {
        self.last_save_dir = Some(dir);
    }

    /// Get the suggested filename for a pending file
    pub fn suggested_filename(&self) -> Option<&str> {
        self.pending.as_ref().and_then(|p| p.name.as_deref())
    }

    /// Get the default save path for the current pending file
    pub fn default_save_path(&self) -> Option<PathBuf> {
        let file = self.pending.as_ref()?;
        let name = file.name.as_deref().unwrap_or("download");

        // Use last save dir if available, otherwise Downloads folder
        let dir = self.last_save_dir.clone().or_else(|| {
            dirs::download_dir().or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        })?;

        Some(dir.join(name))
    }

    /// Save the pending file to the given path
    pub fn save_to_path(&mut self, id: u64, path: &std::path::Path) -> std::io::Result<usize> {
        let file = self
            .take_pending(id)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "No pending file"))?;

        // Update last save directory
        if let Some(parent) = path.parent() {
            self.last_save_dir = Some(parent.to_path_buf());
        }

        let size = file.data.len();
        std::fs::write(path, &file.data)?;
        log::info!("Saved file to {:?} ({} bytes)", path, size);
        Ok(size)
    }
}

/// Helper module for common directories
mod dirs {
    use std::path::PathBuf;

    pub fn home_dir() -> Option<PathBuf> {
        std::env::var_os("HOME").map(PathBuf::from)
    }

    pub fn download_dir() -> Option<PathBuf> {
        home_dir().map(|h| h.join("Downloads"))
    }
}
