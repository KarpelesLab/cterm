//! Async PTY reader task

use crate::session::{OutputData, SessionState};
use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// PTY reader that bridges blocking PTY reads to async
pub struct PtyReader {
    reader: File,
}

impl PtyReader {
    /// Create a new PTY reader
    pub fn new(reader: File) -> Self {
        Self { reader }
    }

    /// Run the reader loop, broadcasting output to the session
    pub async fn run(self, session: Arc<SessionState>) {
        let mut buf = [0u8; 8192];

        loop {
            // Use spawn_blocking for the blocking read
            let reader = self.reader.try_clone();
            let read_result = match reader {
                Ok(mut r) => {
                    tokio::task::spawn_blocking(move || {
                        let n = r.read(&mut buf)?;
                        Ok::<(usize, [u8; 8192]), std::io::Error>((n, buf))
                    })
                    .await
                }
                Err(e) => {
                    log::error!("Failed to clone PTY reader: {}", e);
                    break;
                }
            };

            match read_result {
                Ok(Ok((0, _))) => {
                    // EOF - PTY closed
                    log::debug!("PTY reader got EOF for session {}", session.id);
                    break;
                }
                Ok(Ok((n, data))) => {
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

                    // Broadcast any events
                    for event in events {
                        session.broadcast_event(event);
                    }
                }
                Ok(Err(e)) => {
                    if e.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    log::debug!("PTY read error for session {}: {}", session.id, e);
                    break;
                }
                Err(e) => {
                    log::error!("spawn_blocking panicked: {}", e);
                    break;
                }
            }
        }

        log::debug!("PTY reader task exiting for session {}", session.id);
    }
}
