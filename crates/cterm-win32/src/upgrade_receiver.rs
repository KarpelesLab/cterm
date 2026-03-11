//! Upgrade receiver - handles receiving state during seamless upgrade
//!
//! When cterm is started with --upgrade-state, it reads the saved state from
//! a temp file. Since all sessions live in the ctermd daemon, sessions survive
//! the upgrade and just need to be reconnected.

use std::path::Path;

use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, TranslateMessage, MSG,
};

/// Run the upgrade receiver
///
/// Reads upgrade state from the given file path, then starts the
/// normal application which will reconnect to daemon sessions.
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

    // Log the sessions that need reconnection
    for (i, window) in state.windows.iter().enumerate() {
        for tab in &window.tabs {
            if let Some(ref session_id) = tab.session_id {
                log::info!(
                    "Window {} tab '{}': session_id={}",
                    i,
                    tab.title,
                    session_id
                );
            }
        }
    }

    // Since sessions live in the daemon, they survive the upgrade.
    // Start the normal application which will detect and reconnect
    // to daemon sessions on startup.

    // Load config and theme
    let config = cterm_app::load_config().unwrap_or_default();
    let theme = cterm_app::resolve_theme(&config);

    // Set up DPI awareness
    crate::dpi::setup_dpi_awareness();
    crate::dialog_utils::init_common_controls();
    crate::window::register_window_class()?;

    let _hwnd = crate::window::create_window(&config, &theme)?;

    // Message loop
    let mut msg = MSG::default();
    loop {
        let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if ret.0 == 0 {
            break;
        }
        if ret.0 == -1 {
            return Err("GetMessageW error".into());
        }
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    log::info!("cterm (restored) exiting");
    Ok(())
}
