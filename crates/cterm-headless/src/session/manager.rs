//! Thread-safe session manager

use crate::error::{HeadlessError, Result};
use crate::session::{generate_session_id, SessionState};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Thread-safe manager for terminal sessions
pub struct SessionManager {
    sessions: RwLock<HashMap<String, Arc<SessionState>>>,
    /// Default scrollback lines for new sessions
    scrollback_lines: usize,
    /// Whether at least one session has ever been created
    had_sessions: AtomicBool,
}

impl SessionManager {
    /// Create a new session manager with default scrollback (10000 lines)
    pub fn new() -> Self {
        Self::with_scrollback(10000)
    }

    /// Create a new session manager with custom scrollback
    pub fn with_scrollback(scrollback_lines: usize) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            scrollback_lines,
            had_sessions: AtomicBool::new(false),
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

        let state = SessionState::new(
            id.clone(),
            cols,
            rows,
            shell,
            args,
            cwd,
            env,
            term,
            self.scrollback_lines,
        )?;

        // Start the PTY reader task
        let state = state.start_reader()?;

        // Store the session
        self.had_sessions.store(true, Ordering::Relaxed);
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

    /// Restore a session from a raw FD (used during relaunch).
    ///
    /// # Safety
    /// The caller must ensure `fd` is a valid PTY master FD and `child_pid` is correct.
    #[cfg(unix)]
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn restore_session(
        &self,
        id: String,
        fd: i32,
        child_pid: i32,
        cols: usize,
        rows: usize,
        custom_title: String,
        tab_color: String,
        template_name: String,
        scrollback_lines: usize,
    ) -> Result<Arc<SessionState>> {
        let state = SessionState::from_raw_fd(
            id.clone(),
            fd,
            child_pid,
            cols,
            rows,
            custom_title,
            tab_color,
            template_name,
            scrollback_lines,
        )?;

        // Start the PTY reader task
        let state = state.start_reader()?;

        self.had_sessions.store(true, Ordering::Relaxed);
        self.sessions.write().insert(id.clone(), Arc::clone(&state));

        log::info!(
            "Restored session {} (fd={}, pid={}, {}x{})",
            id,
            fd,
            child_pid,
            cols,
            rows
        );

        Ok(state)
    }

    /// Get the number of active sessions
    pub fn session_count(&self) -> usize {
        self.sessions.read().len()
    }

    /// Whether at least one session has ever been created
    pub fn had_sessions(&self) -> bool {
        self.had_sessions.load(Ordering::Relaxed)
    }

    /// Clean up dead sessions, returns the number of sessions removed
    pub fn cleanup_dead_sessions(&self) -> usize {
        let mut sessions = self.sessions.write();
        let dead_ids: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| !s.is_running())
            .map(|(id, _)| id.clone())
            .collect();

        let count = dead_ids.len();
        for id in dead_ids {
            sessions.remove(&id);
            log::info!("Cleaned up dead session {}", id);
        }
        count
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
