//! Upgrade protocol - handles sending and receiving upgrade state
//!
//! This module provides the core protocol for seamless upgrades:
//! - Sender side: serializes state and sends to new process via inherited FD/handle
//! - Receiver side: deserializes state and reconstructs terminals
//!
//! On Unix: Uses socketpair + SCM_RIGHTS for FD passing
//! On Windows: Uses STARTUPINFOEX + PROC_THREAD_ATTRIBUTE_HANDLE_LIST for handle inheritance

#[cfg(unix)]
use cterm_core::fd_passing;

use std::io;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::io::{FromRawFd, RawFd};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(unix)]
use std::process::Command;

#[cfg(windows)]
use std::os::windows::io::RawHandle;

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

// ============================================================================
// Windows Implementation
// ============================================================================

/// Windows handle info for upgrade transfer
#[cfg(windows)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HandleInfo {
    /// Pseudo console handle value (as usize for serialization)
    pub hpc: usize,
    /// Read pipe handle value
    pub read_pipe: usize,
    /// Write pipe handle value
    pub write_pipe: usize,
    /// Process handle value
    pub process_handle: usize,
    /// Process ID
    pub process_id: u32,
}

/// Data sent over the upgrade pipe on Windows
#[cfg(windows)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WindowsUpgradeData {
    /// Handle information for each PTY (indexed by pty_fd_index in TabUpgradeState)
    pub handles: Vec<HandleInfo>,
    /// Serialized UpgradeState
    pub state_bytes: Vec<u8>,
}

/// Execute an upgrade by sending state to a new process (Windows)
///
/// # Arguments
/// * `new_binary` - Path to the new binary to execute
/// * `state` - The upgrade state to transfer
/// * `handles` - PTY handles to pass (tuples of hpc, read_pipe, write_pipe, process_handle, process_id)
///
/// # Returns
/// Returns Ok(()) if the upgrade was successful
#[cfg(windows)]
pub fn execute_upgrade(
    new_binary: &Path,
    state: &UpgradeState,
    handles: &[(RawHandle, RawHandle, RawHandle, RawHandle, u32)],
) -> Result<(), UpgradeError> {
    use std::ffi::OsStr;
    use std::io::{Read, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::ptr;
    use winapi::shared::minwindef::{DWORD, FALSE, TRUE};
    use winapi::um::handleapi::{CloseHandle, SetHandleInformation, INVALID_HANDLE_VALUE};
    use winapi::um::minwinbase::SECURITY_ATTRIBUTES;
    use winapi::um::namedpipeapi::CreatePipe;
    use winapi::um::processthreadsapi::{
        CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
        UpdateProcThreadAttribute, PROCESS_INFORMATION,
    };
    use winapi::um::winbase::{EXTENDED_STARTUPINFO_PRESENT, HANDLE_FLAG_INHERIT, STARTUPINFOEXW};
    use winapi::um::winnt::HANDLE;

    const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x00020002;

    if handles.len() > MAX_FDS {
        return Err(UpgradeError::TooManyFds(handles.len(), MAX_FDS));
    }

    // Serialize the state
    let state_bytes =
        bincode::serialize(state).map_err(|e| UpgradeError::Serialization(e.to_string()))?;

    log::info!(
        "State serialized: {} bytes, {} handle sets",
        state_bytes.len(),
        handles.len()
    );

    // Create a pipe for sending upgrade data to the new process
    let mut read_pipe: HANDLE = INVALID_HANDLE_VALUE;
    let mut write_pipe: HANDLE = INVALID_HANDLE_VALUE;

    // Set up security attributes to allow inheritance
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: TRUE,
    };

    let result = unsafe { CreatePipe(&mut read_pipe, &mut write_pipe, &mut sa, 0) };

    if result == FALSE {
        return Err(UpgradeError::Socket(format!(
            "Failed to create pipe: {}",
            std::io::Error::last_os_error()
        )));
    }

    // Make write_pipe non-inheritable (we keep it)
    unsafe {
        SetHandleInformation(write_pipe, HANDLE_FLAG_INHERIT, 0);
    }

    // Collect all handles that need to be inherited
    let mut inheritable_handles: Vec<HANDLE> = Vec::new();

    // Add the read pipe (for receiving upgrade data)
    inheritable_handles.push(read_pipe);

    // Mark all PTY handles as inheritable and add them to the list
    for (hpc, read_h, write_h, process_h, _pid) in handles {
        unsafe {
            // Mark each handle as inheritable
            SetHandleInformation(*hpc as HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
            SetHandleInformation(*read_h as HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
            SetHandleInformation(*write_h as HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
            SetHandleInformation(*process_h as HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
        }
        inheritable_handles.push(*hpc as HANDLE);
        inheritable_handles.push(*read_h as HANDLE);
        inheritable_handles.push(*write_h as HANDLE);
        inheritable_handles.push(*process_h as HANDLE);
    }

    log::info!(
        "Prepared {} handles for inheritance",
        inheritable_handles.len()
    );

    // Set up STARTUPINFOEX with explicit handle list
    let mut attr_list_size: usize = 0;
    unsafe {
        InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut attr_list_size);
    }

    let attr_list = vec![0u8; attr_list_size];
    let attr_list_ptr = attr_list.as_ptr() as *mut _;

    if unsafe { InitializeProcThreadAttributeList(attr_list_ptr, 1, 0, &mut attr_list_size) }
        == FALSE
    {
        unsafe {
            CloseHandle(read_pipe);
            CloseHandle(write_pipe);
        }
        return Err(UpgradeError::Spawn(format!(
            "InitializeProcThreadAttributeList failed: {}",
            std::io::Error::last_os_error()
        )));
    }

    // Add the handle list attribute
    if unsafe {
        UpdateProcThreadAttribute(
            attr_list_ptr,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            inheritable_handles.as_ptr() as *mut _,
            inheritable_handles.len() * std::mem::size_of::<HANDLE>(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    } == FALSE
    {
        unsafe {
            DeleteProcThreadAttributeList(attr_list_ptr);
            CloseHandle(read_pipe);
            CloseHandle(write_pipe);
        }
        return Err(UpgradeError::Spawn(format!(
            "UpdateProcThreadAttribute failed: {}",
            std::io::Error::last_os_error()
        )));
    }

    let mut startup_info: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup_info.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup_info.lpAttributeList = attr_list_ptr;

    // Build command line with upgrade receiver argument
    // Pass the pipe handle value as the argument
    let cmd_line = format!(
        "\"{}\" --upgrade-receiver {}",
        new_binary.display(),
        read_pipe as usize
    );

    let cmd_wide: Vec<u16> = OsStr::new(&cmd_line)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let mut process_info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    // Create the new process
    let result = unsafe {
        CreateProcessW(
            ptr::null(),
            cmd_wide.as_ptr() as *mut _,
            ptr::null_mut(),
            ptr::null_mut(),
            TRUE, // Inherit handles
            EXTENDED_STARTUPINFO_PRESENT,
            ptr::null_mut(),
            ptr::null(),
            &mut startup_info.StartupInfo,
            &mut process_info,
        )
    };

    unsafe {
        DeleteProcThreadAttributeList(attr_list_ptr);
    }

    if result == FALSE {
        unsafe {
            CloseHandle(read_pipe);
            CloseHandle(write_pipe);
        }
        return Err(UpgradeError::Spawn(format!(
            "CreateProcessW failed: {}",
            std::io::Error::last_os_error()
        )));
    }

    log::info!(
        "New process spawned with PID: {}",
        process_info.dwProcessId
    );

    // Close handle to the new process thread (we don't need it)
    unsafe {
        CloseHandle(process_info.hThread);
    }

    // Close our copy of the read pipe (child has it now)
    unsafe {
        CloseHandle(read_pipe);
    }

    // Build the upgrade data with handle values
    let handle_infos: Vec<HandleInfo> = handles
        .iter()
        .map(|(hpc, read_h, write_h, process_h, pid)| HandleInfo {
            hpc: *hpc as usize,
            read_pipe: *read_h as usize,
            write_pipe: *write_h as usize,
            process_handle: *process_h as usize,
            process_id: *pid,
        })
        .collect();

    let upgrade_data = WindowsUpgradeData {
        handles: handle_infos,
        state_bytes,
    };

    // Serialize and send the upgrade data
    let data_bytes =
        bincode::serialize(&upgrade_data).map_err(|e| UpgradeError::Serialization(e.to_string()))?;

    // Write length prefix then data
    let mut write_file = unsafe { std::fs::File::from_raw_handle(write_pipe as RawHandle) };
    let len_bytes = (data_bytes.len() as u64).to_le_bytes();
    write_file
        .write_all(&len_bytes)
        .map_err(|e| UpgradeError::Io(e))?;
    write_file
        .write_all(&data_bytes)
        .map_err(|e| UpgradeError::Io(e))?;
    write_file.flush().map_err(|e| UpgradeError::Io(e))?;

    log::info!("Upgrade data sent ({} bytes)", data_bytes.len());

    // Wait for acknowledgment
    let mut ack = [0u8; 1];
    write_file
        .read_exact(&mut ack)
        .map_err(|_| UpgradeError::AckTimeout)?;

    if ack[0] != 1 {
        return Err(UpgradeError::Socket("Invalid acknowledgment".to_string()));
    }

    log::info!("Acknowledgment received, upgrade successful");

    // Close the process handle
    unsafe {
        CloseHandle(process_info.hProcess);
    }

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
            Ok(0) => break,    // EOF - parent closed its end
            Ok(_) => continue, // Ignore any data
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break, // Any other error, assume parent is gone
        }
    }

    log::info!("Parent exited, proceeding with GTK startup");

    Ok((state, fds))
}

/// Receive upgrade state from the old process (Windows)
///
/// This is called by the new process when started with --upgrade-receiver
///
/// # Arguments
/// * `handle` - The inherited pipe handle value (as usize from command line)
///
/// # Returns
/// The upgrade state and handle info for PTYs
#[cfg(windows)]
pub fn receive_upgrade(
    handle: usize,
) -> Result<(UpgradeState, Vec<(RawHandle, RawHandle, RawHandle, RawHandle, u32)>), UpgradeError> {
    use std::io::{Read, Write};
    use std::os::windows::io::FromRawHandle;
    use winapi::um::winnt::HANDLE;

    log::info!("Receiving upgrade state from handle {}", handle);

    // Create a File from the inherited handle
    let mut pipe = unsafe { std::fs::File::from_raw_handle(handle as RawHandle) };

    // Read length prefix
    let mut len_bytes = [0u8; 8];
    pipe.read_exact(&mut len_bytes)
        .map_err(|e| UpgradeError::Io(e))?;
    let data_len = u64::from_le_bytes(len_bytes) as usize;

    if data_len > MAX_STATE_SIZE {
        return Err(UpgradeError::Deserialization(format!(
            "Data too large: {} bytes (max: {})",
            data_len, MAX_STATE_SIZE
        )));
    }

    log::info!("Expecting {} bytes of upgrade data", data_len);

    // Read the data
    let mut data_bytes = vec![0u8; data_len];
    pipe.read_exact(&mut data_bytes)
        .map_err(|e| UpgradeError::Io(e))?;

    // Deserialize the Windows upgrade data
    let upgrade_data: WindowsUpgradeData = bincode::deserialize(&data_bytes)
        .map_err(|e| UpgradeError::Deserialization(e.to_string()))?;

    log::info!(
        "Received {} handle sets",
        upgrade_data.handles.len()
    );

    // Deserialize the state
    let state: UpgradeState = bincode::deserialize(&upgrade_data.state_bytes)
        .map_err(|e| UpgradeError::Deserialization(e.to_string()))?;

    log::info!(
        "State deserialized: format_version={}, cterm_version={}, windows={}",
        state.format_version,
        state.cterm_version,
        state.windows.len()
    );

    // Convert handle infos to raw handles
    let handles: Vec<(RawHandle, RawHandle, RawHandle, RawHandle, u32)> = upgrade_data
        .handles
        .iter()
        .map(|h| {
            (
                h.hpc as RawHandle,
                h.read_pipe as RawHandle,
                h.write_pipe as RawHandle,
                h.process_handle as RawHandle,
                h.process_id,
            )
        })
        .collect();

    // Send acknowledgment
    pipe.write_all(&[1]).map_err(|e| UpgradeError::Io(e))?;
    pipe.flush().map_err(|e| UpgradeError::Io(e))?;

    log::info!("Acknowledgment sent, proceeding with startup");

    // On Windows, we don't need to wait for the parent like on Unix
    // The handles are already inherited and the parent can exit

    Ok((state, handles))
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
