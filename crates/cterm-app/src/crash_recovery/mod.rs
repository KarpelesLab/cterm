//! Crash recovery system for cterm
//!
//! This module provides crash recovery functionality:
//! - Watchdog process that monitors the main cterm process
//! - Crash state file for persisting terminal state
//! - FD passing between watchdog and main process
//! - Recovery and restart after crashes

#[cfg(unix)]
mod state;
#[cfg(unix)]
mod watchdog;

#[cfg(unix)]
pub use state::{
    crash_marker_path, crash_state_path, read_crash_marker, read_crash_state, write_crash_state,
    CrashState,
};
#[cfg(unix)]
pub use watchdog::{
    notify_watchdog_shutdown, receive_recovery_fds, register_fd_with_watchdog, run_watchdog,
    unregister_fd_with_watchdog, WatchdogError,
};
