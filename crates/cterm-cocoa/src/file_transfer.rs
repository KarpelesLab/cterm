//! File transfer manager for iTerm2 protocol
//!
//! Manages pending file transfers received via OSC 1337 with inline=0.
//! Supports both small in-memory files and large streaming files stored on disk.

use cterm_core::StreamingFileData;
use std::path::PathBuf;

/// Where the file data is stored
#[derive(Debug)]
pub enum FileData {
    /// Data is in memory
    Memory(Vec<u8>),
    /// Data is in a temp file (for large streaming transfers)
    TempFile {
        path: PathBuf,
        size: usize,
    },
}

impl FileData {
    /// Get the size of the data
    pub fn size(&self) -> usize {
        match self {
            FileData::Memory(data) => data.len(),
            FileData::TempFile { size, .. } => *size,
        }
    }

    /// Check if data is stored on disk
    pub fn is_on_disk(&self) -> bool {
        matches!(self, FileData::TempFile { .. })
    }
}

impl From<StreamingFileData> for FileData {
    fn from(data: StreamingFileData) -> Self {
        match data {
            StreamingFileData::Memory(bytes) => FileData::Memory(bytes),
            StreamingFileData::TempFile { path, size } => {
                // Note: We take ownership of the path, the StreamingFileData's Drop
                // will not delete it because we moved the path out
                FileData::TempFile { path, size }
            }
        }
    }
}

/// A pending file waiting for user action
#[derive(Debug)]
pub struct PendingFile {
    /// Unique ID for this transfer
    pub id: u64,
    /// Filename (if provided)
    pub name: Option<String>,
    /// File data (memory or temp file)
    pub data: FileData,
}

impl PendingFile {
    /// Get the size of the file
    pub fn size(&self) -> usize {
        self.data.size()
    }
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
        self.pending = Some(PendingFile {
            id,
            name,
            data: FileData::Memory(data),
        });
    }

    /// Set a new pending file from streaming data (discards any existing pending file)
    pub fn set_pending_streaming(
        &mut self,
        id: u64,
        name: Option<String>,
        streaming_data: StreamingFileData,
    ) {
        if self.pending.is_some() {
            log::debug!("Discarding previous pending file");
        }

        // Handle the temp file path - we need to prevent StreamingFileData's Drop from
        // deleting the temp file. We do this by extracting the path before the data
        // is dropped.
        let data = match streaming_data {
            StreamingFileData::Memory(bytes) => FileData::Memory(bytes),
            StreamingFileData::TempFile { path, size } => {
                // Transfer ownership of the temp file path
                // The original StreamingFileData's Drop won't run because we moved the data
                FileData::TempFile { path, size }
            }
        };

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
            if let Some(file) = self.pending.take() {
                // Clean up temp file if needed
                if let FileData::TempFile { path, .. } = file.data {
                    if path.exists() {
                        if let Err(e) = std::fs::remove_file(&path) {
                            log::warn!("Failed to remove temp file {}: {}", path.display(), e);
                        }
                    }
                }
            }
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

        let size = file.data.size();

        match file.data {
            FileData::Memory(data) => {
                std::fs::write(path, &data)?;
            }
            FileData::TempFile {
                path: temp_path, ..
            } => {
                // Move or copy the temp file to the destination
                if let Err(_) = std::fs::rename(&temp_path, path) {
                    // rename failed (likely cross-device), fall back to copy
                    std::fs::copy(&temp_path, path)?;
                    // Clean up temp file
                    let _ = std::fs::remove_file(&temp_path);
                }
            }
        }

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
