//! Upgrade receiver - handles receiving state during seamless upgrade
//!
//! When cterm is started with --upgrade-state, it reads the saved state from
//! a temp file, reconnects to running daemon sessions, and reconstructs windows.

use std::path::Path;

/// Run the upgrade receiver
///
/// Reads upgrade state from the given file path, then starts the app
/// which will reconnect to daemon sessions listed in the state.
pub fn run_receiver(state_path: &str) -> i32 {
    match receive_and_start(state_path) {
        Ok(()) => 0,
        Err(e) => {
            log::error!("Upgrade receiver failed: {}", e);
            1
        }
    }
}

fn receive_and_start(state_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let state = cterm_app::upgrade::receive_upgrade(Path::new(state_path))?;

    log::info!(
        "Upgrade state received: format_version={}, {} window(s)",
        state.format_version,
        state.windows.len()
    );

    // Store the upgrade state for AppDelegate to use during launch
    crate::app::set_upgrade_state(state);

    log::info!("Starting app with restored state...");

    // Run the app - AppDelegate will detect the upgrade state and restore windows
    crate::app::run_app_internal();

    Ok(())
}
