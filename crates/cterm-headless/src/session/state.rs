//! Session state management

use crate::bridge::PtyReader;
use crate::error::Result;
use cterm_core::screen::ScreenConfig;
use cterm_core::term::TerminalEvent;
use cterm_core::{PtyConfig, PtySize, Terminal};
use parking_lot::RwLock;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Output chunk with timestamp
#[derive(Clone, Debug)]
pub struct OutputData {
    pub data: Vec<u8>,
    pub timestamp_ms: u64,
}

/// Session state wrapping a Terminal instance
pub struct SessionState {
    /// The terminal instance
    terminal: RwLock<Terminal>,

    /// Session ID
    pub id: String,

    /// Broadcast sender for output data
    output_tx: broadcast::Sender<OutputData>,

    /// Broadcast sender for terminal events
    event_tx: broadcast::Sender<TerminalEvent>,
}

impl SessionState {
    /// Create a new session with the given configuration
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        cols: usize,
        rows: usize,
        shell: Option<String>,
        args: Vec<String>,
        cwd: Option<std::path::PathBuf>,
        env: Vec<(String, String)>,
        term: Option<String>,
    ) -> Result<Arc<Self>> {
        let pty_config = PtyConfig {
            size: PtySize {
                cols: cols as u16,
                rows: rows as u16,
                ..Default::default()
            },
            shell,
            args,
            cwd,
            env,
            term,
        };

        let terminal = Terminal::with_shell(cols, rows, ScreenConfig::default(), &pty_config)?;

        // Create broadcast channels
        let (output_tx, _) = broadcast::channel(1024);
        let (event_tx, _) = broadcast::channel(256);

        let state = Arc::new(Self {
            terminal: RwLock::new(terminal),
            id,
            output_tx,
            event_tx,
        });

        Ok(state)
    }

    /// Start the PTY reader task
    pub fn start_reader(self: &Arc<Self>) -> Result<Arc<Self>> {
        let pty_reader = self.terminal.read().pty_reader();

        if let Some(reader) = pty_reader {
            let state = Arc::clone(self);
            // Spawn the reader task - it will run until the PTY closes
            tokio::spawn(async move {
                let pty_reader = PtyReader::new(reader);
                pty_reader.run(state).await;
            });
        }

        Ok(Arc::clone(self))
    }

    /// Get the terminal dimensions
    pub fn dimensions(&self) -> (usize, usize) {
        let term = self.terminal.read();
        (term.cols(), term.rows())
    }

    /// Get the terminal title
    pub fn title(&self) -> String {
        self.terminal.read().title().to_string()
    }

    /// Check if the terminal is still running
    pub fn is_running(&self) -> bool {
        self.terminal.write().is_running()
    }

    /// Get the child process ID
    pub fn child_pid(&self) -> Option<i32> {
        self.terminal.read().child_pid()
    }

    /// Write input to the terminal
    pub fn write_input(&self, data: &[u8]) -> Result<usize> {
        let mut term = self.terminal.write();
        term.write(data)?;
        Ok(data.len())
    }

    /// Resize the terminal
    pub fn resize(&self, cols: usize, rows: usize) {
        self.terminal.write().resize(cols, rows);
    }

    /// Send a signal to the child process
    pub fn send_signal(&self, signal: i32) -> Result<()> {
        self.terminal.read().send_signal(signal)?;
        Ok(())
    }

    /// Process PTY output data
    pub fn process_output(&self, data: &[u8]) -> Vec<TerminalEvent> {
        self.terminal.write().process(data)
    }

    /// Broadcast output data to subscribers
    pub fn broadcast_output(&self, data: OutputData) {
        let _ = self.output_tx.send(data);
    }

    /// Broadcast a terminal event to subscribers
    pub fn broadcast_event(&self, event: TerminalEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Subscribe to output stream
    pub fn subscribe_output(&self) -> broadcast::Receiver<OutputData> {
        self.output_tx.subscribe()
    }

    /// Subscribe to event stream
    pub fn subscribe_events(&self) -> broadcast::Receiver<TerminalEvent> {
        self.event_tx.subscribe()
    }

    /// Handle a key press and return the escape sequence
    pub fn handle_key(
        &self,
        key: cterm_core::term::Key,
        modifiers: cterm_core::term::Modifiers,
    ) -> Option<Vec<u8>> {
        self.terminal.read().handle_key(key, modifiers)
    }

    /// Get a reference to the terminal (for reading screen state)
    pub fn with_terminal<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Terminal) -> R,
    {
        let term = self.terminal.read();
        f(&term)
    }

    /// Get a mutable reference to the terminal
    pub fn with_terminal_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Terminal) -> R,
    {
        let mut term = self.terminal.write();
        f(&mut term)
    }
}
