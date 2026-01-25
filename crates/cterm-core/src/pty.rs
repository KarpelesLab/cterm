//! Cross-platform PTY handling with raw handle/fd access
//!
//! This module provides native PTY functionality on Unix (using openpty/fork)
//! and Windows (using ConPTY). Raw file descriptors/handles are exposed to
//! enable seamless upgrades via fd passing.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use thiserror::Error;

/// PTY size in rows and columns
#[derive(Debug, Clone, Copy, Default)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
}

/// Errors that can occur with PTY operations
#[derive(Error, Debug)]
pub enum PtyError {
    #[error("Failed to create PTY: {0}")]
    Create(#[source] Box<dyn std::error::Error + Send + Sync>),

    #[error("Failed to spawn process: {0}")]
    Spawn(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("PTY not running")]
    NotRunning,
}

/// PTY configuration
#[derive(Debug, Clone, Default)]
pub struct PtyConfig {
    /// Initial terminal size
    pub size: PtySize,
    /// Shell command to run (None = default shell)
    pub shell: Option<String>,
    /// Arguments to pass to the shell
    pub args: Vec<String>,
    /// Working directory
    pub cwd: Option<PathBuf>,
    /// Environment variables to set
    pub env: Vec<(String, String)>,
}

// ============================================================================
// Unix Implementation
// ============================================================================

#[cfg(unix)]
mod unix {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};

    /// Native PTY with exposed raw file descriptor
    pub struct Pty {
        /// The master PTY file descriptor
        master_fd: RawFd,
        /// File wrapper for the master (for I/O operations)
        master: File,
        /// Child process ID
        child_pid: libc::pid_t,
        /// Cached exit status
        exit_status: Option<i32>,
    }

    impl Pty {
        /// Create a new PTY and spawn the shell
        pub fn new(config: &PtyConfig) -> Result<Self, PtyError> {
            unsafe { Self::create_pty_and_spawn(config) }
        }

        /// Create PTY from an existing file descriptor (for upgrade receiver)
        ///
        /// # Safety
        /// The caller must ensure `fd` is a valid master PTY file descriptor
        /// and `child_pid` is the correct process ID of the child process.
        pub unsafe fn from_raw_fd(fd: RawFd, child_pid: i32) -> Self {
            Self {
                master_fd: fd,
                master: File::from_raw_fd(fd),
                child_pid,
                exit_status: None,
            }
        }

        /// Get the raw file descriptor for passing via SCM_RIGHTS
        pub fn raw_fd(&self) -> RawFd {
            self.master_fd
        }

        /// Get the child process ID
        pub fn child_pid(&self) -> i32 {
            self.child_pid
        }

        /// Duplicate the file descriptor for transfer
        /// Returns a new FD that can be passed to another process
        pub fn dup_fd(&self) -> io::Result<RawFd> {
            let new_fd = unsafe { libc::dup(self.master_fd) };
            if new_fd < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(new_fd)
            }
        }

        /// Write data to the PTY
        pub fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.master.write(data)
        }

        /// Read data from the PTY
        pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.master.read(buf)
        }

        /// Resize the PTY
        pub fn resize(&self, rows: u16, cols: u16) -> io::Result<()> {
            let size = libc::winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };

            let ret = unsafe { libc::ioctl(self.master_fd, libc::TIOCSWINSZ, &size) };
            if ret < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }

        /// Check if the child process is still running
        pub fn is_running(&mut self) -> bool {
            if self.exit_status.is_some() {
                return false;
            }

            let mut status: libc::c_int = 0;
            let ret = unsafe { libc::waitpid(self.child_pid, &mut status, libc::WNOHANG) };

            if ret == self.child_pid {
                // Process has exited
                if libc::WIFEXITED(status) {
                    self.exit_status = Some(libc::WEXITSTATUS(status));
                } else if libc::WIFSIGNALED(status) {
                    self.exit_status = Some(128 + libc::WTERMSIG(status));
                }
                false
            } else {
                true
            }
        }

        /// Wait for the child process to exit
        pub fn wait(&mut self) -> io::Result<i32> {
            if let Some(status) = self.exit_status {
                return Ok(status);
            }

            let mut status: libc::c_int = 0;
            let ret = unsafe { libc::waitpid(self.child_pid, &mut status, 0) };

            if ret < 0 {
                return Err(io::Error::last_os_error());
            }

            let exit_code = if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else if libc::WIFSIGNALED(status) {
                128 + libc::WTERMSIG(status)
            } else {
                -1
            };

            self.exit_status = Some(exit_code);
            Ok(exit_code)
        }

        /// Try to wait for the child process without blocking
        /// Returns Ok(Some(exit_code)) if process exited, Ok(None) if still running
        pub fn try_wait(&mut self) -> io::Result<Option<i32>> {
            if let Some(status) = self.exit_status {
                return Ok(Some(status));
            }

            let mut status: libc::c_int = 0;
            let ret = unsafe { libc::waitpid(self.child_pid, &mut status, libc::WNOHANG) };

            if ret < 0 {
                return Err(io::Error::last_os_error());
            }

            if ret == 0 {
                // Child still running
                return Ok(None);
            }

            let exit_code = if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else if libc::WIFSIGNALED(status) {
                128 + libc::WTERMSIG(status)
            } else {
                -1
            };

            self.exit_status = Some(exit_code);
            Ok(Some(exit_code))
        }

        /// Send a signal to the child process
        pub fn send_signal(&self, signal: i32) -> io::Result<()> {
            let ret = unsafe { libc::kill(self.child_pid, signal) };
            if ret < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }

        /// Get the foreground process group ID of the terminal
        ///
        /// Returns the process group ID of the foreground process, or None if
        /// it cannot be determined.
        pub fn foreground_process_group(&self) -> Option<i32> {
            let pgid = unsafe { libc::tcgetpgrp(self.master_fd) };
            if pgid < 0 {
                None
            } else {
                Some(pgid)
            }
        }

        /// Check if there's a foreground process running (other than the shell)
        ///
        /// Returns true if the foreground process group differs from the shell's
        /// process group, indicating a command is running.
        pub fn has_foreground_process(&self) -> bool {
            if let Some(fg_pgid) = self.foreground_process_group() {
                // The shell's process group is typically the same as its PID
                // If the foreground process group differs, something else is running
                fg_pgid != self.child_pid
            } else {
                false
            }
        }

        /// Get the name of the foreground process (if any)
        ///
        /// Returns the process name if a foreground process is running, None otherwise.
        #[cfg(target_os = "macos")]
        pub fn foreground_process_name(&self) -> Option<String> {
            let fg_pgid = self.foreground_process_group()?;

            // On macOS, use proc_name to get the process name
            let mut name_buf = [0i8; 256];
            let ret = unsafe {
                libc::proc_name(fg_pgid, name_buf.as_mut_ptr() as *mut libc::c_void, 256)
            };

            if ret > 0 {
                let name = unsafe {
                    std::ffi::CStr::from_ptr(name_buf.as_ptr())
                        .to_string_lossy()
                        .into_owned()
                };
                Some(name)
            } else {
                None
            }
        }

        #[cfg(not(target_os = "macos"))]
        pub fn foreground_process_name(&self) -> Option<String> {
            let fg_pgid = self.foreground_process_group()?;

            // On Linux, read from /proc/<pid>/comm
            std::fs::read_to_string(format!("/proc/{}/comm", fg_pgid))
                .ok()
                .map(|s| s.trim().to_string())
        }

        /// Try to clone the reader for concurrent access
        pub fn try_clone_reader(&self) -> io::Result<File> {
            let new_fd = unsafe { libc::dup(self.master_fd) };
            if new_fd < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(unsafe { File::from_raw_fd(new_fd) })
            }
        }

        /// Internal: Create PTY and spawn child process
        unsafe fn create_pty_and_spawn(config: &PtyConfig) -> Result<Self, PtyError> {
            // Open a new PTY pair
            let mut master_fd: libc::c_int = 0;
            let mut slave_fd: libc::c_int = 0;

            let ret = libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            );

            if ret < 0 {
                return Err(PtyError::Create(Box::new(io::Error::last_os_error())));
            }

            // Set the initial window size
            let size = libc::winsize {
                ws_row: config.size.rows,
                ws_col: config.size.cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            libc::ioctl(slave_fd, libc::TIOCSWINSZ, &size);

            // Fork the process
            let pid = libc::fork();

            if pid < 0 {
                libc::close(master_fd);
                libc::close(slave_fd);
                return Err(PtyError::Create(Box::new(io::Error::last_os_error())));
            }

            if pid == 0 {
                // Child process - setup_child never returns (it calls exec or exit)
                Self::setup_child(slave_fd, master_fd, config);
            }

            // Parent process
            libc::close(slave_fd);

            Ok(Self {
                master_fd,
                master: File::from_raw_fd(master_fd),
                child_pid: pid,
                exit_status: None,
            })
        }

        /// Setup the child process (runs in the forked child)
        unsafe fn setup_child(slave_fd: RawFd, master_fd: RawFd, config: &PtyConfig) -> ! {
            // Close the master FD in child
            libc::close(master_fd);

            // Create a new session
            if libc::setsid() < 0 {
                libc::_exit(1);
            }

            // Set the slave as the controlling terminal
            // Cast needed: TIOCSCTTY type differs between Linux and macOS
            if libc::ioctl(slave_fd, libc::TIOCSCTTY as libc::c_ulong, 0) < 0 {
                libc::_exit(1);
            }

            // Duplicate slave to stdin/stdout/stderr
            if libc::dup2(slave_fd, libc::STDIN_FILENO) < 0 {
                libc::_exit(1);
            }
            if libc::dup2(slave_fd, libc::STDOUT_FILENO) < 0 {
                libc::_exit(1);
            }
            if libc::dup2(slave_fd, libc::STDERR_FILENO) < 0 {
                libc::_exit(1);
            }

            // Close the original slave FD if it's not one of the standard FDs
            if slave_fd > libc::STDERR_FILENO {
                libc::close(slave_fd);
            }

            // Change to the working directory if specified
            if let Some(ref cwd) = config.cwd {
                if let Ok(cwd_cstring) = CString::new(cwd.to_string_lossy().as_bytes()) {
                    libc::chdir(cwd_cstring.as_ptr());
                }
            }

            // Set environment variables
            for (key, value) in &config.env {
                if let (Ok(key_c), Ok(value_c)) =
                    (CString::new(key.as_str()), CString::new(value.as_str()))
                {
                    libc::setenv(key_c.as_ptr(), value_c.as_ptr(), 1);
                }
            }

            // Set TERM environment variable
            let term = CString::new("TERM").unwrap();
            let term_value = CString::new("xterm-256color").unwrap();
            libc::setenv(term.as_ptr(), term_value.as_ptr(), 1);

            // Determine the shell to execute
            let shell = config.shell.clone().unwrap_or_else(get_default_shell);

            let shell_cstring = match CString::new(shell.as_str()) {
                Ok(s) => s,
                Err(_) => libc::_exit(1),
            };

            // Build arguments
            let mut args_cstrings: Vec<CString> = Vec::new();

            // Shell name as argv[0]
            let shell_name = shell.rsplit('/').next().unwrap_or(&shell);
            args_cstrings
                .push(CString::new(shell_name).unwrap_or_else(|_| CString::new("sh").unwrap()));

            // Add additional arguments
            for arg in &config.args {
                if let Ok(arg_c) = CString::new(arg.as_str()) {
                    args_cstrings.push(arg_c);
                }
            }

            // Convert to pointer array for execv
            let mut args_ptrs: Vec<*const libc::c_char> =
                args_cstrings.iter().map(|s| s.as_ptr()).collect();
            args_ptrs.push(std::ptr::null());

            // Execute the shell (execvp searches PATH)
            libc::execvp(shell_cstring.as_ptr(), args_ptrs.as_ptr());

            // If exec fails, exit
            libc::_exit(127);
        }
    }

    impl Drop for Pty {
        fn drop(&mut self) {
            // Send SIGHUP to the child process
            let _ = self.send_signal(libc::SIGHUP);
            // Note: We don't close master_fd here because File will do it
        }
    }

    impl AsRawFd for Pty {
        fn as_raw_fd(&self) -> RawFd {
            self.master_fd
        }
    }

    /// Get the default shell for the current user
    fn get_default_shell() -> String {
        // Try to get shell from environment
        if let Ok(shell) = std::env::var("SHELL") {
            return shell;
        }

        // Try to get shell from passwd entry
        unsafe {
            let uid = libc::getuid();
            let passwd = libc::getpwuid(uid);
            if !passwd.is_null() {
                let shell_ptr = (*passwd).pw_shell;
                if !shell_ptr.is_null() {
                    if let Ok(shell) = CStr::from_ptr(shell_ptr).to_str() {
                        return shell.to_string();
                    }
                }
            }
        }

        // Fallback to /bin/sh
        "/bin/sh".to_string()
    }
}

// ============================================================================
// Windows Implementation (ConPTY)
// ============================================================================

#[cfg(windows)]
mod windows {
    use super::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
    use std::ptr;
    use winapi::shared::minwindef::{DWORD, FALSE};
    use winapi::shared::winerror::S_OK;
    use winapi::um::consoleapi::{ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole};
    use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
    use winapi::um::namedpipeapi::CreatePipe;
    use winapi::um::processthreadsapi::{
        CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
        UpdateProcThreadAttribute, PROCESS_INFORMATION,
    };
    use winapi::um::synchapi::WaitForSingleObject;
    use winapi::um::winbase::{
        EXTENDED_STARTUPINFO_PRESENT, INFINITE, STARTUPINFOEXW, WAIT_OBJECT_0,
    };
    use winapi::um::wincon::COORD;
    use winapi::um::winnt::HANDLE;

    const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x00020016;

    /// Windows PTY using ConPTY
    pub struct Pty {
        /// Pseudo console handle
        hpc: HANDLE,
        /// Pipe for reading from PTY
        read_pipe: File,
        /// Pipe for writing to PTY
        write_pipe: File,
        /// Process handle
        process_handle: HANDLE,
        /// Thread handle
        thread_handle: HANDLE,
        /// Process ID
        process_id: u32,
        /// Cached exit status
        exit_status: Option<i32>,
    }

    // SAFETY: Windows HANDLEs are just integer values that can be safely sent between threads.
    // The underlying resources are managed by the Windows kernel and are not tied to any thread.
    unsafe impl Send for Pty {}

    impl Pty {
        /// Create a new PTY and spawn the shell
        pub fn new(config: &PtyConfig) -> Result<Self, PtyError> {
            unsafe { Self::create_conpty(config) }
        }

        /// Create PTY from raw handles (for upgrade receiver)
        ///
        /// # Safety
        /// The caller must ensure all handles are valid.
        pub unsafe fn from_raw_handles(
            hpc: RawHandle,
            read_pipe: RawHandle,
            write_pipe: RawHandle,
            process_handle: RawHandle,
            process_id: u32,
        ) -> Self {
            Self {
                hpc: hpc as HANDLE,
                read_pipe: File::from_raw_handle(read_pipe),
                write_pipe: File::from_raw_handle(write_pipe),
                process_handle: process_handle as HANDLE,
                thread_handle: INVALID_HANDLE_VALUE,
                process_id,
                exit_status: None,
            }
        }

        /// Get the process ID (equivalent to child_pid on Unix)
        pub fn child_pid(&self) -> i32 {
            self.process_id as i32
        }

        /// Write data to the PTY
        pub fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.write_pipe.write(data)
        }

        /// Read data from the PTY
        pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.read_pipe.read(buf)
        }

        /// Resize the PTY
        pub fn resize(&self, rows: u16, cols: u16) -> io::Result<()> {
            let size = COORD {
                X: cols as i16,
                Y: rows as i16,
            };
            let hr = unsafe { ResizePseudoConsole(self.hpc, size) };
            if hr != S_OK {
                Err(io::Error::from_raw_os_error(hr))
            } else {
                Ok(())
            }
        }

        /// Check if the process is still running
        pub fn is_running(&mut self) -> bool {
            if self.exit_status.is_some() {
                return false;
            }

            let result = unsafe { WaitForSingleObject(self.process_handle, 0) };
            if result == WAIT_OBJECT_0 {
                // Process has exited
                let mut exit_code: DWORD = 0;
                unsafe {
                    winapi::um::processthreadsapi::GetExitCodeProcess(
                        self.process_handle,
                        &mut exit_code,
                    );
                }
                self.exit_status = Some(exit_code as i32);
                false
            } else {
                true
            }
        }

        /// Wait for the process to exit
        pub fn wait(&mut self) -> io::Result<i32> {
            if let Some(status) = self.exit_status {
                return Ok(status);
            }

            unsafe { WaitForSingleObject(self.process_handle, INFINITE) };

            let mut exit_code: DWORD = 0;
            unsafe {
                winapi::um::processthreadsapi::GetExitCodeProcess(
                    self.process_handle,
                    &mut exit_code,
                );
            }
            self.exit_status = Some(exit_code as i32);
            Ok(exit_code as i32)
        }

        /// Try to wait for the process to exit without blocking
        /// Returns Ok(Some(exit_code)) if process has exited, Ok(None) if still running
        pub fn try_wait(&mut self) -> io::Result<Option<i32>> {
            if let Some(status) = self.exit_status {
                return Ok(Some(status));
            }

            let result = unsafe { WaitForSingleObject(self.process_handle, 0) };
            if result == WAIT_OBJECT_0 {
                // Process has exited
                let mut exit_code: DWORD = 0;
                unsafe {
                    winapi::um::processthreadsapi::GetExitCodeProcess(
                        self.process_handle,
                        &mut exit_code,
                    );
                }
                self.exit_status = Some(exit_code as i32);
                Ok(Some(exit_code as i32))
            } else {
                // Process still running
                Ok(None)
            }
        }

        /// Send a signal to the process
        pub fn send_signal(&self, signal: i32) -> io::Result<()> {
            // Windows doesn't have Unix signals, handle common cases
            match signal {
                // SIGTERM/SIGKILL - terminate the process
                9 | 15 => {
                    let ret = unsafe {
                        winapi::um::processthreadsapi::TerminateProcess(self.process_handle, 1)
                    };
                    if ret == 0 {
                        Err(io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                }
                // SIGINT - send Ctrl+C
                2 => {
                    // Write Ctrl+C to the PTY
                    let mut write_pipe = &self.write_pipe;
                    write_pipe.write_all(&[0x03])?;
                    Ok(())
                }
                _ => Ok(()), // Ignore other signals
            }
        }

        /// Try to clone the reader for concurrent access
        pub fn try_clone_reader(&self) -> io::Result<File> {
            self.read_pipe.try_clone()
        }

        /// Get all handles needed for upgrade transfer
        ///
        /// Returns (hpc, read_pipe, write_pipe, process_handle, process_id)
        pub fn get_upgrade_handles(&self) -> (RawHandle, RawHandle, RawHandle, RawHandle, u32) {
            (
                self.hpc as RawHandle,
                self.read_pipe.as_raw_handle(),
                self.write_pipe.as_raw_handle(),
                self.process_handle as RawHandle,
                self.process_id,
            )
        }

        /// Duplicate a handle for transfer to another process
        ///
        /// # Arguments
        /// * `handle` - The handle to duplicate
        /// * `target_process` - Handle to the target process (from OpenProcess)
        ///
        /// # Returns
        /// The duplicated handle value in the target process
        pub fn duplicate_handle_to_process(
            handle: RawHandle,
            target_process: HANDLE,
        ) -> io::Result<RawHandle> {
            use winapi::um::handleapi::DuplicateHandle;
            use winapi::um::processthreadsapi::GetCurrentProcess;

            let mut new_handle: HANDLE = INVALID_HANDLE_VALUE;
            let result = unsafe {
                DuplicateHandle(
                    GetCurrentProcess(),
                    handle as HANDLE,
                    target_process,
                    &mut new_handle,
                    0,
                    FALSE,
                    winapi::um::winnt::DUPLICATE_SAME_ACCESS,
                )
            };

            if result == FALSE {
                Err(io::Error::last_os_error())
            } else {
                Ok(new_handle as RawHandle)
            }
        }

        /// Duplicate handles for transfer (for compatibility with Unix API)
        pub fn dup_fd(&self) -> io::Result<RawHandle> {
            // Return the read pipe handle - caller should use get_upgrade_handles() for full transfer
            Ok(self.read_pipe.as_raw_handle())
        }

        /// Get the raw handle (for compatibility)
        pub fn raw_fd(&self) -> RawHandle {
            self.read_pipe.as_raw_handle()
        }

        unsafe fn create_conpty(config: &PtyConfig) -> Result<Self, PtyError> {
            // Create pipes for PTY communication
            let mut read_pipe_pty: HANDLE = INVALID_HANDLE_VALUE;
            let mut write_pipe_pty: HANDLE = INVALID_HANDLE_VALUE;
            let mut read_pipe_process: HANDLE = INVALID_HANDLE_VALUE;
            let mut write_pipe_process: HANDLE = INVALID_HANDLE_VALUE;

            // Create pipes: PTY reads from read_pipe_pty, process writes to write_pipe_process
            if CreatePipe(
                &mut read_pipe_pty,
                &mut write_pipe_process,
                ptr::null_mut(),
                0,
            ) == FALSE
            {
                return Err(PtyError::Create(Box::new(io::Error::last_os_error())));
            }
            if CreatePipe(
                &mut read_pipe_process,
                &mut write_pipe_pty,
                ptr::null_mut(),
                0,
            ) == FALSE
            {
                CloseHandle(read_pipe_pty);
                CloseHandle(write_pipe_process);
                return Err(PtyError::Create(Box::new(io::Error::last_os_error())));
            }

            // Create the pseudo console
            let size = COORD {
                X: config.size.cols as i16,
                Y: config.size.rows as i16,
            };

            let mut hpc: HANDLE = INVALID_HANDLE_VALUE;
            let hr = CreatePseudoConsole(size, read_pipe_pty, write_pipe_pty, 0, &mut hpc);

            // Close the PTY-side pipe handles (the console owns them now)
            CloseHandle(read_pipe_pty);
            CloseHandle(write_pipe_pty);

            if hr != S_OK {
                CloseHandle(read_pipe_process);
                CloseHandle(write_pipe_process);
                return Err(PtyError::Create(Box::new(io::Error::from_raw_os_error(hr))));
            }

            // Prepare startup info with pseudo console
            let mut attr_list_size: usize = 0;
            InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut attr_list_size);

            let attr_list = vec![0u8; attr_list_size];
            let attr_list_ptr = attr_list.as_ptr() as *mut _;

            if InitializeProcThreadAttributeList(attr_list_ptr, 1, 0, &mut attr_list_size) == FALSE
            {
                ClosePseudoConsole(hpc);
                CloseHandle(read_pipe_process);
                CloseHandle(write_pipe_process);
                return Err(PtyError::Create(Box::new(io::Error::last_os_error())));
            }

            if UpdateProcThreadAttribute(
                attr_list_ptr,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                hpc as *mut _,
                std::mem::size_of::<HANDLE>(),
                ptr::null_mut(),
                ptr::null_mut(),
            ) == FALSE
            {
                DeleteProcThreadAttributeList(attr_list_ptr);
                ClosePseudoConsole(hpc);
                CloseHandle(read_pipe_process);
                CloseHandle(write_pipe_process);
                return Err(PtyError::Create(Box::new(io::Error::last_os_error())));
            }

            let mut startup_info: STARTUPINFOEXW = std::mem::zeroed();
            startup_info.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
            startup_info.lpAttributeList = attr_list_ptr;

            // Determine command to run
            let command = config.shell.clone().unwrap_or_else(get_default_shell);
            let mut cmd_line = command.clone();
            for arg in &config.args {
                cmd_line.push(' ');
                cmd_line.push_str(arg);
            }

            let cmd_wide: Vec<u16> = OsStr::new(&cmd_line)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            let mut process_info: PROCESS_INFORMATION = std::mem::zeroed();

            // Set working directory
            let cwd_wide: Option<Vec<u16>> = config.cwd.as_ref().map(|cwd| {
                OsStr::new(cwd)
                    .encode_wide()
                    .chain(std::iter::once(0))
                    .collect()
            });
            let cwd_ptr = cwd_wide.as_ref().map(|v| v.as_ptr()).unwrap_or(ptr::null());

            let result = CreateProcessW(
                ptr::null(),
                cmd_wide.as_ptr() as *mut _,
                ptr::null_mut(),
                ptr::null_mut(),
                FALSE,
                EXTENDED_STARTUPINFO_PRESENT,
                ptr::null_mut(),
                cwd_ptr,
                &mut startup_info.StartupInfo,
                &mut process_info,
            );

            DeleteProcThreadAttributeList(attr_list_ptr);

            if result == FALSE {
                ClosePseudoConsole(hpc);
                CloseHandle(read_pipe_process);
                CloseHandle(write_pipe_process);
                return Err(PtyError::Spawn(format!(
                    "CreateProcessW failed: {}",
                    io::Error::last_os_error()
                )));
            }

            Ok(Self {
                hpc,
                read_pipe: File::from_raw_handle(read_pipe_process as RawHandle),
                write_pipe: File::from_raw_handle(write_pipe_process as RawHandle),
                process_handle: process_info.hProcess,
                thread_handle: process_info.hThread,
                process_id: process_info.dwProcessId,
                exit_status: None,
            })
        }
    }

    impl Drop for Pty {
        fn drop(&mut self) {
            unsafe {
                ClosePseudoConsole(self.hpc);
                if self.thread_handle != INVALID_HANDLE_VALUE {
                    CloseHandle(self.thread_handle);
                }
                CloseHandle(self.process_handle);
            }
        }
    }

    fn get_default_shell() -> String {
        // Try COMSPEC first
        if let Ok(shell) = std::env::var("COMSPEC") {
            return shell;
        }
        // Fall back to cmd.exe
        "cmd.exe".to_string()
    }
}

// ============================================================================
// Re-export the platform-specific Pty
// ============================================================================

#[cfg(unix)]
pub use unix::Pty;

#[cfg(windows)]
pub use windows::Pty;

/// Platform-specific raw handle type
/// On Unix this is RawFd (i32), on Windows this is RawHandle (isize)
#[cfg(unix)]
pub type RawPtyHandle = std::os::unix::io::RawFd;

#[cfg(windows)]
pub type RawPtyHandle = std::os::windows::io::RawHandle;

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pty_config_default() {
        let config = PtyConfig::default();
        assert_eq!(config.size.rows, 0);
        assert_eq!(config.size.cols, 0);
        assert!(config.shell.is_none());
    }

    #[test]
    fn test_pty_size_default() {
        let size = PtySize::default();
        assert_eq!(size.rows, 0);
        assert_eq!(size.cols, 0);
        assert_eq!(size.pixel_width, 0);
        assert_eq!(size.pixel_height, 0);
    }

    /// Helper to wait with a timeout for tests
    #[allow(dead_code)]
    fn wait_with_timeout(pty: &mut Pty, timeout_ms: u64) -> Option<i32> {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);
        loop {
            if let Ok(Some(status)) = pty.try_wait() {
                return Some(status);
            }
            if start.elapsed() > timeout {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_pty_creation_unix() {
        let config = PtyConfig {
            size: PtySize {
                rows: 24,
                cols: 80,
                ..Default::default()
            },
            shell: Some("/bin/sh".to_string()),
            args: vec!["-c".to_string(), "echo hello".to_string()],
            ..Default::default()
        };

        let mut pty = Pty::new(&config).expect("Failed to create PTY");
        assert!(pty.child_pid() > 0);

        // Give the command time to produce output
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Read the output (important on macOS to unblock the child)
        let mut buf = [0u8; 1024];
        let _ = pty.read(&mut buf);

        // Wait for the child with a timeout
        let status = wait_with_timeout(&mut pty, 5000).expect("Child did not exit in time");
        assert_eq!(status, 0);
    }

    #[test]
    #[cfg(unix)]
    fn test_pty_read_write_unix() {
        let config = PtyConfig {
            size: PtySize {
                rows: 24,
                cols: 80,
                ..Default::default()
            },
            shell: Some("/bin/cat".to_string()),
            args: vec![],
            ..Default::default()
        };

        let mut pty = Pty::new(&config).expect("Failed to create PTY");

        // Write to PTY
        pty.write(b"test\n").expect("Failed to write");

        // Give it time to echo back
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Read from PTY
        let mut buf = [0u8; 1024];
        let n = pty.read(&mut buf).expect("Failed to read");
        assert!(n > 0);

        // Kill the process
        pty.send_signal(15).expect("Failed to send signal");
    }

    #[test]
    #[cfg(unix)]
    fn test_pty_resize_unix() {
        let config = PtyConfig {
            size: PtySize {
                rows: 24,
                cols: 80,
                ..Default::default()
            },
            shell: Some("/bin/sh".to_string()),
            args: vec!["-c".to_string(), "sleep 1".to_string()],
            ..Default::default()
        };

        let pty = Pty::new(&config).expect("Failed to create PTY");

        // Test resize
        pty.resize(40, 120).expect("Failed to resize PTY");
        pty.resize(25, 80).expect("Failed to resize PTY again");

        // Clean up
        let _ = pty.send_signal(15);
    }

    #[test]
    #[cfg(unix)]
    fn test_pty_is_running_unix() {
        let config = PtyConfig {
            size: PtySize {
                rows: 24,
                cols: 80,
                ..Default::default()
            },
            shell: Some("/bin/sh".to_string()),
            args: vec!["-c".to_string(), "sleep 10".to_string()],
            ..Default::default()
        };

        let mut pty = Pty::new(&config).expect("Failed to create PTY");

        // Should be running initially
        assert!(pty.is_running());

        // Send SIGTERM
        pty.send_signal(15).expect("Failed to send signal");

        // Give it time to terminate
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Should no longer be running
        assert!(!pty.is_running());
    }

    #[test]
    #[cfg(unix)]
    fn test_pty_try_clone_reader_unix() {
        let config = PtyConfig {
            size: PtySize {
                rows: 24,
                cols: 80,
                ..Default::default()
            },
            shell: Some("/bin/sh".to_string()),
            args: vec!["-c".to_string(), "echo clone_test".to_string()],
            ..Default::default()
        };

        let pty = Pty::new(&config).expect("Failed to create PTY");

        // Clone the reader
        let mut reader = pty.try_clone_reader().expect("Failed to clone reader");

        // Give it time to produce output
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Read from cloned reader
        use std::io::Read;
        let mut buf = [0u8; 1024];
        let n = reader
            .read(&mut buf)
            .expect("Failed to read from cloned reader");
        assert!(n > 0);

        // The output should contain "clone_test"
        let output = String::from_utf8_lossy(&buf[..n]);
        assert!(output.contains("clone_test"), "Output was: {}", output);
    }

    #[test]
    #[cfg(unix)]
    fn test_pty_dup_fd_unix() {
        let config = PtyConfig {
            size: PtySize {
                rows: 24,
                cols: 80,
                ..Default::default()
            },
            shell: Some("/bin/sh".to_string()),
            args: vec!["-c".to_string(), "sleep 1".to_string()],
            ..Default::default()
        };

        let pty = Pty::new(&config).expect("Failed to create PTY");

        // Duplicate the FD
        let duped_fd = pty.dup_fd().expect("Failed to dup FD");
        assert!(duped_fd >= 0);

        // The original FD should still be valid
        let original_fd = pty.raw_fd();
        assert!(original_fd >= 0);

        // They should be different FDs
        assert_ne!(duped_fd, original_fd);

        // Clean up the duped FD
        unsafe {
            libc::close(duped_fd);
        }

        // Clean up
        let _ = pty.send_signal(15);
    }

    /// Test FD passover: create a PTY, duplicate the FD, and reconstruct from raw FD
    #[test]
    #[cfg(unix)]
    fn test_pty_fd_passover_unix() {
        // Create original PTY running cat (echoes input)
        let config = PtyConfig {
            size: PtySize {
                rows: 24,
                cols: 80,
                ..Default::default()
            },
            shell: Some("/bin/cat".to_string()),
            args: vec![],
            ..Default::default()
        };

        let mut original_pty = Pty::new(&config).expect("Failed to create PTY");
        let child_pid = original_pty.child_pid();

        // Duplicate the FD (simulating what happens during upgrade)
        let duped_fd = original_pty.dup_fd().expect("Failed to dup FD");

        // Write something to the original PTY
        original_pty
            .write(b"hello_passover\n")
            .expect("Failed to write");

        // Give it time to echo
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Drop the original PTY (but the child should still be running due to duped FD)
        // Note: In real usage, we'd transfer FD before dropping, here we simulate with dup
        // Actually, don't drop original_pty yet - we need it to keep the child alive

        // Create a new PTY from the duplicated FD
        let mut restored_pty = unsafe { Pty::from_raw_fd(duped_fd, child_pid) };

        // The restored PTY should be able to read
        let mut buf = [0u8; 1024];
        let n = restored_pty
            .read(&mut buf)
            .expect("Failed to read from restored PTY");
        assert!(n > 0);
        let output = String::from_utf8_lossy(&buf[..n]);
        assert!(output.contains("hello_passover"), "Output was: {}", output);

        // The restored PTY should be able to write
        restored_pty
            .write(b"test_write\n")
            .expect("Failed to write to restored PTY");

        // Give it time to echo
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Read again
        let n = restored_pty.read(&mut buf).expect("Failed to read again");
        assert!(n > 0);
        let output = String::from_utf8_lossy(&buf[..n]);
        assert!(output.contains("test_write"), "Output was: {}", output);

        // Child PID should match
        assert_eq!(restored_pty.child_pid(), child_pid);

        // Clean up - send signal to terminate cat
        restored_pty.send_signal(15).expect("Failed to send signal");

        // Don't drop original_pty here - it will try to SIGHUP the same process
        std::mem::forget(original_pty);
    }

    /// Test PTY exit status
    #[test]
    #[cfg(unix)]
    fn test_pty_exit_status_unix() {
        // Test successful exit
        let config = PtyConfig {
            size: PtySize {
                rows: 24,
                cols: 80,
                ..Default::default()
            },
            shell: Some("/bin/sh".to_string()),
            args: vec!["-c".to_string(), "exit 0".to_string()],
            ..Default::default()
        };

        let mut pty = Pty::new(&config).expect("Failed to create PTY");
        let status = wait_with_timeout(&mut pty, 5000).expect("Child did not exit in time");
        assert_eq!(status, 0);

        // Test non-zero exit
        let config = PtyConfig {
            size: PtySize {
                rows: 24,
                cols: 80,
                ..Default::default()
            },
            shell: Some("/bin/sh".to_string()),
            args: vec!["-c".to_string(), "exit 42".to_string()],
            ..Default::default()
        };

        let mut pty = Pty::new(&config).expect("Failed to create PTY");
        let status = wait_with_timeout(&mut pty, 5000).expect("Child did not exit in time");
        assert_eq!(status, 42);
    }

    /// Test PTY with environment variables
    #[test]
    #[cfg(unix)]
    fn test_pty_env_vars_unix() {
        use std::io::Read;

        let config = PtyConfig {
            size: PtySize {
                rows: 24,
                cols: 80,
                ..Default::default()
            },
            shell: Some("/bin/sh".to_string()),
            args: vec!["-c".to_string(), "echo $TEST_VAR".to_string()],
            env: vec![("TEST_VAR".to_string(), "test_value_123".to_string())],
            ..Default::default()
        };

        let pty = Pty::new(&config).expect("Failed to create PTY");

        // Give it time to produce output
        std::thread::sleep(std::time::Duration::from_millis(100));

        let mut reader = pty.try_clone_reader().expect("Failed to clone reader");
        let mut buf = [0u8; 1024];
        let n = reader.read(&mut buf).expect("Failed to read");
        let output = String::from_utf8_lossy(&buf[..n]);
        assert!(output.contains("test_value_123"), "Output was: {}", output);
    }
}
