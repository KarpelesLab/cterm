//! Thread-safe session manager

use crate::error::{HeadlessError, Result};
use crate::session::{generate_session_id, SessionState};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Thread-safe manager for terminal sessions
pub struct SessionManager {
    sessions: RwLock<HashMap<String, Arc<SessionState>>>,
}

impl SessionManager {
    /// Create a new session manager
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    /// Create a new terminal session
    #[allow(clippy::too_many_arguments)]
    pub fn create_session(
        &self,
        cols: usize,
        rows: usize,
        shell: Option<String>,
        args: Vec<String>,
        cwd: Option<PathBuf>,
        env: Vec<(String, String)>,
        term: Option<String>,
    ) -> Result<Arc<SessionState>> {
        let id = generate_session_id();

        // Check for collision (extremely unlikely with UUID v4)
        if self.sessions.read().contains_key(&id) {
            return Err(HeadlessError::SessionAlreadyExists(id));
        }

        let state = SessionState::new(id.clone(), cols, rows, shell, args, cwd, env, term)?;

        // Start the PTY reader task
        let state = state.start_reader()?;

        // Store the session
        self.sessions.write().insert(id, Arc::clone(&state));

        log::info!("Created session {} ({}x{})", state.id, cols, rows);

        Ok(state)
    }

    /// Get a session by ID
    pub fn get_session(&self, id: &str) -> Result<Arc<SessionState>> {
        self.sessions
            .read()
            .get(id)
            .cloned()
            .ok_or_else(|| HeadlessError::SessionNotFound(id.to_string()))
    }

    /// List all sessions
    pub fn list_sessions(&self) -> Vec<Arc<SessionState>> {
        self.sessions.read().values().cloned().collect()
    }

    /// Destroy a session
    pub fn destroy_session(&self, id: &str, signal: Option<i32>) -> Result<()> {
        let session = self
            .sessions
            .write()
            .remove(id)
            .ok_or_else(|| HeadlessError::SessionNotFound(id.to_string()))?;

        // Send signal to terminate the process
        #[cfg(unix)]
        let sig = signal.unwrap_or(libc::SIGHUP);
        #[cfg(not(unix))]
        let sig = signal.unwrap_or(15); // SIGTERM

        let _ = session.send_signal(sig);

        log::info!("Destroyed session {}", id);

        Ok(())
    }

    /// Get the number of active sessions
    pub fn session_count(&self) -> usize {
        self.sessions.read().len()
    }

    /// Clean up dead sessions
    pub fn cleanup_dead_sessions(&self) {
        let mut sessions = self.sessions.write();
        let dead_ids: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| !s.is_running())
            .map(|(id, _)| id.clone())
            .collect();

        for id in dead_ids {
            sessions.remove(&id);
            log::info!("Cleaned up dead session {}", id);
        }
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_manager_new() {
        let manager = SessionManager::new();
        assert_eq!(manager.session_count(), 0);
    }

    #[test]
    fn test_session_not_found() {
        let manager = SessionManager::new();
        let result = manager.get_session("nonexistent");
        assert!(matches!(result, Err(HeadlessError::SessionNotFound(_))));
    }
}
