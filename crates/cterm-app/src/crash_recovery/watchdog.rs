//! Watchdog process for crash recovery
//!
//! The watchdog is a lightweight parent process that:
//! 1. Spawns the main cterm UI process
//! 2. Receives PTY file descriptors from the child via Unix socket
//! 3. Monitors the child for crashes
//! 4. On crash, relaunches and passes the saved FDs to the new process

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command};

use cterm_core::fd_passing;

use super::state::write_crash_marker;

/// Errors from the watchdog
#[derive(Debug, thiserror::Error)]
pub enum WatchdogError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Failed to spawn child: {0}")]
    Spawn(String),

    #[error("Socket error: {0}")]
    Socket(String),
}

/// Message types sent between watchdog and supervised process
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogMessage {
    /// Register a new PTY FD (child sends FD, watchdog responds with u64 ID)
    RegisterFd = 1,
    /// Unregister a PTY FD by ID (child sends u64 ID)
    UnregisterFd = 2,
    /// Graceful shutdown (no restart on exit)
    Shutdown = 3,
    /// Heartbeat
    Heartbeat = 4,
    /// Registration response (watchdog sends u64 ID back)
    RegisteredFd = 5,
}

/// Held FD info
struct HeldFd {
    fd: RawFd,
    /// Child PID that registered this FD
    child_pid: i32,
}

/// Run the watchdog process
///
/// This function:
/// 1. Creates a socket pair for communication
/// 2. Spawns the supervised child process
/// 3. Receives and stores PTY FDs from the child
/// 4. Monitors for crashes and relaunches if needed
/// 5. On crash, passes held FDs to the new process
///
/// Returns when graceful shutdown is requested or max restarts exceeded.
pub fn run_watchdog(binary_path: &Path, args: &[String]) -> Result<i32, WatchdogError> {
    let max_restarts = 5;
    let mut restart_count = 0;
    let mut graceful_shutdown = false;

    // Track FDs across restarts
    let mut pty_fds: HashMap<u64, HeldFd> = HashMap::new();
    let mut next_fd_id: u64 = 1; // Start at 1 so 0 can mean "invalid"

    loop {
        // Create socket pair for communication
        let (watchdog_sock, child_sock) = UnixStream::pair()
            .map_err(|e| WatchdogError::Socket(format!("Failed to create socketpair: {}", e)))?;

        let child_fd = child_sock.as_raw_fd();

        // Determine if this is a crash recovery restart
        let is_recovery = restart_count > 0 && !pty_fds.is_empty();

        // Build args - pass the watchdog socket FD
        let mut child_args: Vec<String> = args
            .iter()
            .filter(|a| !a.starts_with("--supervised") && !a.starts_with("--crash-recovery"))
            .cloned()
            .collect();

        if is_recovery {
            child_args.push("--crash-recovery".to_string());
        } else {
            child_args.push("--supervised".to_string());
        }
        child_args.push(child_fd.to_string());

        // Spawn child process
        let child_fd_for_closure = child_fd;
        let mut child = unsafe {
            Command::new(binary_path)
                .args(&child_args)
                .pre_exec(move || {
                    // Clear close-on-exec for the socket FD
                    let flags = libc::fcntl(child_fd_for_closure, libc::F_GETFD);
                    if flags != -1 {
                        libc::fcntl(
                            child_fd_for_closure,
                            libc::F_SETFD,
                            flags & !libc::FD_CLOEXEC,
                        );
                    }
                    Ok(())
                })
                .spawn()
                .map_err(|e| WatchdogError::Spawn(e.to_string()))?
        };

        let child_pid = child.id() as i32;

        log::info!(
            "Watchdog: spawned child PID {} (restart count: {}, recovery: {}, held FDs: {})",
            child_pid,
            restart_count,
            is_recovery,
            pty_fds.len()
        );

        // Close our copy of the child socket
        drop(child_sock);

        // If this is a recovery, send the held FDs to the new process
        if is_recovery {
            if let Err(e) = send_recovery_fds(&watchdog_sock, &pty_fds) {
                log::error!("Watchdog: failed to send recovery FDs: {}", e);
            } else {
                log::info!("Watchdog: sent {} FDs to recovered process", pty_fds.len());
            }
        }

        // Monitor loop
        let exit_status = monitor_child(
            &mut child,
            &watchdog_sock,
            &mut pty_fds,
            &mut next_fd_id,
            &mut graceful_shutdown,
            child_pid,
        )?;

        if graceful_shutdown {
            log::info!("Watchdog: graceful shutdown requested");
            // Close all PTY FDs
            for (_, held) in pty_fds.drain() {
                unsafe { libc::close(held.fd) };
            }
            return Ok(exit_status.unwrap_or(0));
        }

        // Check if it was a crash (signal) vs normal exit
        let crashed = exit_status.is_none() || exit_status.unwrap() != 0;

        if crashed && !pty_fds.is_empty() {
            restart_count += 1;
            log::warn!(
                "Watchdog: child crashed with status {:?}, restart {}/{} (preserving {} FDs)",
                exit_status,
                restart_count,
                max_restarts,
                pty_fds.len()
            );

            if restart_count > max_restarts {
                log::error!("Watchdog: max restarts exceeded, giving up");
                for (_, held) in pty_fds.drain() {
                    unsafe { libc::close(held.fd) };
                }
                return Ok(1);
            }

            // Write crash marker for the new process
            let _ = write_crash_marker(exit_status.unwrap_or(-1));

            // Small delay before restart
            std::thread::sleep(std::time::Duration::from_millis(100));

            // Continue to next iteration to restart with FD recovery
        } else if crashed {
            // Crashed but no FDs to recover - still restart but no recovery mode
            restart_count += 1;
            log::warn!(
                "Watchdog: child crashed with status {:?}, restart {}/{} (no FDs to recover)",
                exit_status,
                restart_count,
                max_restarts
            );

            if restart_count > max_restarts {
                log::error!("Watchdog: max restarts exceeded, giving up");
                return Ok(1);
            }

            let _ = write_crash_marker(exit_status.unwrap_or(-1));
            std::thread::sleep(std::time::Duration::from_millis(100));
        } else {
            log::info!("Watchdog: child exited normally");
            for (_, held) in pty_fds.drain() {
                unsafe { libc::close(held.fd) };
            }
            return Ok(0);
        }
    }
}

/// Send recovery FDs to the new process
fn send_recovery_fds(sock: &UnixStream, pty_fds: &HashMap<u64, HeldFd>) -> io::Result<()> {
    // Build list of (id, fd, pid) tuples
    let fds: Vec<RawFd> = pty_fds.values().map(|h| h.fd).collect();
    let entries: Vec<(u64, i32)> = pty_fds
        .iter()
        .map(|(&id, h)| (id, h.child_pid))
        .collect();

    // Send count first, then id + pid pairs
    let count = fds.len() as u32;
    let mut header = Vec::with_capacity(4 + entries.len() * 12);
    header.extend_from_slice(&count.to_le_bytes());
    for (id, pid) in &entries {
        header.extend_from_slice(&id.to_le_bytes());
        header.extend_from_slice(&pid.to_le_bytes());
    }

    // Send header and FDs together
    fd_passing::send_fds(sock, &fds, &header)?;

    Ok(())
}

/// Check if socket has data available using poll
fn socket_has_data(sock: &UnixStream, timeout_ms: i32) -> io::Result<bool> {
    let mut pollfd = libc::pollfd {
        fd: sock.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };

    let ret = unsafe { libc::poll(&mut pollfd, 1, timeout_ms) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(ret > 0 && (pollfd.revents & libc::POLLIN) != 0)
}

/// Monitor the child process and handle messages
fn monitor_child(
    child: &mut Child,
    sock: &UnixStream,
    pty_fds: &mut HashMap<u64, HeldFd>,
    next_fd_id: &mut u64,
    graceful_shutdown: &mut bool,
    child_pid: i32,
) -> Result<Option<i32>, WatchdogError> {
    let mut buf = [0u8; 1024];

    // Keep socket in blocking mode for recv_fds to work properly
    sock.set_nonblocking(false)?;

    loop {
        // Check if child has exited
        match child.try_wait()? {
            Some(status) => {
                return Ok(status.code());
            }
            None => {}
        }

        // Use poll to check if data is available (with 50ms timeout)
        match socket_has_data(sock, 50) {
            Ok(false) => {
                // No data available, continue loop to check child status
                continue;
            }
            Ok(true) => {
                // Data available, receive it
            }
            Err(e) => {
                return Err(WatchdogError::Io(e));
            }
        }

        // Receive message with potential FDs
        // We must use recv_fds for ALL reads because SCM_RIGHTS ancillary data
        // is attached to the message - a regular read() would discard any FDs.
        match fd_passing::recv_fds(sock, 16, &mut buf) {
            Ok((fds, data_len)) => {
                if data_len == 0 && fds.is_empty() {
                    // Child closed socket, wait for exit
                    let status = child.wait()?;
                    return Ok(status.code());
                }

                // Process message
                if data_len >= 1 {
                    match buf[0] {
                        1 => {
                            // RegisterFd - FD should be in the fds we just received
                            if let Some(&fd) = fds.first() {
                                let id = *next_fd_id;
                                *next_fd_id += 1;
                                pty_fds.insert(id, HeldFd { fd, child_pid });
                                log::debug!(
                                    "Watchdog: registered FD {} with id {} (total: {})",
                                    fd,
                                    id,
                                    pty_fds.len()
                                );

                                // Send ID back to child using same protocol
                                let mut response = [0u8; 9];
                                response[0] = WatchdogMessage::RegisteredFd as u8;
                                response[1..9].copy_from_slice(&id.to_le_bytes());
                                let _ = fd_passing::send_fds(sock, &[], &response);
                            } else {
                                log::error!("Watchdog: RegisterFd message but no FD received");
                            }
                        }
                        2 => {
                            // UnregisterFd - close the FD
                            if data_len >= 9 {
                                let id = u64::from_le_bytes(buf[1..9].try_into().unwrap());
                                if let Some(held) = pty_fds.remove(&id) {
                                    unsafe { libc::close(held.fd) };
                                    log::debug!(
                                        "Watchdog: unregistered FD id {} (remaining: {})",
                                        id,
                                        pty_fds.len()
                                    );
                                }
                            }
                            // Close any unexpected FDs that came with this message
                            for &fd in &fds {
                                unsafe { libc::close(fd) };
                            }
                        }
                        3 => {
                            // Shutdown
                            *graceful_shutdown = true;
                            log::debug!("Watchdog: shutdown requested");
                            // Close any unexpected FDs
                            for &fd in &fds {
                                unsafe { libc::close(fd) };
                            }
                        }
                        4 => {
                            // Heartbeat
                            log::trace!("Watchdog: heartbeat from child");
                            // Close any unexpected FDs
                            for &fd in &fds {
                                unsafe { libc::close(fd) };
                            }
                        }
                        _ => {
                            // Close any unexpected FDs
                            for &fd in &fds {
                                unsafe { libc::close(fd) };
                            }
                        }
                    }
                }
            }
            Err(e) => {
                // On EOF (socket closed), wait for child exit
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    let status = child.wait()?;
                    return Ok(status.code());
                }
                return Err(WatchdogError::Io(e));
            }
        }
    }
}

/// Send a PTY FD to the watchdog for safekeeping, returns the assigned ID
pub fn register_fd_with_watchdog(watchdog_fd: RawFd, pty_fd: RawFd) -> io::Result<u64> {
    let sock = unsafe { UnixStream::from_raw_fd(watchdog_fd) };

    // Send register message with FD (using same protocol as upgrade)
    let msg = [WatchdogMessage::RegisterFd as u8];
    fd_passing::send_fds(&sock, &[pty_fd], &msg)?;

    // Wait for response with ID (using same protocol)
    let mut response = [0u8; 64];
    let (_, data_len) = fd_passing::recv_fds(&sock, 0, &mut response)?;

    if data_len < 9 || response[0] != WatchdogMessage::RegisteredFd as u8 {
        // Don't close the socket - it's borrowed
        std::mem::forget(sock);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid response from watchdog",
        ));
    }

    let id = u64::from_le_bytes(response[1..9].try_into().unwrap());

    // Don't close the socket - it's borrowed
    std::mem::forget(sock);

    Ok(id)
}

/// Tell watchdog to close a PTY FD (when tab is closed)
pub fn unregister_fd_with_watchdog(watchdog_fd: RawFd, fd_id: u64) -> io::Result<()> {
    let sock = unsafe { UnixStream::from_raw_fd(watchdog_fd) };

    let mut msg = [0u8; 9];
    msg[0] = WatchdogMessage::UnregisterFd as u8;
    msg[1..9].copy_from_slice(&fd_id.to_le_bytes());

    // Use send_fds with empty FDs to match the protocol
    fd_passing::send_fds(&sock, &[], &msg)?;

    // Don't close the socket - it's borrowed
    std::mem::forget(sock);

    Ok(())
}

/// Tell watchdog we're shutting down gracefully
pub fn notify_watchdog_shutdown(watchdog_fd: RawFd) -> io::Result<()> {
    let sock = unsafe { UnixStream::from_raw_fd(watchdog_fd) };

    // Use send_fds with empty FDs to match the protocol
    fd_passing::send_fds(&sock, &[], &[WatchdogMessage::Shutdown as u8])?;

    // Don't close the socket - it's borrowed
    std::mem::forget(sock);

    Ok(())
}

/// Recovery FD info
pub struct RecoveredFd {
    /// Watchdog-assigned ID
    pub id: u64,
    /// PTY master file descriptor
    pub fd: RawFd,
    /// Child process ID
    pub child_pid: i32,
}

/// Receive recovery FDs from watchdog (called on crash recovery startup)
pub fn receive_recovery_fds(watchdog_fd: RawFd) -> io::Result<Vec<RecoveredFd>> {
    let sock = unsafe { UnixStream::from_raw_fd(watchdog_fd) };

    // Receive the header and FDs
    let mut buf = [0u8; 4096];
    let (fds, data_len) = fd_passing::recv_fds(&sock, 256, &mut buf)?;

    if data_len < 4 {
        std::mem::forget(sock);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid recovery data",
        ));
    }

    let count = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;

    // Each entry is 8 bytes (id) + 4 bytes (pid) = 12 bytes
    if count != fds.len() || data_len < 4 + count * 12 {
        std::mem::forget(sock);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "FD count mismatch: expected {} FDs with {} bytes, got {} FDs with {} bytes",
                count,
                4 + count * 12,
                fds.len(),
                data_len
            ),
        ));
    }

    // Parse IDs and PIDs
    let mut result = Vec::with_capacity(count);
    for i in 0..count {
        let offset = 4 + i * 12;
        let id = u64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap());
        let child_pid = i32::from_le_bytes(buf[offset + 8..offset + 12].try_into().unwrap());
        result.push(RecoveredFd {
            id,
            fd: fds[i],
            child_pid,
        });
    }

    log::info!("Received {} recovery FDs from watchdog", result.len());

    // Don't close the socket - keep it for ongoing communication
    std::mem::forget(sock);

    Ok(result)
}
