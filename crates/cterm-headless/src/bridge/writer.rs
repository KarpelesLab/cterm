//! Dedicated PTY writer thread.
//!
//! All writes to a session's PTY master — keyboard input and parser-generated
//! responses (DSR/DA/cursor reports, etc.) — are routed through a single
//! [`PtyWriter`]. The actual `write_all` happens on a dedicated OS thread that is
//! *expected* to block: if the child stops draining its stdin and the PTY buffer
//! fills, that thread blocks instead of a tokio worker thread holding the terminal
//! lock. This prevents a wedged child from starving the gRPC runtime (the cause of
//! the daemon deadlock).
//!
//! A single FIFO channel preserves the relative ordering of input and responses.
//! The channel is bounded; if it fills (child wedged), further writes are dropped
//! and logged rather than blocking the enqueuer — bounding memory and never stalling
//! a worker thread. A terminal whose child has stopped reading is already
//! non-functional, so dropping further bytes is the correct degradation.

use std::io::Write;
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};

/// Maximum number of queued write messages before new writes are dropped.
const WRITE_QUEUE_CAP: usize = 1024;

/// Routes PTY writes to a dedicated blocking writer thread.
pub struct PtyWriter {
    tx: SyncSender<Vec<u8>>,
}

impl PtyWriter {
    /// Spawn a writer thread that drains the queue into the given PTY master `writer`.
    ///
    /// `session_id` is used only for logging. The thread exits when the [`PtyWriter`]
    /// (and thus the sender) is dropped, or when a write fails (e.g. the child exited
    /// and the PTY closed).
    pub fn new(mut writer: std::fs::File, session_id: String) -> Self {
        let (tx, rx) = sync_channel::<Vec<u8>>(WRITE_QUEUE_CAP);

        std::thread::Builder::new()
            .name(format!("pty-writer-{session_id}"))
            .spawn(move || {
                while let Ok(data) = rx.recv() {
                    if let Err(e) = writer.write_all(&data) {
                        log::debug!("PTY writer for session {session_id} stopping: {e}");
                        break;
                    }
                }
                log::debug!("PTY writer thread exiting for session {session_id}");
            })
            .expect("failed to spawn PTY writer thread");

        Self { tx }
    }

    /// Enqueue bytes to be written to the PTY. Never blocks: if the queue is full
    /// (child not draining), the write is dropped and logged.
    pub fn send(&self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        match self.tx.try_send(data.to_vec()) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                log::warn!(
                    "PTY write queue full ({} bytes dropped); child not draining stdin",
                    data.len()
                );
            }
            Err(TrySendError::Disconnected(_)) => {
                // Writer thread has exited (PTY closed / child gone). Drop silently.
            }
        }
    }
}
