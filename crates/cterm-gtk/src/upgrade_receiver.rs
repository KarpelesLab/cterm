//! Upgrade receiver - handles receiving state during seamless upgrade
//!
//! When cterm is started with --upgrade-state, it reads the saved state from
//! a temp file, reconnects to running daemon sessions, and reconstructs windows.

use std::path::Path;

use cterm_app::config::load_config;
use cterm_app::upgrade::UpgradeState;
use gtk4::glib;
use gtk4::prelude::*;

/// Run the upgrade receiver
///
/// Reads upgrade state from the given file path, reconnects to daemon
/// sessions, and reconstructs the GTK application with restored windows.
pub fn run_receiver(state_path: &str) -> glib::ExitCode {
    #[cfg(feature = "adwaita")]
    let _ = libadwaita::init();

    match receive_and_reconstruct(state_path) {
        Ok(()) => glib::ExitCode::SUCCESS,
        Err(e) => {
            log::error!("Upgrade receiver failed: {}", e);
            glib::ExitCode::FAILURE
        }
    }
}

fn receive_and_reconstruct(state_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let state = cterm_app::upgrade::receive_upgrade(Path::new(state_path))?;

    log::info!(
        "Upgrade state: format_version={}, {} window(s)",
        state.format_version,
        state.windows.len()
    );

    // Store state for use during GTK activate
    UPGRADE_STATE.with(|s| {
        *s.borrow_mut() = Some(state);
    });

    // Start GTK and reconstruct windows
    let app = gtk4::Application::builder()
        .application_id("com.cterm.terminal")
        .flags(gtk4::gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_activate(|app| {
        UPGRADE_STATE.with(|s| {
            if let Some(state) = s.borrow_mut().take() {
                reconstruct_windows(app, state);
            }
        });
    });

    app.run_with_args(&[] as &[&str]);
    Ok(())
}

thread_local! {
    static UPGRADE_STATE: std::cell::RefCell<Option<UpgradeState>> =
        const { std::cell::RefCell::new(None) };
}

/// Reconstruct windows by reconnecting to daemon sessions
fn reconstruct_windows(app: &gtk4::Application, state: UpgradeState) {
    log::info!(
        "Reconstructing {} window(s) from upgrade state",
        state.windows.len()
    );

    let config = load_config().unwrap_or_default();
    let theme = cterm_app::resolve_theme(&config);

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            log::error!("Failed to create tokio runtime: {}", e);
            // Fall back to creating a fresh window
            crate::app::build_ui(app);
            return;
        }
    };

    let mut any_restored = false;

    for (window_idx, window_state) in state.windows.into_iter().enumerate() {
        log::info!(
            "Window {}: {}x{}, {} tab(s)",
            window_idx,
            window_state.width,
            window_state.height,
            window_state.tabs.len(),
        );

        let mut reconnected_sessions = Vec::new();

        for tab_state in &window_state.tabs {
            let Some(ref session_id) = tab_state.session_id else {
                log::warn!("Tab '{}' has no session_id, skipping", tab_state.title);
                continue;
            };

            match rt.block_on(async {
                let conn = cterm_client::DaemonConnection::connect_local().await?;
                conn.attach_session(session_id, 80, 24).await
            }) {
                Ok((handle, screen)) => {
                    log::info!("Reconnected to session {}", session_id);
                    reconnected_sessions.push(cterm_app::daemon_reconnect::ReconnectedSession {
                        handle,
                        title: tab_state.title.clone(),
                        screen,
                    });
                }
                Err(e) => {
                    log::error!("Failed to reconnect session {}: {}", session_id, e);
                }
            }
        }

        if reconnected_sessions.is_empty() {
            log::warn!("No sessions restored for window {}, skipping", window_idx);
            continue;
        }

        // Create a window and add reconnected tabs
        let window = crate::window::CtermWindow::new_empty(app, &config, &theme);

        for recon in reconnected_sessions {
            window.add_reconnected_tab(recon);
        }

        // Restore window geometry
        if window_state.maximized {
            window.window.maximize();
        }
        if window_state.fullscreen {
            window.window.fullscreen();
        }

        window.present();
        any_restored = true;
        log::info!("Window {} restored successfully", window_idx);
    }

    if !any_restored {
        log::warn!("No sessions could be restored, creating fresh window");
        crate::app::build_ui(app);
    }
}
