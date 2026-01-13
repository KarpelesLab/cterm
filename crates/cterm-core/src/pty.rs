//! Cross-platform PTY handling
//!
//! Uses the `portable-pty` crate to provide PTY functionality on
//! Linux, macOS, and Windows.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;
use portable_pty::{
    native_pty_system, Child, CommandBuilder, MasterPty, PtySize,
};
use thiserror::Error;
use tokio::sync::mpsc;

/// Errors that can occur with PTY operations
#[derive(Error, Debug)]
pub enum PtyError {
    #[error("Failed to create PTY: {0}")]
    Create(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("Failed to spawn process: {0}")]
    Spawn(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("PTY not running")]
    NotRunning,
}

impl From<anyhow::Error> for PtyError {
    fn from(err: anyhow::Error) -> Self {
        PtyError::Create(err.into())
    }
}

/// PTY configuration
#[derive(Debug, Clone)]
pub struct PtyConfig {
    /// Initial terminal size
    pub size: PtySize,
    /// Shell command to run (None = default shell)
    pub shell: Option<String>,
    /// Arguments to pass to the shell
    pub args: Vec<String>,
    /// Working directory
    pub cwd: Option<PathBuf>,
    /// Environment variables to set
    pub env: Vec<(String, String)>,
}

impl Default for PtyConfig {
    fn default() -> Self {
        Self {
            size: PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            },
            shell: None,
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
        }
    }
}

/// PTY handle for reading/writing to a pseudo-terminal
pub struct Pty {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    reader: Arc<Mutex<Box<dyn Read + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl Pty {
    /// Create a new PTY with the given configuration
    pub fn new(config: &PtyConfig) -> Result<Self, PtyError> {
        let pty_system = native_pty_system();

        let pair = pty_system.openpty(config.size)?;

        // Build the command
        let mut cmd = if let Some(ref shell) = config.shell {
            CommandBuilder::new(shell)
        } else {
            CommandBuilder::new(Self::default_shell())
        };

        // Add arguments
        for arg in &config.args {
            cmd.arg(arg);
        }

        // Set working directory
        if let Some(ref cwd) = config.cwd {
            cmd.cwd(cwd);
        }

        // Set environment variables
        for (key, value) in &config.env {
            cmd.env(key, value);
        }

        // Set TERM environment variable
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        // Spawn the process
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::Spawn(e.to_string()))?;

        // Get reader and writer
        let reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        Ok(Self {
            master: Arc::new(Mutex::new(pair.master)),
            child: Arc::new(Mutex::new(child)),
            reader: Arc::new(Mutex::new(reader)),
            writer: Arc::new(Mutex::new(writer)),
        })
    }

    /// Get the default shell for the current platform
    fn default_shell() -> &'static str {
        #[cfg(windows)]
        {
            "powershell.exe"
        }
        #[cfg(not(windows))]
        {
            // Try to get shell from environment or default to bash
            std::env::var("SHELL")
                .ok()
                .map(|s| {
                    // Leak the string to get a static reference
                    // This is fine since we only call this once per PTY
                    Box::leak(s.into_boxed_str()) as &str
                })
                .unwrap_or("/bin/bash")
        }
    }

    /// Write data to the PTY
    pub fn write(&self, data: &[u8]) -> Result<usize, PtyError> {
        let mut writer = self.writer.lock();
        let n = writer.write(data)?;
        writer.flush()?;
        Ok(n)
    }

    /// Read data from the PTY (blocking)
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, PtyError> {
        let mut reader = self.reader.lock();
        Ok(reader.read(buf)?)
    }

    /// Resize the PTY
    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), PtyError> {
        let master = self.master.lock();
        master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    /// Check if the child process is still running
    pub fn is_running(&self) -> bool {
        let mut child = self.child.lock();
        match child.try_wait() {
            Ok(Some(_)) => false, // Process exited
            Ok(None) => true,     // Still running
            Err(_) => false,      // Error, assume not running
        }
    }

    /// Wait for the child process to exit
    pub fn wait(&self) -> Result<u32, PtyError> {
        let mut child = self.child.lock();
        let status = child.wait().map_err(|e| PtyError::Spawn(e.to_string()))?;
        Ok(status.exit_code())
    }

    /// Kill the child process
    pub fn kill(&self) -> Result<(), PtyError> {
        let mut child = self.child.lock();
        child.kill().map_err(|e| PtyError::Spawn(e.to_string()))?;
        Ok(())
    }

    /// Get a clone of the reader for async operations
    pub fn clone_reader(&self) -> Arc<Mutex<Box<dyn Read + Send>>> {
        Arc::clone(&self.reader)
    }

    /// Get a clone of the writer for async operations
    pub fn clone_writer(&self) -> Arc<Mutex<Box<dyn Write + Send>>> {
        Arc::clone(&self.writer)
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        // Try to kill the child process if it's still running
        let _ = self.kill();
    }
}

/// Async PTY reader using channels
pub struct AsyncPtyReader {
    rx: mpsc::Receiver<Vec<u8>>,
}

impl AsyncPtyReader {
    /// Create a new async reader for the PTY
    pub fn new(pty: &Pty, buffer_size: usize) -> (Self, tokio::task::JoinHandle<()>) {
        let (tx, rx) = mpsc::channel(32);
        let reader = pty.clone_reader();

        let handle = tokio::task::spawn_blocking(move || {
            let mut buf = vec![0u8; buffer_size];
            loop {
                let mut reader = reader.lock();
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(n) => {
                        if tx.blocking_send(buf[..n].to_vec()).is_err() {
                            break; // Receiver dropped
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        (Self { rx }, handle)
    }

    /// Receive data from the PTY
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.rx.recv().await
    }
}

/// Helper to spawn a shell with a specific command
pub fn spawn_with_command(command: &str, cwd: Option<PathBuf>) -> Result<Pty, PtyError> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

    let config = PtyConfig {
        shell: Some(shell),
        args: vec!["-c".to_string(), command.to_string()],
        cwd,
        ..Default::default()
    };

    Pty::new(&config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pty_config_default() {
        let config = PtyConfig::default();
        assert_eq!(config.size.rows, 24);
        assert_eq!(config.size.cols, 80);
        assert!(config.shell.is_none());
    }

    #[test]
    #[ignore] // Requires actual PTY support
    fn test_pty_create() {
        let config = PtyConfig::default();
        let pty = Pty::new(&config);
        assert!(pty.is_ok());
    }
}
