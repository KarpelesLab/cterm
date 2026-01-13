//! Native PTY implementation for Unix with raw file descriptor access
//!
//! This module provides a PTY implementation that exposes raw file descriptors,
//! enabling seamless upgrades by passing PTY FDs to a new process via SCM_RIGHTS.

use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};

use crate::pty::{PtyConfig, PtyError};

/// Native PTY with exposed raw file descriptor
pub struct NativePty {
    /// The master PTY file descriptor
    master_fd: RawFd,
    /// File wrapper for the master (for I/O operations)
    master: File,
    /// Child process ID
    child_pid: libc::pid_t,
    /// Cached exit status
    exit_status: Option<i32>,
}

impl NativePty {
    /// Create a new PTY and spawn the shell
    pub fn new(config: &PtyConfig) -> Result<Self, PtyError> {
        unsafe { Self::create_pty_and_spawn(config) }
    }

    /// Create PTY from an existing file descriptor (for upgrade receiver)
    ///
    /// # Safety
    /// The caller must ensure `fd` is a valid master PTY file descriptor
    /// and `child_pid` is the correct process ID of the child process.
    pub unsafe fn from_raw_fd(fd: RawFd, child_pid: libc::pid_t) -> Self {
        Self {
            master_fd: fd,
            master: File::from_raw_fd(fd),
            child_pid,
            exit_status: None,
        }
    }

    /// Get the raw file descriptor for passing via SCM_RIGHTS
    pub fn as_raw_fd(&self) -> RawFd {
        self.master_fd
    }

    /// Get the child process ID
    pub fn child_pid(&self) -> libc::pid_t {
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

    /// Send a signal to the child process
    pub fn kill(&self, signal: i32) -> io::Result<()> {
        let ret = unsafe { libc::kill(self.child_pid, signal) };
        if ret < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
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

        // Set master to non-blocking for better behavior
        // Note: We don't set non-blocking here as the reader thread expects blocking I/O

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

        // Execute the shell
        libc::execv(shell_cstring.as_ptr(), args_ptrs.as_ptr());

        // If exec fails, exit
        libc::_exit(127);
    }
}

impl Drop for NativePty {
    fn drop(&mut self) {
        // Send SIGHUP to the child process
        let _ = self.kill(libc::SIGHUP);
        // Note: We don't close master_fd here because File will do it
    }
}

impl AsRawFd for NativePty {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pty::PtySize;

    #[test]
    fn test_native_pty_creation() {
        let config = PtyConfig {
            size: PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            },
            shell: Some("/bin/sh".to_string()),
            args: vec!["-c".to_string(), "echo hello".to_string()],
            ..Default::default()
        };

        let mut pty = NativePty::new(&config).expect("Failed to create PTY");
        assert!(pty.child_pid() > 0);
        assert!(pty.as_raw_fd() >= 0);

        // Wait for the child to exit
        let status = pty.wait().expect("Failed to wait");
        assert_eq!(status, 0);
    }

    #[test]
    fn test_native_pty_read_write() {
        let config = PtyConfig {
            size: PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            },
            shell: Some("/bin/cat".to_string()),
            args: vec![],
            ..Default::default()
        };

        let mut pty = NativePty::new(&config).expect("Failed to create PTY");

        // Write to PTY
        pty.write(b"test\n").expect("Failed to write");

        // Give it time to echo back
        std::thread::sleep(std::time::Duration::from_millis(100));

        // Read from PTY
        let mut buf = [0u8; 1024];
        let n = pty.read(&mut buf).expect("Failed to read");
        assert!(n > 0);

        // Kill the cat process
        pty.kill(libc::SIGTERM).expect("Failed to kill");
    }

    #[test]
    fn test_dup_fd() {
        let config = PtyConfig {
            size: PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            },
            shell: Some("/bin/sh".to_string()),
            args: vec!["-c".to_string(), "sleep 1".to_string()],
            ..Default::default()
        };

        let pty = NativePty::new(&config).expect("Failed to create PTY");
        let duped = pty.dup_fd().expect("Failed to dup FD");
        assert!(duped >= 0);
        assert_ne!(duped, pty.as_raw_fd());

        // Clean up the duped FD
        unsafe {
            libc::close(duped);
        }
    }
}
