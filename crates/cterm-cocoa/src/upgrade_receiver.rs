//! Upgrade receiver - handles receiving state from the old process during seamless upgrade
//!
//! This module is used when cterm is started with --upgrade-receiver flag.
//! It receives state from the parent process via an inherited FD, receives the terminal state
//! and PTY file descriptors, then reconstructs the windows.

use std::os::unix::io::RawFd;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::DefinedClass;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate};
use objc2_foundation::{MainThreadMarker, NSNotification, NSObject, NSObjectProtocol};

use cterm_app::config::{load_config, Config};
use cterm_app::upgrade::{receive_upgrade, UpgradeState, WindowUpgradeState};
use cterm_core::screen::{Screen, ScreenConfig};
use cterm_core::term::Terminal;
use cterm_core::Pty;
use cterm_ui::theme::Theme;

use crate::menu;
use crate::window::CtermWindow;

/// Run the upgrade receiver
///
/// This function:
/// 1. Reads from the inherited FD passed by the parent
/// 2. Receives the upgrade state and PTY file descriptors
/// 3. Sends acknowledgment
/// 4. Reconstructs the macOS application with the received state
pub fn run_receiver(fd: i32) -> i32 {
    match receive_and_reconstruct(fd) {
        Ok(()) => 0,
        Err(e) => {
            log::error!("Upgrade receiver failed: {}", e);
            1
        }
    }
}

fn receive_and_reconstruct(fd: i32) -> Result<(), Box<dyn std::error::Error>> {
    // Use the upgrade module to receive the state
    let (state, fds) = receive_upgrade(fd as RawFd)?;

    log::info!(
        "Upgrade state: format_version={}, cterm_version={}, windows={}",
        state.format_version,
        state.cterm_version,
        state.windows.len()
    );

    log::info!("Starting Cocoa app with restored state...");

    // Now start the Cocoa app and reconstruct the windows
    run_cocoa_with_state(state, fds)?;

    Ok(())
}

// Thread-local storage for upgrade state (used to pass data to AppDelegate)
thread_local! {
    static UPGRADE_STATE: std::cell::RefCell<Option<(UpgradeState, Vec<RawFd>)>> =
        const { std::cell::RefCell::new(None) };
}

/// Start Cocoa application with the restored state
fn run_cocoa_with_state(
    state: UpgradeState,
    fds: Vec<RawFd>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Store state and FDs for use during window construction
    UPGRADE_STATE.with(|s| {
        *s.borrow_mut() = Some((state, fds));
    });

    // Get main thread marker
    let mtm = MainThreadMarker::new().expect("Must be called on main thread");

    // Load config and theme
    let config = load_config().unwrap_or_default();
    let theme = get_theme(&config);

    // Get the shared application instance
    let app = NSApplication::sharedApplication(mtm);

    // Set activation policy to regular (shows in Dock)
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    // Create and set the application delegate
    let delegate = UpgradeReceiverDelegate::new(mtm, config, theme);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    // Create the menu bar
    let menu_bar = menu::create_menu_bar(mtm);
    app.setMainMenu(Some(&menu_bar));

    // Activate the app
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);

    log::info!("Starting main run loop (upgrade receiver)");

    // Run the main event loop
    app.run();

    Ok(())
}

/// Get the theme based on configuration
fn get_theme(config: &Config) -> Theme {
    if let Some(ref custom) = config.appearance.custom_theme {
        return custom.clone();
    }

    // Find built-in theme by name
    let themes = Theme::builtin_themes();
    themes
        .into_iter()
        .find(|t| t.name == config.appearance.theme)
        .unwrap_or_else(Theme::dark)
}

/// Application delegate for upgrade receiver
pub struct UpgradeReceiverDelegateIvars {
    config: Config,
    theme: Theme,
    windows: std::cell::RefCell<Vec<Retained<CtermWindow>>>,
}

objc2::define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = objc2::MainThreadOnly]
    #[name = "UpgradeReceiverDelegate"]
    #[ivars = UpgradeReceiverDelegateIvars]
    pub struct UpgradeReceiverDelegate;

    unsafe impl NSObjectProtocol for UpgradeReceiverDelegate {}

    unsafe impl NSApplicationDelegate for UpgradeReceiverDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn application_did_finish_launching(&self, _notification: &NSNotification) {
            log::info!("Upgrade receiver: Application did finish launching");

            // Retrieve the stored state
            UPGRADE_STATE.with(|s| {
                if let Some((state, fds)) = s.borrow_mut().take() {
                    self.reconstruct_windows(state, fds);
                }
            });
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _sender: &NSApplication) -> bool {
            true
        }
    }
);

impl UpgradeReceiverDelegate {
    pub fn new(mtm: MainThreadMarker, config: Config, theme: Theme) -> Retained<Self> {
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(UpgradeReceiverDelegateIvars {
            config,
            theme,
            windows: std::cell::RefCell::new(Vec::new()),
        });
        unsafe { objc2::msg_send![super(this), init] }
    }

    fn reconstruct_windows(&self, state: UpgradeState, fds: Vec<RawFd>) {
        log::info!("Reconstructing {} windows", state.windows.len());

        let mtm = MainThreadMarker::from(self);

        for (window_idx, window_state) in state.windows.into_iter().enumerate() {
            log::info!(
                "Window {}: {}x{} at ({}, {}), {} tabs, active={}",
                window_idx,
                window_state.width,
                window_state.height,
                window_state.x,
                window_state.y,
                window_state.tabs.len(),
                window_state.active_tab
            );

            match self.create_restored_window(mtm, window_state, &fds) {
                Ok(window) => {
                    window.makeKeyAndOrderFront(None);
                    self.ivars().windows.borrow_mut().push(window);
                    log::info!("Window {} restored successfully", window_idx);
                }
                Err(e) => {
                    log::error!("Failed to restore window {}: {}", window_idx, e);
                }
            }
        }
    }

    fn create_restored_window(
        &self,
        mtm: MainThreadMarker,
        window_state: WindowUpgradeState,
        fds: &[RawFd],
    ) -> Result<Retained<CtermWindow>, Box<dyn std::error::Error>> {
        // For now, we only support restoring the first tab
        // (macOS native tabbing would require multiple windows)
        if let Some(tab_state) = window_state.tabs.first() {
            // Get the PTY FD for this tab
            if tab_state.pty_fd_index >= fds.len() {
                return Err(format!(
                    "PTY FD index {} out of range (max {})",
                    tab_state.pty_fd_index,
                    fds.len()
                )
                .into());
            }

            let pty_fd = fds[tab_state.pty_fd_index];

            // Reconstruct Pty from the FD and child PID
            let pty = unsafe { Pty::from_raw_fd(pty_fd, tab_state.child_pid) };

            // Reconstruct Screen from the terminal state
            let term_state = &tab_state.terminal;
            let screen_config = ScreenConfig {
                scrollback_lines: self.ivars().config.general.scrollback_lines,
            };

            let screen = Screen::from_upgrade_state(
                term_state.grid.clone(),
                term_state.scrollback.clone(),
                term_state.alternate_grid.clone(),
                term_state.cursor.clone(),
                term_state.saved_cursor.clone(),
                term_state.alt_saved_cursor.clone(),
                term_state.scroll_region,
                term_state.style.clone(),
                term_state.modes.clone(),
                term_state.title.clone(),
                term_state.scroll_offset,
                term_state.tab_stops.clone(),
                screen_config,
            );

            // Create Terminal with the restored screen and PTY
            let terminal = Terminal::from_restored(screen, pty);

            // Create the window with the restored terminal
            let window = CtermWindow::from_restored(
                mtm,
                &self.ivars().config,
                &self.ivars().theme,
                terminal,
            );

            // Restore template_name if present (needed for unique tab detection)
            if tab_state.template_name.is_some() {
                if let Some(terminal_view) = window.active_terminal() {
                    terminal_view.set_template_name(tab_state.template_name.clone());
                    log::info!("Restored template_name: {:?}", tab_state.template_name);
                }
            }

            Ok(window)
        } else {
            Err("No tabs in window state".into())
        }
    }
}
