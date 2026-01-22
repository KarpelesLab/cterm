//! Streaming file receiver for large file transfers
//!
//! Handles incremental base64 decoding and automatic memory-to-disk spillover
//! when file size exceeds a threshold.

use crate::iterm2::Iterm2FileParams;
use base64::Engine;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

/// Threshold for switching from memory buffer to temp file (1 MB)
const MEMORY_THRESHOLD: usize = 1024 * 1024;

/// Base64 decode chunk size (must be multiple of 4 for base64)
const BASE64_CHUNK_SIZE: usize = 4096;

/// State of the streaming file receiver
#[derive(Debug)]
enum StorageState {
    /// Data is stored in memory
    Memory(Vec<u8>),
    /// Data has been spilled to a temp file
    File {
        path: PathBuf,
        writer: BufWriter<File>,
        size: usize,
    },
}

/// Streaming file receiver that handles incremental base64 decoding
///
/// Data flow:
/// 1. Receives base64 bytes incrementally via `put()`
/// 2. Decodes base64 in chunks
/// 3. Stores in memory until MEMORY_THRESHOLD is reached
/// 4. Spills to temp file if threshold exceeded
#[derive(Debug)]
pub struct StreamingFileReceiver {
    /// Parsed iTerm2 file parameters
    params: Iterm2FileParams,
    /// Buffer for incomplete base64 chunks (0-3 bytes)
    base64_buffer: Vec<u8>,
    /// Where the decoded data is stored
    storage: StorageState,
    /// Total decoded bytes received
    total_bytes: usize,
    /// Whether we've encountered an error
    error: Option<String>,
}

impl StreamingFileReceiver {
    /// Create a new streaming file receiver with parsed parameters
    pub fn new(params: Iterm2FileParams) -> Self {
        // Pre-allocate memory based on expected size if known
        let initial_capacity = params.size.map(|s| s.min(MEMORY_THRESHOLD)).unwrap_or(8192);

        Self {
            params,
            base64_buffer: Vec::with_capacity(BASE64_CHUNK_SIZE),
            storage: StorageState::Memory(Vec::with_capacity(initial_capacity)),
            total_bytes: 0,
            error: None,
        }
    }

    /// Get the file parameters
    pub fn params(&self) -> &Iterm2FileParams {
        &self.params
    }

    /// Get the total bytes decoded so far
    pub fn total_bytes(&self) -> usize {
        self.total_bytes
    }

    /// Get expected size if known
    pub fn expected_size(&self) -> Option<usize> {
        self.params.size
    }

    /// Check if an error occurred
    pub fn has_error(&self) -> bool {
        self.error.is_some()
    }

    /// Get the error message if any
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Get download progress as a fraction (0.0 - 1.0), or None if size unknown
    pub fn progress(&self) -> Option<f32> {
        self.params.size.map(|expected| {
            if expected == 0 {
                1.0
            } else {
                (self.total_bytes as f32 / expected as f32).min(1.0)
            }
        })
    }

    /// Feed base64 encoded bytes to the receiver
    ///
    /// Returns true if more data is expected, false if an error occurred
    pub fn put(&mut self, byte: u8) -> bool {
        if self.error.is_some() {
            return false;
        }

        // Skip whitespace in base64 data
        if byte.is_ascii_whitespace() {
            return true;
        }

        // Add to base64 buffer
        self.base64_buffer.push(byte);

        // Decode when we have enough base64 data
        if self.base64_buffer.len() >= BASE64_CHUNK_SIZE {
            if !self.decode_chunk() {
                return false;
            }
        }

        true
    }

    /// Feed multiple base64 bytes at once
    pub fn put_bytes(&mut self, bytes: &[u8]) -> bool {
        for &byte in bytes {
            if !self.put(byte) {
                return false;
            }
        }
        true
    }

    /// Decode accumulated base64 data in the buffer
    fn decode_chunk(&mut self) -> bool {
        if self.base64_buffer.is_empty() {
            return true;
        }

        // Base64 works in groups of 4 characters
        // Keep any remainder for the next chunk
        let decode_len = (self.base64_buffer.len() / 4) * 4;
        if decode_len == 0 {
            return true;
        }

        let to_decode = &self.base64_buffer[..decode_len];

        match base64::engine::general_purpose::STANDARD.decode(to_decode) {
            Ok(decoded) => {
                if !self.write_decoded(&decoded) {
                    return false;
                }
                // Keep remainder
                let remainder: Vec<u8> = self.base64_buffer[decode_len..].to_vec();
                self.base64_buffer = remainder;
                true
            }
            Err(e) => {
                self.error = Some(format!("Base64 decode error: {}", e));
                false
            }
        }
    }

    /// Write decoded bytes to storage, spilling to disk if needed
    fn write_decoded(&mut self, data: &[u8]) -> bool {
        self.total_bytes += data.len();

        // Check if we need to spill from memory to disk
        let should_spill = match &self.storage {
            StorageState::Memory(buffer) => buffer.len() + data.len() > MEMORY_THRESHOLD,
            StorageState::File { .. } => false,
        };

        if should_spill {
            // Take ownership of the buffer for spilling
            let old_storage = std::mem::replace(
                &mut self.storage,
                StorageState::Memory(Vec::new()), // Temporary placeholder
            );

            if let StorageState::Memory(existing_data) = old_storage {
                match self.spill_to_disk(&existing_data, data) {
                    Ok(()) => return true,
                    Err(e) => {
                        self.error = Some(format!("Failed to create temp file: {}", e));
                        return false;
                    }
                }
            }
        }

        match &mut self.storage {
            StorageState::Memory(buffer) => {
                buffer.extend_from_slice(data);
                true
            }
            StorageState::File { writer, size, .. } => match writer.write_all(data) {
                Ok(()) => {
                    *size += data.len();
                    true
                }
                Err(e) => {
                    self.error = Some(format!("Failed to write to temp file: {}", e));
                    false
                }
            },
        }
    }

    /// Spill memory buffer to a temp file
    fn spill_to_disk(&mut self, existing_data: &[u8], new_data: &[u8]) -> io::Result<()> {
        // Create temp file
        let temp_dir = std::env::temp_dir();
        let temp_path = temp_dir.join(format!("cterm_transfer_{}", std::process::id()));

        let file = File::create(&temp_path)?;
        let mut writer = BufWriter::new(file);

        // Write existing memory data
        writer.write_all(existing_data)?;

        // Write new data
        writer.write_all(new_data)?;

        let size = existing_data.len() + new_data.len();

        log::debug!(
            "Spilled file transfer to disk: {} ({} bytes so far)",
            temp_path.display(),
            size
        );

        self.storage = StorageState::File {
            path: temp_path,
            writer,
            size,
        };

        Ok(())
    }

    /// Finish receiving the file and get the result
    ///
    /// Returns the file parameters, data (memory or temp file path), and total size.
    pub fn finish(mut self) -> Result<StreamingFileResult, String> {
        // Decode any remaining base64 data
        if !self.base64_buffer.is_empty() {
            // Pad with '=' if needed for final chunk
            while self.base64_buffer.len() % 4 != 0 {
                self.base64_buffer.push(b'=');
            }
            if !self.decode_chunk() {
                return Err(self.error.unwrap_or_else(|| "Unknown error".to_string()));
            }
        }

        if let Some(error) = self.error {
            return Err(error);
        }

        let data = match self.storage {
            StorageState::Memory(buffer) => StreamingFileData::Memory(buffer),
            StorageState::File {
                path,
                mut writer,
                size,
            } => {
                // Flush the writer
                if let Err(e) = writer.flush() {
                    return Err(format!("Failed to flush temp file: {}", e));
                }
                drop(writer);
                StreamingFileData::TempFile { path, size }
            }
        };

        Ok(StreamingFileResult {
            params: self.params,
            data,
            total_bytes: self.total_bytes,
        })
    }

    /// Check if data is stored on disk
    pub fn is_on_disk(&self) -> bool {
        matches!(self.storage, StorageState::File { .. })
    }
}

/// Result of a completed streaming file transfer
#[derive(Debug)]
pub struct StreamingFileResult {
    /// The parsed file parameters
    pub params: Iterm2FileParams,
    /// The file data (memory or temp file)
    pub data: StreamingFileData,
    /// Total bytes received
    pub total_bytes: usize,
}

/// Where the file data is stored
#[derive(Debug)]
pub enum StreamingFileData {
    /// Data is in memory
    Memory(Vec<u8>),
    /// Data is in a temp file
    TempFile { path: PathBuf, size: usize },
}

impl StreamingFileData {
    /// Read the data into a Vec<u8>
    ///
    /// For memory data, clones the data.
    /// For temp files, reads the file into memory.
    pub fn to_bytes(&self) -> io::Result<Vec<u8>> {
        match self {
            StreamingFileData::Memory(data) => Ok(data.clone()),
            StreamingFileData::TempFile { path, .. } => std::fs::read(path),
        }
    }

    /// Take the data, consuming self
    ///
    /// For memory data, returns the data directly.
    /// For temp files, reads the file into memory and deletes the temp file.
    pub fn take(self) -> io::Result<Vec<u8>> {
        match self {
            StreamingFileData::Memory(data) => Ok(data),
            StreamingFileData::TempFile { ref path, .. } => {
                let data = std::fs::read(path)?;
                // Delete temp file after reading
                let _ = std::fs::remove_file(path);
                Ok(data)
            }
        }
    }

    /// Get the temp file path if data is on disk
    pub fn temp_path(&self) -> Option<&PathBuf> {
        match self {
            StreamingFileData::Memory(_) => None,
            StreamingFileData::TempFile { path, .. } => Some(path),
        }
    }

    /// Get the size
    pub fn size(&self) -> usize {
        match self {
            StreamingFileData::Memory(data) => data.len(),
            StreamingFileData::TempFile { size, .. } => *size,
        }
    }

    /// Check if data is in memory
    pub fn is_memory(&self) -> bool {
        matches!(self, StreamingFileData::Memory(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_streaming_small_file() {
        let params = Iterm2FileParams::default();
        let mut receiver = StreamingFileReceiver::new(params);

        // "Hello, World!" in base64 is "SGVsbG8sIFdvcmxkIQ=="
        let base64_data = b"SGVsbG8sIFdvcmxkIQ==";
        assert!(receiver.put_bytes(base64_data));

        let result = receiver.finish().unwrap();
        assert!(matches!(result.data, StreamingFileData::Memory(_)));

        let bytes = result.data.take().unwrap();
        assert_eq!(bytes, b"Hello, World!");
    }

    #[test]
    fn test_streaming_chunks() {
        let params = Iterm2FileParams::default();
        let mut receiver = StreamingFileReceiver::new(params);

        // Feed data in small chunks
        let base64_data = b"SGVsbG8sIFdvcmxkIQ==";
        for chunk in base64_data.chunks(4) {
            assert!(receiver.put_bytes(chunk));
        }

        let result = receiver.finish().unwrap();
        let bytes = result.data.take().unwrap();
        assert_eq!(bytes, b"Hello, World!");
    }

    #[test]
    fn test_streaming_with_whitespace() {
        let params = Iterm2FileParams::default();
        let mut receiver = StreamingFileReceiver::new(params);

        // Base64 with whitespace
        let base64_data = b"SGVs\nbG8s\nIFdv\ncmxk\nIQ==";
        assert!(receiver.put_bytes(base64_data));

        let result = receiver.finish().unwrap();
        let bytes = result.data.take().unwrap();
        assert_eq!(bytes, b"Hello, World!");
    }
}
