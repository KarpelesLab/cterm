//! Daemon session reconnection for upgrades and restarts
//!
//! When ctermd is running with existing sessions, a new cterm UI instance
//! can reconnect to them automatically. This simplifies upgrades — instead
//! of complex FD passing and state serialization, the new UI just reconnects
//! to the daemon's sessions.

use cterm_client::{ClientError, DaemonConnection, SessionHandle};
use cterm_core::Terminal;
use cterm_proto::proto::GetScreenResponse;

/// Information about available daemon sessions for reconnection
pub struct DaemonSessionInfo {
    pub session_id: String,
    pub title: String,
    pub custom_title: String,
    pub cols: u32,
    pub rows: u32,
    pub running: bool,
}

/// Result of checking for reconnectable daemon sessions
pub enum ReconnectCheck {
    /// Daemon is running with sessions available
    Available(Vec<DaemonSessionInfo>),
    /// Daemon is running but has no sessions
    NoSessions,
    /// Daemon is not running or not reachable
    NotAvailable,
}

/// Check if there are daemon sessions available for reconnection.
///
/// This is a non-blocking check that returns quickly. It does NOT
/// auto-start the daemon.
pub async fn check_daemon_sessions() -> ReconnectCheck {
    // Try to connect without auto-starting
    let socket_path = cterm_client::default_socket_path();
    let conn = match DaemonConnection::connect_unix(&socket_path, false).await {
        Ok(conn) => conn,
        Err(_) => return ReconnectCheck::NotAvailable,
    };

    match conn.list_sessions().await {
        Ok(sessions) if sessions.is_empty() => ReconnectCheck::NoSessions,
        Ok(sessions) => ReconnectCheck::Available(
            sessions
                .into_iter()
                .map(|s| DaemonSessionInfo {
                    session_id: s.session_id,
                    title: s.title,
                    custom_title: s.custom_title,
                    cols: s.cols,
                    rows: s.rows,
                    running: s.running,
                })
                .collect(),
        ),
        Err(_) => ReconnectCheck::NotAvailable,
    }
}

/// A reconnected daemon session with its handle, title, and screen snapshot
pub struct ReconnectedSession {
    /// Handle to the daemon session
    pub handle: SessionHandle,
    /// Session title from the daemon
    pub title: String,
    /// User-set custom title (empty if none)
    pub custom_title: String,
    /// Initial screen snapshot (if available)
    pub screen: Option<GetScreenResponse>,
}

impl ReconnectedSession {
    /// Apply the screen snapshot (if any) to a local terminal.
    ///
    /// This restores visible rows, scrollback, cursor position, title,
    /// and terminal modes from the daemon's screen state. Should be called
    /// BEFORE starting the output stream so the terminal shows correct
    /// content immediately.
    pub fn apply_screen(&self, terminal: &mut Terminal) {
        if let Some(ref screen_data) = self.screen {
            crate::daemon_session::apply_screen_snapshot(terminal, screen_data);
        }
    }
}

/// Reconnect to all running daemon sessions.
///
/// Returns a list of ReconnectedSessions including screen snapshots.
/// The caller is responsible for creating terminal widgets/tabs for each.
pub async fn reconnect_all_sessions() -> Result<Vec<ReconnectedSession>, ClientError> {
    let socket_path = cterm_client::default_socket_path();
    let conn = DaemonConnection::connect_unix(&socket_path, false).await?;

    let sessions = conn.list_sessions().await?;
    let mut results = Vec::new();

    for session_info in sessions {
        if !session_info.running {
            continue;
        }
        let title = session_info.title.clone();
        let custom_title = session_info.custom_title.clone();
        match conn
            .attach_session(
                &session_info.session_id,
                session_info.cols,
                session_info.rows,
            )
            .await
        {
            Ok((handle, screen)) => {
                results.push(ReconnectedSession {
                    handle,
                    title,
                    custom_title,
                    screen,
                });
            }
            Err(e) => {
                log::warn!(
                    "Failed to reattach session {}: {}",
                    session_info.session_id,
                    e
                );
            }
        }
    }

    Ok(results)
}
