//! Async PTY reader task

use crate::session::{OutputData, SessionState};
use std::io::Read;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// A blocking PTY/SSH output reader (either a duped FD or an SSH channel).
type BoxedReader = Box<dyn Read + Send>;

/// PTY reader that bridges blocking PTY reads to async
pub struct PtyReader {
    reader: BoxedReader,
}

impl PtyReader {
    /// Create a new PTY reader
    pub fn new(reader: BoxedReader) -> Self {
        Self { reader }
    }

    /// Run the reader loop, broadcasting output to the session
    pub async fn run(self, session: Arc<SessionState>) {
        // The boxed reader is not cloneable, so we hand ownership into each
        // blocking read and take it back out, threading it through the loop.
        let mut reader = self.reader;

        loop {
            let read_result = tokio::task::spawn_blocking(move || {
                let mut buf = [0u8; 8192];
                let n = reader.read(&mut buf);
                (n, buf, reader)
            })
            .await;

            let (n, data, returned) = match read_result {
                Ok(triple) => triple,
                Err(e) => {
                    log::error!("spawn_blocking panicked: {}", e);
                    break;
                }
            };
            reader = returned;

            match n.map(|n| (n, data)) {
                Ok((0, _)) => {
                    // EOF - PTY closed
                    log::debug!("PTY reader got EOF for session {}", session.id);
                    break;
                }
                Ok((n, data)) => {
                    let data = data[..n].to_vec();
                    let timestamp_ms = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);

                    // Process the data through the terminal
                    let events = session.process_output(&data);

                    // Broadcast the raw output
                    session.broadcast_output(OutputData {
                        data: data.clone(),
                        timestamp_ms,
                    });

                    // Broadcast events; set alerted state on bell
                    for event in &events {
                        if matches!(event, cterm_core::term::TerminalEvent::Bell) {
                            session.set_alerted(true);
                        }
                    }
                    for event in events {
                        session.broadcast_event(event);
                    }
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    log::debug!("PTY read error for session {}: {}", session.id, e);
                    break;
                }
            }
        }

        log::debug!("PTY reader task exiting for session {}", session.id);
    }
}
