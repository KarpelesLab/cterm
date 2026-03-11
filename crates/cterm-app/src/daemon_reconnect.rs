//! Daemon session reconnection for upgrades and restarts
//!
//! When ctermd is running with existing sessions, a new cterm UI instance
//! can reconnect to them automatically. This simplifies upgrades — instead
//! of complex FD passing and state serialization, the new UI just reconnects
//! to the daemon's sessions.

use cterm_client::{ClientError, DaemonConnection, SessionHandle};

/// Information about available daemon sessions for reconnection
pub struct DaemonSessionInfo {
    pub session_id: String,
    pub title: String,
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
                    cols: s.cols,
                    rows: s.rows,
                    running: s.running,
                })
                .collect(),
        ),
        Err(_) => ReconnectCheck::NotAvailable,
    }
}

/// Reconnect to all running daemon sessions.
///
/// Returns a list of SessionHandles, one per session. The caller is
/// responsible for creating terminal widgets/tabs for each.
pub async fn reconnect_all_sessions() -> Result<Vec<SessionHandle>, ClientError> {
    let socket_path = cterm_client::default_socket_path();
    let conn = DaemonConnection::connect_unix(&socket_path, false).await?;

    let sessions = conn.list_sessions().await?;
    let mut handles = Vec::new();

    for session_info in sessions {
        if !session_info.running {
            continue;
        }
        match conn
            .attach_session(
                &session_info.session_id,
                session_info.cols,
                session_info.rows,
            )
            .await
        {
            Ok((handle, _screen)) => {
                handles.push(handle);
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

    Ok(handles)
}
