//! Daemon relaunch (exec-in-place) for seamless upgrades
//!
//! When a relaunch is requested, the daemon:
//! 1. Serializes session state (FDs, PIDs, screen snapshots) to a temp file
//! 2. Clears FD_CLOEXEC on all PTY master FDs so they survive exec
//! 3. exec()s the new (or same) binary with `--relaunch-state <path>`
//! 4. The new process reads the state file, reconstructs sessions from the
//!    preserved FDs, and resumes serving on the same socket path.

use crate::session::SessionManager;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Serialized state for a single session, written to the relaunch state file
#[derive(Serialize, Deserialize, Debug)]
pub struct RelaunchSessionState {
    pub session_id: String,
    /// Raw PTY master file descriptor number (preserved across exec)
    pub master_fd: i32,
    /// Child process PID
    pub child_pid: i32,
    /// Terminal dimensions
    pub cols: usize,
    pub rows: usize,
    /// User-set custom title
    pub custom_title: String,
    /// Scrollback lines setting
    pub scrollback_lines: usize,
}

/// Full relaunch state written to the temp file
#[derive(Serialize, Deserialize, Debug)]
pub struct RelaunchState {
    pub sessions: Vec<RelaunchSessionState>,
    pub socket_path: String,
    pub scrollback_lines: usize,
}

/// Collect relaunch state from the session manager.
///
/// This extracts the raw FD, child PID, dimensions, and custom title
/// from each running session.
pub fn collect_relaunch_state(
    session_manager: &Arc<SessionManager>,
    socket_path: &str,
    scrollback_lines: usize,
) -> RelaunchState {
    let sessions = session_manager.list_sessions();
    let mut session_states = Vec::new();

    for session in &sessions {
        let (fd, pid) = session.with_terminal(|term| {
            let fd = term.pty().map(|p| p.raw_fd()).unwrap_or(-1);
            let pid = term.child_pid().unwrap_or(-1);
            (fd, pid)
        });

        if fd < 0 || pid < 0 {
            log::warn!(
                "Skipping session {} (no valid FD/PID: fd={}, pid={})",
                session.id,
                fd,
                pid
            );
            continue;
        }

        let (cols, rows) = session.dimensions();
        let custom_title = session.custom_title();

        session_states.push(RelaunchSessionState {
            session_id: session.id.clone(),
            master_fd: fd,
            child_pid: pid,
            cols,
            rows,
            custom_title,
            scrollback_lines,
        });
    }

    RelaunchState {
        sessions: session_states,
        socket_path: socket_path.to_string(),
        scrollback_lines,
    }
}

/// Write relaunch state to a temp file and return the path.
pub fn write_relaunch_state(state: &RelaunchState) -> std::io::Result<std::path::PathBuf> {
    let uid = unsafe { libc::getuid() };
    let path = std::path::PathBuf::from(format!("/tmp/ctermd_relaunch_{}.json", uid));

    let json = serde_json::to_string_pretty(state).map_err(std::io::Error::other)?;

    std::fs::write(&path, &json)?;

    // Set permissions to user-only
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(path)
}

/// Clear FD_CLOEXEC on a file descriptor so it survives exec().
#[cfg(unix)]
fn clear_cloexec(fd: i32) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let ret = unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) };
    if ret < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Perform the relaunch: collect state, clear CLOEXEC, exec().
///
/// This function does not return on success (the process is replaced).
/// On failure, it returns an error.
#[cfg(unix)]
pub fn perform_relaunch(
    session_manager: &Arc<SessionManager>,
    socket_path: &str,
    scrollback_lines: usize,
    binary_path: Option<&str>,
) -> Result<(), String> {
    let state = collect_relaunch_state(session_manager, socket_path, scrollback_lines);

    if state.sessions.is_empty() {
        return Err("No sessions to preserve".to_string());
    }

    log::info!("Relaunch: preserving {} sessions", state.sessions.len());

    // Write state file
    let state_path =
        write_relaunch_state(&state).map_err(|e| format!("Failed to write state: {}", e))?;

    // Clear CLOEXEC on all PTY master FDs so they survive exec
    for s in &state.sessions {
        if let Err(e) = clear_cloexec(s.master_fd) {
            log::error!("Failed to clear CLOEXEC on fd {}: {}", s.master_fd, e);
            // Clean up and abort
            let _ = std::fs::remove_file(&state_path);
            return Err(format!(
                "Failed to clear CLOEXEC on fd {}: {}",
                s.master_fd, e
            ));
        }
        log::info!(
            "Cleared CLOEXEC on fd {} (session {}, pid {})",
            s.master_fd,
            s.session_id,
            s.child_pid
        );
    }

    // Determine the binary to exec
    let binary = if let Some(path) = binary_path {
        std::path::PathBuf::from(path)
    } else {
        std::env::current_exe().map_err(|e| format!("Failed to get current exe: {}", e))?
    };

    log::info!("Exec-ing into: {}", binary.display());

    // Remove the socket file so the new process can bind to it
    let _ = std::fs::remove_file(socket_path);

    // Also remove the PID file
    let _ = std::fs::remove_file(crate::cli::pid_file_path());

    // Build argv: binary --foreground --relaunch-state <path> --listen <socket_path>
    let binary_cstr = std::ffi::CString::new(binary.to_string_lossy().as_bytes())
        .map_err(|e| format!("Invalid binary path: {}", e))?;
    let state_path_str = state_path.to_string_lossy().to_string();
    let args = [
        binary_cstr.clone(),
        std::ffi::CString::new("--foreground").unwrap(),
        std::ffi::CString::new("--relaunch-state").unwrap(),
        std::ffi::CString::new(state_path_str.as_bytes()).unwrap(),
        std::ffi::CString::new("--listen").unwrap(),
        std::ffi::CString::new(socket_path.as_bytes()).unwrap(),
        std::ffi::CString::new("--scrollback").unwrap(),
        std::ffi::CString::new(scrollback_lines.to_string().as_bytes()).unwrap(),
    ];
    let mut arg_ptrs: Vec<*const libc::c_char> = args.iter().map(|a| a.as_ptr()).collect();
    arg_ptrs.push(std::ptr::null()); // NULL terminator required by execv

    // exec replaces the current process — does not return on success
    unsafe {
        libc::execv(binary_cstr.as_ptr(), arg_ptrs.as_ptr());
    }

    // If we get here, exec failed
    let err = std::io::Error::last_os_error();
    Err(format!("execv failed: {}", err))
}

/// Read relaunch state from a file and delete it.
pub fn read_relaunch_state(path: &str) -> Result<RelaunchState, String> {
    let json =
        std::fs::read_to_string(path).map_err(|e| format!("Failed to read state file: {}", e))?;
    let state: RelaunchState =
        serde_json::from_str(&json).map_err(|e| format!("Failed to parse state file: {}", e))?;

    // Delete the state file
    let _ = std::fs::remove_file(path);

    Ok(state)
}
