//! Transport bridge connecting remote clients to ctermd sessions.
//!
//! The bridge handles server-side rendering: it subscribes to terminal
//! events, renders screen diffs as ANSI, and sends them to the transport.

use crate::session::SessionState;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use super::renderer::{self, ScreenSnapshot};
use cterm_core::term::TerminalEvent;

/// Commands from the transport to the bridge.
pub enum BridgeCommand {
    /// Client sent input data (keystrokes).
    Input(Vec<u8>),
    /// Client resized their terminal.
    Resize(u32, u32),
    /// Client disconnected.
    Disconnect,
}

/// A bridge between a transport and a ctermd session.
///
/// Handles server-side rendering: subscribes to session events,
/// renders screen diffs, sends ANSI frames to the output callback.
pub struct LatchBridge {
    session: Arc<SessionState>,
    cmd_rx: mpsc::Receiver<BridgeCommand>,
    output_tx: mpsc::Sender<Vec<u8>>,
    coalesce_ms: u64,
}

impl LatchBridge {
    /// Create a new bridge.
    ///
    /// - `session`: The ctermd session to bridge to
    /// - `cmd_rx`: Receives input/resize/disconnect commands from the transport
    /// - `output_tx`: Sends rendered ANSI frames to the transport
    /// - `coalesce_ms`: Render coalesce window in milliseconds
    pub fn new(
        session: Arc<SessionState>,
        cmd_rx: mpsc::Receiver<BridgeCommand>,
        output_tx: mpsc::Sender<Vec<u8>>,
        coalesce_ms: u64,
    ) -> Self {
        Self {
            session,
            cmd_rx,
            output_tx,
            coalesce_ms,
        }
    }

    /// Run the bridge event loop. Blocks until the client disconnects
    /// or the session dies.
    pub async fn run(mut self) {
        self.session.attach();

        // Send initial full screen render
        let initial_frame = self
            .session
            .with_terminal(|term| renderer::render_full(term.screen()));
        if self.output_tx.send(initial_frame).await.is_err() {
            self.session.detach();
            return;
        }

        // Take initial snapshot for diffing
        let mut snapshot = self
            .session
            .with_terminal(|term| ScreenSnapshot::capture(term.screen()));

        // Subscribe to terminal events for render triggers
        let mut event_rx = self.session.subscribe_events();
        let coalesce = Duration::from_millis(self.coalesce_ms);

        loop {
            tokio::select! {
                // Handle commands from the transport
                cmd = self.cmd_rx.recv() => {
                    match cmd {
                        Some(BridgeCommand::Input(data)) => {
                            if let Err(e) = self.session.write_input(&data) {
                                log::error!("Failed to write to PTY: {}", e);
                                break;
                            }
                        }
                        Some(BridgeCommand::Resize(cols, rows)) => {
                            self.session.resize(cols as usize, rows as usize);
                        }
                        Some(BridgeCommand::Disconnect) | None => {
                            break;
                        }
                    }
                }

                // Handle terminal events (content changed, process exited, etc.)
                event = event_rx.recv() => {
                    match event {
                        Ok(TerminalEvent::ProcessExited(_)) => {
                            // Send final screen render before closing
                            let frame = self.session.with_terminal(|term| {
                                renderer::render_diff(&snapshot, term.screen())
                            });
                            if !frame.is_empty() {
                                let _ = self.output_tx.send(frame).await;
                            }
                            break;
                        }
                        Ok(TerminalEvent::ContentChanged) => {
                            // Coalesce: wait briefly for more changes
                            tokio::time::sleep(coalesce).await;
                            // Drain any queued ContentChanged events
                            while let Ok(TerminalEvent::ContentChanged) = event_rx.try_recv() {}

                            // Render diff
                            let frame = self.session.with_terminal(|term| {
                                renderer::render_diff(&snapshot, term.screen())
                            });

                            // Update snapshot
                            snapshot = self.session.with_terminal(|term| {
                                ScreenSnapshot::capture(term.screen())
                            });

                            if !frame.is_empty()
                                && self.output_tx.send(frame).await.is_err()
                            {
                                break;
                            }
                        }
                        Ok(_) => {
                            // Other events (title change, bell, etc.) — ignore for now
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            log::warn!("Bridge lagged {} events, doing full render", n);
                            // Full re-render after lag
                            let frame = self.session.with_terminal(|term| {
                                renderer::render_full(term.screen())
                            });
                            snapshot = self.session.with_terminal(|term| {
                                ScreenSnapshot::capture(term.screen())
                            });
                            if self.output_tx.send(frame).await.is_err() {
                                break;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            break;
                        }
                    }
                }
            }
        }

        self.session.detach();
    }
}

/// Create a bridge channel pair.
///
/// Returns (cmd_tx, output_rx) for the transport side,
/// and spawns the bridge task in the background.
pub fn spawn_bridge(
    session: Arc<SessionState>,
    coalesce_ms: u64,
) -> (mpsc::Sender<BridgeCommand>, mpsc::Receiver<Vec<u8>>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(256);
    let (output_tx, output_rx) = mpsc::channel(64);

    let bridge = LatchBridge::new(session, cmd_rx, output_tx, coalesce_ms);
    tokio::spawn(bridge.run());

    (cmd_tx, output_rx)
}
