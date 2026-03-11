//! Daemon relaunch (exec-in-place) for seamless upgrades
//!
//! When a relaunch is requested, the daemon:
//! 1. Serializes session state (FDs, PIDs, screen snapshots) to a temp directory
//! 2. Clears FD_CLOEXEC on all PTY master FDs so they survive exec
//! 3. exec()s the new (or same) binary with `--relaunch-state <path>`
//! 4. The new process reads the state, reconstructs sessions from the
//!    preserved FDs, and resumes serving on the same socket path.
//!
//! Screen snapshots are written as separate binary protobuf files to avoid
//! bloating the JSON metadata (scrollback can be many megabytes).

#[cfg(unix)]
use crate::session::SessionManager;
use prost::Message;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::Arc;

/// Serialized state for a single session (JSON metadata only, screen data is separate)
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

/// Full relaunch state written to state.json
#[derive(Serialize, Deserialize, Debug)]
pub struct RelaunchState {
    pub sessions: Vec<RelaunchSessionState>,
    pub socket_path: String,
    pub scrollback_lines: usize,
}

/// Collect relaunch state and write it to a temp directory.
///
/// Creates `<dir>/state.json` with metadata and `<dir>/<session_id>.screen`
/// with binary protobuf screen snapshots. Returns the directory path.
#[cfg(unix)]
pub fn collect_and_write_relaunch_state(
    session_manager: &Arc<SessionManager>,
    socket_path: &str,
    scrollback_lines: usize,
) -> Result<PathBuf, String> {
    use cterm_proto::convert::screen::screen_to_proto;

    let uid = unsafe { libc::getuid() };
    let dir = PathBuf::from(format!("/tmp/ctermd_relaunch_{}", uid));

    // Create the directory (remove stale one if present)
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create state dir: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok();
    }

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

        // Write screen snapshot as binary protobuf
        session.with_terminal(|term| {
            let screen_proto = screen_to_proto(term.screen(), true);
            let mut buf = Vec::new();
            if screen_proto.encode(&mut buf).is_ok() && !buf.is_empty() {
                let screen_path = dir.join(format!("{}.screen", session.id));
                if let Err(e) = std::fs::write(&screen_path, &buf) {
                    log::warn!("Failed to write screen for session {}: {}", session.id, e);
                }
            }
        });

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

    let state = RelaunchState {
        sessions: session_states,
        socket_path: socket_path.to_string(),
        scrollback_lines,
    };

    let json = serde_json::to_string(&state).map_err(|e| format!("Failed to serialize: {}", e))?;
    std::fs::write(dir.join("state.json"), json)
        .map_err(|e| format!("Failed to write state.json: {}", e))?;

    Ok(dir)
}

/// Read a session's screen snapshot from a binary protobuf file.
pub fn read_screen_snapshot(
    state_dir: &Path,
    session_id: &str,
) -> Option<cterm_proto::proto::GetScreenResponse> {
    let path = state_dir.join(format!("{}.screen", session_id));
    let bytes = std::fs::read(&path).ok()?;
    let _ = std::fs::remove_file(&path); // clean up
    cterm_proto::proto::GetScreenResponse::decode(bytes.as_slice()).ok()
}

/// Read relaunch state from the state directory. Deletes state.json but
/// leaves screen files for per-session reading.
pub fn read_relaunch_state(state_dir: &str) -> Result<(RelaunchState, PathBuf), String> {
    let dir = PathBuf::from(state_dir);
    let json_path = dir.join("state.json");
    let json = std::fs::read_to_string(&json_path)
        .map_err(|e| format!("Failed to read state.json: {}", e))?;
    let state: RelaunchState =
        serde_json::from_str(&json).map_err(|e| format!("Failed to parse state: {}", e))?;

    let _ = std::fs::remove_file(&json_path);

    Ok((state, dir))
}

/// Clean up the relaunch state directory (call after all sessions restored).
pub fn cleanup_state_dir(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
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
    let state_dir =
        collect_and_write_relaunch_state(session_manager, socket_path, scrollback_lines)?;

    // Read back the state to get session list for CLOEXEC clearing
    let json_path = state_dir.join("state.json");
    let json = std::fs::read_to_string(&json_path)
        .map_err(|e| format!("Failed to re-read state: {}", e))?;
    let state: RelaunchState =
        serde_json::from_str(&json).map_err(|e| format!("Failed to parse state: {}", e))?;

    if state.sessions.is_empty() {
        let _ = std::fs::remove_dir_all(&state_dir);
        return Err("No sessions to preserve".to_string());
    }

    log::info!("Relaunch: preserving {} sessions", state.sessions.len());

    // Clear CLOEXEC on all PTY master FDs so they survive exec
    for s in &state.sessions {
        if let Err(e) = clear_cloexec(s.master_fd) {
            log::error!("Failed to clear CLOEXEC on fd {}: {}", s.master_fd, e);
            let _ = std::fs::remove_dir_all(&state_dir);
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
        PathBuf::from(path)
    } else {
        std::env::current_exe().map_err(|e| format!("Failed to get current exe: {}", e))?
    };

    log::info!("Exec-ing into: {}", binary.display());

    // Remove the socket file so the new process can bind to it
    let _ = std::fs::remove_file(socket_path);

    // Also remove the PID file
    let _ = std::fs::remove_file(crate::cli::pid_file_path());

    // Build argv: binary --foreground --relaunch-state <dir> --listen <socket_path>
    let binary_cstr = std::ffi::CString::new(binary.to_string_lossy().as_bytes())
        .map_err(|e| format!("Invalid binary path: {}", e))?;
    let state_dir_str = state_dir.to_string_lossy().to_string();
    let args = [
        binary_cstr.clone(),
        std::ffi::CString::new("--foreground").unwrap(),
        std::ffi::CString::new("--relaunch-state").unwrap(),
        std::ffi::CString::new(state_dir_str.as_bytes()).unwrap(),
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
