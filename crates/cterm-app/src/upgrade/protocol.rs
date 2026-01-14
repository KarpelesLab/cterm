//! Upgrade protocol - handles sending and receiving upgrade state
//!
//! This module provides the core protocol for seamless upgrades:
//! - Sender side: serializes state and sends to new process via inherited FD
//! - Receiver side: deserializes state and reconstructs terminals

#[cfg(unix)]
use cterm_core::fd_passing;

use std::io;
#[cfg(unix)]
use std::path::Path;

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::process::Command;

use super::state::UpgradeState;

/// Maximum size of the state buffer (64MB)
pub const MAX_STATE_SIZE: usize = 64 * 1024 * 1024;

/// Maximum number of file descriptors to transfer
pub const MAX_FDS: usize = 256;

/// Errors that can occur during upgrade
#[derive(Debug, thiserror::Error)]
pub enum UpgradeError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),

    #[error("Process spawn error: {0}")]
    Spawn(String),

    #[error("Socket error: {0}")]
    Socket(String),

    #[error("Acknowledgment timeout")]
    AckTimeout,

    #[error("Too many file descriptors: {0} (max: {1})")]
    TooManyFds(usize, usize),
}

/// Execute an upgrade by sending state to a new process
///
/// # Arguments
/// * `new_binary` - Path to the new binary to execute
/// * `state` - The upgrade state to transfer
/// * `fds` - File descriptors to pass (PTY master FDs)
///
/// # Returns
/// Returns Ok(()) if the upgrade was successful (at which point this process should exit)
#[cfg(unix)]
pub fn execute_upgrade(
    new_binary: &Path,
    state: &UpgradeState,
    fds: &[RawFd],
) -> Result<(), UpgradeError> {
    use std::io::Read;

    if fds.len() > MAX_FDS {
        return Err(UpgradeError::TooManyFds(fds.len(), MAX_FDS));
    }

    // Create a socketpair for communication
    let (parent_sock, child_sock) = UnixStream::pair()
        .map_err(|e| UpgradeError::Socket(format!("Failed to create socketpair: {}", e)))?;

    // Get the raw FD for the child socket - this will be inherited
    use std::os::unix::io::AsRawFd;
    let child_fd = child_sock.as_raw_fd();

    log::info!("Created socketpair, child FD: {}", child_fd);

    // Serialize the state
    let state_bytes =
        bincode::serialize(state).map_err(|e| UpgradeError::Serialization(e.to_string()))?;

    log::info!(
        "State serialized: {} bytes, {} FDs",
        state_bytes.len(),
        fds.len()
    );

    // Spawn the new process with the child socket FD inherited
    // We need to keep child_sock alive until after spawn, and use pre_exec to
    // prevent it from being closed
    let child_fd_for_closure = child_fd;
    let child = unsafe {
        Command::new(new_binary)
            .arg("--upgrade-receiver")
            .arg(child_fd.to_string())
            .pre_exec(move || {
                // Clear the close-on-exec flag for the child socket FD
                // so it gets inherited by the child process
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
            .map_err(|e| UpgradeError::Spawn(e.to_string()))?
    };

    log::info!("New process spawned with PID: {}", child.id());

    // Close our copy of the child socket - child has its own now
    drop(child_sock);

    // Send the state and FDs over the parent socket
    fd_passing::send_fds(&parent_sock, fds, &state_bytes)?;

    log::info!("State and FDs sent");

    // Wait for acknowledgment
    let mut stream = parent_sock;
    let mut ack = [0u8; 1];
    stream
        .read_exact(&mut ack)
        .map_err(|_| UpgradeError::AckTimeout)?;

    if ack[0] != 1 {
        return Err(UpgradeError::Socket("Invalid acknowledgment".to_string()));
    }

    log::info!("Acknowledgment received, upgrade successful");

    Ok(())
}

/// Receive upgrade state from the old process
///
/// This is called by the new process when started with --upgrade-receiver
///
/// # Arguments
/// * `fd` - The inherited file descriptor to read from
///
/// # Returns
/// The upgrade state and file descriptors received
#[cfg(unix)]
pub fn receive_upgrade(fd: RawFd) -> Result<(UpgradeState, Vec<RawFd>), UpgradeError> {
    use std::io::{Read, Write};

    log::info!("Receiving upgrade state from FD {}", fd);

    // Create a UnixStream from the inherited FD
    let mut stream = unsafe { UnixStream::from_raw_fd(fd) };

    // Receive state and FDs
    let mut buf = vec![0u8; MAX_STATE_SIZE];
    let (fds, data_len) = fd_passing::recv_fds(&stream, MAX_FDS, &mut buf)?;

    log::info!(
        "Received {} bytes of state data and {} FDs",
        data_len,
        fds.len()
    );

    // Deserialize the state
    let state: UpgradeState = bincode::deserialize(&buf[..data_len])
        .map_err(|e| UpgradeError::Deserialization(e.to_string()))?;

    log::info!(
        "State deserialized: format_version={}, cterm_version={}, windows={}",
        state.format_version,
        state.cterm_version,
        state.windows.len()
    );

    // Send acknowledgment that we received the data
    stream.write_all(&[1])?;
    stream.flush()?;

    log::info!("Acknowledgment sent, waiting for parent to exit...");

    // Wait for parent to close its end of the socketpair (EOF)
    // This ensures parent has fully exited and released DBus before we start GTK
    let mut buf = [0u8; 1];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break, // EOF - parent closed its end
            Ok(_) => continue, // Ignore any data
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break, // Any other error, assume parent is gone
        }
    }

    log::info!("Parent exited, proceeding with GTK startup");

    Ok((state, fds))
}

#[cfg(test)]
mod tests {
    use crate::upgrade::state::*;

    #[test]
    fn test_state_serialization_roundtrip() {
        let mut state = UpgradeState::new("0.1.0");

        let mut window = WindowUpgradeState::new();
        window.width = 1024;
        window.height = 768;
        state.windows.push(window);

        let bytes = bincode::serialize(&state).expect("Serialize failed");
        let restored: UpgradeState = bincode::deserialize(&bytes).expect("Deserialize failed");

        assert_eq!(restored.windows.len(), 1);
        assert_eq!(restored.windows[0].width, 1024);
    }
}
