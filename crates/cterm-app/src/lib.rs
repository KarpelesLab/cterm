//! cterm-app: Application logic for cterm
//!
//! This crate contains the application logic that is independent of the UI,
//! including configuration management, session handling, sticky tabs,
//! seamless upgrade functionality, and crash recovery.

pub mod config;
#[cfg(unix)]
pub mod crash_recovery;
pub mod docker;
pub mod file_transfer;
pub mod git_sync;
pub mod log_capture;
pub mod session;
pub mod shortcuts;
pub mod upgrade;

pub use config::{
    background_sync, load_config, load_sticky_tabs, save_config, save_config_with_sync,
    save_sticky_tabs, Config,
};
#[cfg(unix)]
pub use crash_recovery::{
    clear_crash_state, crash_marker_path, crash_state_path, notify_watchdog_shutdown,
    read_crash_marker, read_crash_state, receive_recovery_fds, register_fd_with_watchdog,
    run_watchdog, unregister_fd_with_watchdog, write_crash_state, CrashState, RecoveredFd,
    WatchdogError,
};
pub use git_sync::{
    clone_repo, get_directory_remote_url, get_remote_url, get_sync_status, init_with_remote,
    is_git_repo, prepare_working_directory, pull_with_conflict_resolution, GitError, InitResult,
    PullResult, SyncStatus,
};
pub use session::{Session, TabState, WindowState};
pub use shortcuts::ShortcutManager;
#[cfg(windows)]
pub use upgrade::{execute_upgrade, receive_upgrade, HandleInfo, UpgradeError, WindowsUpgradeData};
#[cfg(unix)]
pub use upgrade::{execute_upgrade, receive_upgrade, UpgradeError};
pub use upgrade::{UpdateError, UpdateInfo, Updater, UpgradeState};
