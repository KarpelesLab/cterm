//! Seamless upgrade system for cterm
//!
//! Since terminal sessions live in the ctermd daemon, upgrading cterm
//! is simple: save UI layout state to a temp file, spawn the new binary,
//! and exit. The new process reads the state, connects to ctermd, and
//! reconstructs windows with the same tabs and session mappings.
//!
//! The updater module handles checking for updates, downloading, and
//! verifying new binaries from GitHub releases.

mod protocol;
mod state;
mod updater;

pub use protocol::{execute_upgrade, receive_upgrade, UpgradeError};
pub use state::{TabUpgradeState, UpgradeState, WindowUpgradeState};
pub use updater::{UpdateError, UpdateInfo, Updater};
