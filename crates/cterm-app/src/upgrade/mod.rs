//! Seamless upgrade system for cterm
//!
//! This module provides functionality for upgrading cterm without losing
//! any running terminal sessions. It handles:
//!
//! - Checking for updates from GitHub releases
//! - Downloading and verifying new binaries
//! - Serializing terminal state
//! - Passing PTY file descriptors/handles to the new process
//! - Reconstructing windows and tabs in the new process
//!
//! On Unix: Uses SCM_RIGHTS for FD passing via socketpair
//! On Windows: Uses STARTUPINFOEX with PROC_THREAD_ATTRIBUTE_HANDLE_LIST

mod protocol;
mod state;
mod updater;

#[cfg(unix)]
pub use protocol::{execute_upgrade, receive_upgrade, UpgradeError, MAX_FDS, MAX_STATE_SIZE};
#[cfg(windows)]
pub use protocol::{
    execute_upgrade, receive_upgrade, HandleInfo, UpgradeError, WindowsUpgradeData, MAX_FDS,
    MAX_STATE_SIZE,
};
pub use state::{TabUpgradeState, TerminalUpgradeState, UpgradeState, WindowUpgradeState};
pub use updater::{UpdateError, UpdateInfo, Updater};
