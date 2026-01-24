//! Application setup and management for macOS
//!
//! Handles NSApplication lifecycle and main event loop.

use clap::Parser;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate};
use objc2_foundation::{MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSString};
use std::path::PathBuf;

use cterm_app::config::{load_config, Config};
use cterm_ui::theme::Theme;

use crate::menu;
use crate::window::CtermWindow;

/// Command-line arguments for cterm
#[derive(Parser, Debug)]
#[command(
    name = "cterm",
    version,
    about = "A high-performance terminal emulator"
)]
pub struct Args {
    /// Execute a command instead of the default shell
    #[arg(short = 'e', long = "execute")]
    pub command: Option<String>,

    /// Set the working directory
    #[arg(short = 'd', long = "directory")]
    pub directory: Option<PathBuf>,

    /// Start in fullscreen mode
    #[arg(long)]
    pub fullscreen: bool,

    /// Start maximized
    #[arg(long)]
    pub maximized: bool,

    /// Set the window title
    #[arg(short = 't', long = "title")]
    pub title: Option<String>,

    /// Receive upgrade state from parent process via inherited FD (internal use)
    #[arg(long, hide = true)]
    pub upgrade_receiver: Option<i32>,

    /// Run under watchdog supervision (internal use)
    #[arg(long, hide = true)]
    pub supervised: Option<i32>,

    /// Recover from crash with FDs from watchdog (internal use)
    #[arg(long, hide = true)]
    pub crash_recovery: Option<i32>,

    /// Disable watchdog supervision (run directly without crash recovery)
    #[arg(long)]
    pub no_watchdog: bool,
}

/// Global application arguments (accessible from window creation)
static APP_ARGS: std::sync::OnceLock<Args> = std::sync::OnceLock::new();

/// Watchdog FD for crash recovery (-1 if not supervised)
#[cfg(unix)]
static WATCHDOG_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Get the watchdog FD if we're running supervised
#[cfg(unix)]
pub fn get_watchdog_fd() -> Option<i32> {
    let fd = WATCHDOG_FD.load(std::sync::atomic::Ordering::SeqCst);
    if fd >= 0 {
        Some(fd)
    } else {
        None
    }
}

/// Thread-local storage for recovery FDs (used during crash recovery)
#[cfg(unix)]
thread_local! {
    static RECOVERY_FDS: std::cell::RefCell<Vec<cterm_app::RecoveredFd>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Thread-local storage for upgrade state (used during seamless upgrade)
#[cfg(unix)]
thread_local! {
    static UPGRADE_STATE: std::cell::RefCell<Option<(cterm_app::upgrade::UpgradeState, Vec<std::os::unix::io::RawFd>)>> =
        const { std::cell::RefCell::new(None) };
}

/// Take recovery FDs (consumes them)
#[cfg(unix)]
pub fn take_recovery_fds() -> Vec<cterm_app::RecoveredFd> {
    RECOVERY_FDS.with(|r| std::mem::take(&mut *r.borrow_mut()))
}

/// Take upgrade state (consumes it)
#[cfg(unix)]
pub fn take_upgrade_state() -> Option<(
    cterm_app::upgrade::UpgradeState,
    Vec<std::os::unix::io::RawFd>,
)> {
    UPGRADE_STATE.with(|s| s.borrow_mut().take())
}

/// Store upgrade state for use during app launch
#[cfg(unix)]
pub fn set_upgrade_state(
    state: cterm_app::upgrade::UpgradeState,
    fds: Vec<std::os::unix::io::RawFd>,
) {
    UPGRADE_STATE.with(|s| {
        *s.borrow_mut() = Some((state, fds));
    });
}

/// Check if we're in crash recovery mode
#[cfg(unix)]
pub fn is_crash_recovery() -> bool {
    RECOVERY_FDS.with(|r| !r.borrow().is_empty())
}

/// Get the application arguments (call only after run())
pub fn get_args() -> &'static Args {
    APP_ARGS.get().expect("Args not initialized")
}

/// Application state stored in the delegate
pub struct AppDelegateIvars {
    config: Config,
    theme: Theme,
    windows: std::cell::RefCell<Vec<Retained<CtermWindow>>>,
    /// Hash of last saved crash state (to avoid redundant writes)
    #[cfg(unix)]
    last_state_hash: std::cell::Cell<u64>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "CtermAppDelegate"]
    #[ivars = AppDelegateIvars]
    pub struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn application_did_finish_launching(&self, _notification: &NSNotification) {
            log::info!("Application did finish launching");

            let mtm = MainThreadMarker::from(self);

            // Check for crash recovery
            #[cfg(unix)]
            let recovery_fds = take_recovery_fds();

            #[cfg(unix)]
            if !recovery_fds.is_empty() {
                log::info!("Recovering {} terminals from crash", recovery_fds.len());

                // Try to read saved crash state for display restoration
                let saved_state = cterm_app::read_crash_state().ok();

                // Build a map from watchdog FD ID to saved terminal state
                let state_map: std::collections::HashMap<u64, &cterm_app::upgrade::TabUpgradeState> =
                    if let Some(ref state) = saved_state {
                        state
                            .state
                            .windows
                            .iter()
                            .flat_map(|w| w.tabs.iter())
                            .filter(|t| t.watchdog_fd_id > 0)
                            .map(|t| (t.watchdog_fd_id, t))
                            .collect()
                    } else {
                        std::collections::HashMap::new()
                    };

                log::info!(
                    "Found {} saved terminal states to restore",
                    state_map.len()
                );

                // Check for crash marker to show recovery message
                if let Some((signal, pid)) = cterm_app::read_crash_marker() {
                    log::warn!(
                        "Recovered from crash: signal {}, previous PID {}",
                        signal,
                        pid
                    );
                    // Show crash recovery dialog
                    let wants_report = crate::dialogs::show_crash_recovery(
                        mtm,
                        signal,
                        pid as i32,
                        recovery_fds.len(),
                    );
                    if wants_report {
                        // Open GitHub issues page for crash reporting
                        let title = format!("Crash report (signal {})", signal);
                        let body = format!(
                            "## Crash Details\n\n- Signal: {}\n- Previous PID: {}\n- Recovered terminals: {}\n\n## Description\n\nPlease describe what you were doing when the crash occurred:\n\n",
                            signal, pid, recovery_fds.len()
                        );
                        // Simple URL encoding for the query parameters
                        fn url_encode(s: &str) -> String {
                            let mut result = String::with_capacity(s.len() * 3);
                            for c in s.chars() {
                                match c {
                                    'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => result.push(c),
                                    ' ' => result.push('+'),
                                    _ => {
                                        for byte in c.to_string().as_bytes() {
                                            result.push_str(&format!("%{:02X}", byte));
                                        }
                                    }
                                }
                            }
                            result
                        }
                        let url_str = format!(
                            "https://github.com/KarpelesLab/cterm/issues/new?title={}&body={}",
                            url_encode(&title),
                            url_encode(&body)
                        );
                        if let Some(url) = objc2_foundation::NSURL::URLWithString(&NSString::from_str(&url_str)) {
                            unsafe {
                                let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
                                workspace.openURL(&url);
                            }
                        }
                    }
                }

                // Create windows for recovered terminals
                for (i, recovered) in recovery_fds.iter().enumerate() {
                    let window = CtermWindow::from_recovered_fd(
                        mtm,
                        &self.ivars().config,
                        &self.ivars().theme,
                        recovered,
                    );

                    // Try to restore display state and title if we have saved state for this FD
                    if let Some(tab_state) = state_map.get(&recovered.id) {
                        if let Some(terminal_view) = window.active_terminal() {
                            terminal_view.restore_display_state(&tab_state.terminal);
                            // Restore template name if present
                            if tab_state.template_name.is_some() {
                                terminal_view.set_template_name(tab_state.template_name.clone());
                            }
                            log::info!(
                                "Restored display state for terminal (watchdog_fd_id={})",
                                recovered.id
                            );
                        }
                        // Restore window title
                        window.setTitle(&NSString::from_str(&tab_state.title));
                    }

                    // Disable tabbing to prevent auto-merging into existing windows
                    window.setTabbingMode(objc2_app_kit::NSWindowTabbingMode::Disallowed);

                    self.ivars().windows.borrow_mut().push(window.clone());

                    if i == 0 {
                        // Make first window key
                        window.makeKeyAndOrderFront(None);
                    } else {
                        // Show as separate window (not as tab)
                        window.makeKeyAndOrderFront(None);
                    }

                    // Re-enable tabbing for future tabs in this window
                    window.setTabbingMode(objc2_app_kit::NSWindowTabbingMode::Preferred);
                }

                // Clear crash state file after successful recovery
                let _ = cterm_app::clear_crash_state();

                // Start periodic state saving
                self.start_state_save_timer(mtm);

                // Activate the app to bring window to front
                #[allow(deprecated)]
                NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);

                return;
            }

            // Check for seamless upgrade state
            #[cfg(unix)]
            if let Some((upgrade_state, fds)) = take_upgrade_state() {
                log::info!(
                    "Restoring {} windows from seamless upgrade",
                    upgrade_state.windows.len()
                );
                self.restore_from_upgrade(mtm, upgrade_state, fds);

                // Activate the app to bring window to front
                #[allow(deprecated)]
                NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);

                return;
            }

            // Normal startup - create the main window
            let window = CtermWindow::new(mtm, &self.ivars().config, &self.ivars().theme);

            // Store window reference
            self.ivars().windows.borrow_mut().push(window.clone());

            // Show the window
            window.makeKeyAndOrderFront(None);

            // Activate the app to bring window to front
            #[allow(deprecated)]
            NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);

            // Start periodic state saving (only if running under watchdog)
            #[cfg(unix)]
            if get_watchdog_fd().is_some() {
                self.start_state_save_timer(mtm);
            }
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _sender: &NSApplication) -> bool {
            true
        }
    }

    // Menu action handlers
    impl AppDelegate {
        /// Timer callback for periodic state saving
        #[unsafe(method(saveStateTimer:))]
        fn save_state_timer(&self, _timer: Option<&objc2::runtime::AnyObject>) {
            self.save_crash_state();
        }

        #[unsafe(method(showPreferences:))]
        fn action_show_preferences(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let mtm = MainThreadMarker::from(self);
            let config = self.ivars().config.clone();
            crate::preferences::show_preferences(mtm, &config, |_new_config| {
                // Config saved - could reload theme or apply changes here
                log::info!("Preferences saved");
            });
        }

        #[unsafe(method(showTabTemplates:))]
        fn action_show_tab_templates(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let mtm = MainThreadMarker::from(self);
            let templates = cterm_app::config::load_sticky_tabs().unwrap_or_default();
            crate::tab_templates::show_tab_templates(mtm, templates);
        }

        #[unsafe(method(openTabTemplate:))]
        fn action_open_tab_template(&self, sender: Option<&objc2::runtime::AnyObject>) {
            use objc2_app_kit::NSMenuItem;

            if let Some(sender) = sender {
                // Get the menu item's tag which is the template index
                let item: &NSMenuItem = unsafe { &*(sender as *const _ as *const NSMenuItem) };
                let index = item.tag() as usize;

                if let Ok(templates) = cterm_app::config::load_sticky_tabs() {
                    if let Some(template) = templates.get(index) {
                        self.open_template(template);
                    }
                }
            }
        }

        #[unsafe(method(newWindow:))]
        fn action_new_window(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            use objc2_app_kit::NSWindowTabbingMode;

            let mtm = MainThreadMarker::from(self);
            let window = CtermWindow::new(mtm, &self.ivars().config, &self.ivars().theme);

            // Temporarily disable tabbing to force a new window instead of a tab
            window.setTabbingMode(NSWindowTabbingMode::Disallowed);

            self.ivars().windows.borrow_mut().push(window.clone());
            window.makeKeyAndOrderFront(None);

            // Re-enable tabbing for future tabs in this window
            window.setTabbingMode(NSWindowTabbingMode::Preferred);

            log::info!("Created new window");
        }

        /// Open a new tab running in a Docker devcontainer
        #[unsafe(method(openInContainer:))]
        fn action_open_in_container(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            // Check if Docker is available
            if let Err(e) = cterm_app::docker::check_docker_available() {
                log::warn!("Docker not available: {}", e);
                // Could show an alert here
                return;
            }

            // Create a devcontainer template with current directory
            let cwd = std::env::current_dir().ok();
            let template = cterm_app::config::StickyTabConfig::claude_devcontainer(cwd);

            self.open_template(&template);
            log::info!("Opened devcontainer tab");
        }

        /// Called by windows when they close to remove from tracking
        #[unsafe(method(windowDidClose:))]
        fn window_did_close(&self, window: &CtermWindow) {
            let mut windows = self.ivars().windows.borrow_mut();
            let initial_count = windows.len();

            // Remove the closed window from our tracking array
            windows.retain(|w| !std::ptr::eq(&**w, window));

            let removed = initial_count - windows.len();
            log::debug!(
                "Window closed, removed {} from tracking ({} remaining)",
                removed,
                windows.len()
            );

            // If no windows left, terminate the app
            if windows.is_empty() {
                drop(windows); // Release borrow before terminating
                log::info!("Last window closed, terminating app");
                let mtm = MainThreadMarker::from(self);
                let app = NSApplication::sharedApplication(mtm);
                app.terminate(None);
            }
        }

        /// Register a window for tracking (called by newWindowForTab: etc.)
        #[unsafe(method(registerWindow:))]
        fn register_window(&self, window: &CtermWindow) {
            // Convert raw pointer to Retained by retaining it
            let retained: Retained<CtermWindow> = unsafe {
                Retained::retain(window as *const _ as *mut CtermWindow).unwrap()
            };
            self.ivars().windows.borrow_mut().push(retained);
            log::debug!(
                "Registered window ({} total)",
                self.ivars().windows.borrow().len()
            );
        }

        /// Debug menu: Relaunch cterm with state preservation (uses real upgrade path)
        #[cfg(unix)]
        #[unsafe(method(debugRelaunch:))]
        fn action_debug_relaunch(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.perform_relaunch();
        }

        /// Debug menu: Show application logs
        #[unsafe(method(showLogs:))]
        fn action_show_logs(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let mtm = MainThreadMarker::from(self);
            crate::log_viewer::show_log_viewer(mtm);
        }

        /// Set log level to Error
        #[unsafe(method(setLogLevelError:))]
        fn action_set_log_level_error(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            log::set_max_level(log::LevelFilter::Error);
            log::info!("Log level set to Error");
        }

        /// Set log level to Warn
        #[unsafe(method(setLogLevelWarn:))]
        fn action_set_log_level_warn(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            log::set_max_level(log::LevelFilter::Warn);
            log::info!("Log level set to Warn");
        }

        /// Set log level to Info
        #[unsafe(method(setLogLevelInfo:))]
        fn action_set_log_level_info(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            log::set_max_level(log::LevelFilter::Info);
            log::info!("Log level set to Info");
        }

        /// Set log level to Debug
        #[unsafe(method(setLogLevelDebug:))]
        fn action_set_log_level_debug(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            log::set_max_level(log::LevelFilter::Debug);
            log::info!("Log level set to Debug");
        }

        /// Set log level to Trace
        #[unsafe(method(setLogLevelTrace:))]
        fn action_set_log_level_trace(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            log::set_max_level(log::LevelFilter::Trace);
            log::info!("Log level set to Trace");
        }
    }
);

impl AppDelegate {
    pub fn new(mtm: MainThreadMarker, config: Config, theme: Theme) -> Retained<Self> {
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(AppDelegateIvars {
            config,
            theme,
            windows: std::cell::RefCell::new(Vec::new()),
            #[cfg(unix)]
            last_state_hash: std::cell::Cell::new(0),
        });
        unsafe { msg_send![super(this), init] }
    }

    /// Open a tab from a template
    fn open_template(&self, template: &cterm_app::config::StickyTabConfig) {
        let mtm = MainThreadMarker::from(self);

        // If the template is unique, check if we already have a tab with this template
        if template.unique {
            // Look through all windows to find a matching tab
            let windows = self.ivars().windows.borrow();
            for window in windows.iter() {
                // Check if this window has a tab with the template name
                if let Some(terminal_view) = window.active_terminal() {
                    if terminal_view.template_name().as_deref() == Some(template.name.as_str()) {
                        // Focus this window
                        window.makeKeyAndOrderFront(None);
                        log::info!("Focused existing unique tab: {}", template.name);
                        return;
                    }
                }
            }
        }

        // Create a new tab from the template
        let window =
            CtermWindow::from_template(mtm, &self.ivars().config, &self.ivars().theme, template);
        self.ivars().windows.borrow_mut().push(window.clone());

        // If there's a key window, add as a tab to it; otherwise show standalone
        let app = NSApplication::sharedApplication(mtm);
        if let Some(key_window) = app.keyWindow() {
            // Add as a tab to the existing key window
            key_window.addTabbedWindow_ordered(&window, objc2_app_kit::NSWindowOrderingMode::Above);
            window.makeKeyAndOrderFront(None);
            log::info!(
                "Created new tab from template: {} (added to existing window)",
                template.name
            );
        } else {
            window.makeKeyAndOrderFront(None);
            log::info!(
                "Created new tab from template: {} (standalone window)",
                template.name
            );
        }

        // Apply tab color after window is visible (tab property is only available then)
        if let Some(ref color) = template.color {
            window.set_tab_color(Some(color));
        }
    }

    /// Restore windows from seamless upgrade state
    #[cfg(unix)]
    fn restore_from_upgrade(
        &self,
        mtm: MainThreadMarker,
        state: cterm_app::upgrade::UpgradeState,
        fds: Vec<std::os::unix::io::RawFd>,
    ) {
        use cterm_core::screen::{Screen, ScreenConfig};
        use cterm_core::term::Terminal;
        use cterm_core::Pty;
        use objc2_foundation::{NSPoint, NSRect, NSSize};

        log::info!("Reconstructing {} windows", state.windows.len());

        for (window_idx, window_state) in state.windows.into_iter().enumerate() {
            log::info!(
                "Window {}: {}x{} at ({}, {}), {} tabs",
                window_idx,
                window_state.width,
                window_state.height,
                window_state.x,
                window_state.y,
                window_state.tabs.len()
            );

            // Restore all tabs in this window
            let mut first_window: Option<Retained<CtermWindow>> = None;

            for (tab_idx, tab_state) in window_state.tabs.into_iter().enumerate() {
                // Get the PTY FD for this tab
                if tab_state.pty_fd_index >= fds.len() {
                    log::error!(
                        "PTY FD index {} out of range (max {})",
                        tab_state.pty_fd_index,
                        fds.len()
                    );
                    continue;
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

                // Restore window title
                if !tab_state.title.is_empty() {
                    window.setTitle(&NSString::from_str(&tab_state.title));
                }

                // Restore template_name if present
                if tab_state.template_name.is_some() {
                    if let Some(terminal_view) = window.active_terminal() {
                        terminal_view.set_template_name(tab_state.template_name.clone());
                    }
                }

                // Track this window
                self.ivars().windows.borrow_mut().push(window.clone());

                if let Some(ref first) = first_window {
                    // Add as a tab to the first window
                    first.addTabbedWindow_ordered(
                        &window,
                        objc2_app_kit::NSWindowOrderingMode::Above,
                    );
                    log::info!(
                        "Window {}, tab {} restored as tabbed window",
                        window_idx,
                        tab_idx
                    );
                } else {
                    // First tab - restore window position and size, then show it
                    let frame = NSRect::new(
                        NSPoint::new(window_state.x as f64, window_state.y as f64),
                        NSSize::new(window_state.width as f64, window_state.height as f64),
                    );
                    window.setFrame_display(frame, true);
                    window.makeKeyAndOrderFront(None);
                    first_window = Some(window.clone());
                    log::info!(
                        "Window {}, tab {} restored as main window",
                        window_idx,
                        tab_idx
                    );
                }
            }

            if first_window.is_some() {
                log::info!("Window {} restored successfully", window_idx);
            }
        }
    }

    /// Save crash recovery state to disk
    #[cfg(unix)]
    pub fn save_crash_state(&self) {
        use cterm_app::crash_recovery::{write_crash_state, CrashState};
        use cterm_app::upgrade::{TabUpgradeState, UpgradeState, WindowUpgradeState};

        let windows = self.ivars().windows.borrow();

        // Build upgrade state from all windows
        let mut upgrade_state = UpgradeState::new(env!("CARGO_PKG_VERSION"));

        for window in windows.iter() {
            let mut window_state = WindowUpgradeState::new();

            // Get window frame
            let frame = window.frame();
            window_state.x = frame.origin.x as i32;
            window_state.y = frame.origin.y as i32;
            window_state.width = frame.size.width as i32;
            window_state.height = frame.size.height as i32;

            // Get terminal state
            if let Some(terminal_view) = window.active_terminal() {
                let watchdog_fd_id = terminal_view.watchdog_fd_id();

                // Only save if registered with watchdog
                if watchdog_fd_id > 0 {
                    let terminal_state = terminal_view.export_state();

                    let terminal = terminal_view.terminal();
                    let term = terminal.lock();
                    let child_pid = term.child_pid().unwrap_or(0);
                    drop(term);

                    let mut tab_state = TabUpgradeState::with_watchdog_fd_id(
                        0, // Tab ID not used for crash recovery
                        0, // FD index not used (we use watchdog_fd_id instead)
                        child_pid,
                        watchdog_fd_id,
                    );
                    tab_state.terminal = terminal_state;
                    tab_state.title = window.title().to_string();
                    tab_state.template_name = terminal_view.template_name();

                    window_state.tabs.push(tab_state);
                }
            }

            if !window_state.tabs.is_empty() {
                upgrade_state.windows.push(window_state);
            }
        }

        // Only write if we have state to save
        if upgrade_state.windows.is_empty() {
            return;
        }

        // Compute a simple hash of the state to avoid redundant writes
        // We hash the debug representation which includes all fields
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}", upgrade_state).hash(&mut hasher);
        let state_hash = hasher.finish();

        // Skip if state hasn't changed
        let last_hash = self.ivars().last_state_hash.get();
        if state_hash == last_hash {
            return;
        }

        let crash_state = CrashState::new(upgrade_state);

        if let Err(e) = write_crash_state(&crash_state) {
            log::error!("Failed to save crash state: {}", e);
        } else {
            self.ivars().last_state_hash.set(state_hash);
            log::trace!(
                "Saved crash state: {} windows",
                crash_state.state.windows.len()
            );
        }
    }

    /// Start the periodic state saving timer
    #[cfg(unix)]
    pub fn start_state_save_timer(&self, mtm: MainThreadMarker) {
        use objc2::sel;
        use objc2_foundation::NSTimer;

        // Save state every 5 seconds
        let interval = 5.0;

        unsafe {
            // scheduledTimer... automatically adds to the current run loop which retains it
            let _timer = NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                interval,
                self,
                sel!(saveStateTimer:),
                None,
                true,
            );
        }

        log::info!("Started crash state save timer (interval: {}s)", interval);
    }

    /// Perform a seamless relaunch, preserving all windows and tabs
    ///
    /// This collects state from all windows, duplicates PTY file descriptors,
    /// and transfers everything to a new process running the same binary.
    ///
    /// For upgrades: replace the binary first, then call this method.
    /// For debug/testing: just call this method to relaunch with the same binary.
    #[cfg(unix)]
    pub fn perform_relaunch(&self) {
        use cterm_app::upgrade::{
            execute_upgrade, TabUpgradeState, UpgradeState, WindowUpgradeState,
        };
        use std::os::unix::io::RawFd;

        let binary = match std::env::current_exe() {
            Ok(path) => path,
            Err(e) => {
                log::error!("Failed to get current executable path: {}", e);
                return;
            }
        };

        log::info!("Performing seamless relaunch: {}", binary.display());

        let windows = self.ivars().windows.borrow();
        let mut upgrade_state = UpgradeState::new(env!("CARGO_PKG_VERSION"));
        let mut fds: Vec<RawFd> = Vec::new();

        for window in windows.iter() {
            let mut window_state = WindowUpgradeState::new();

            // Get window frame
            let frame = window.frame();
            window_state.x = frame.origin.x as i32;
            window_state.y = frame.origin.y as i32;
            window_state.width = frame.size.width as i32;
            window_state.height = frame.size.height as i32;

            // Get terminal state
            if let Some(terminal_view) = window.active_terminal() {
                let terminal_state = terminal_view.export_state();

                let terminal = terminal_view.terminal();
                let term = terminal.lock();
                let child_pid = term.child_pid().unwrap_or(0);
                drop(term);

                // Duplicate the PTY FD
                if let Some(fd) = terminal_view.dup_pty_fd() {
                    let fd_index = fds.len();
                    fds.push(fd);

                    let mut tab_state = TabUpgradeState::new(
                        0, // Tab ID not needed
                        fd_index, child_pid,
                    );
                    tab_state.terminal = terminal_state;
                    tab_state.title = window.title().to_string();
                    tab_state.template_name = terminal_view.template_name();

                    window_state.tabs.push(tab_state);
                } else {
                    log::warn!("Failed to duplicate PTY FD for terminal");
                }
            }

            if !window_state.tabs.is_empty() {
                upgrade_state.windows.push(window_state);
            }
        }

        // Release windows borrow before potentially exiting
        drop(windows);

        if upgrade_state.windows.is_empty() {
            log::warn!("No terminals to preserve in relaunch");
            return;
        }

        log::info!(
            "Relaunch state collected: {} windows, {} FDs",
            upgrade_state.windows.len(),
            fds.len()
        );

        // Execute relaunch
        match execute_upgrade(&binary, &upgrade_state, &fds) {
            Ok(()) => {
                log::info!("Relaunch successful, terminating old process");
                // Terminate this process - the new one has taken over
                let mtm = MainThreadMarker::from(self);
                let app = NSApplication::sharedApplication(mtm);
                app.terminate(None);
            }
            Err(e) => {
                log::error!("Relaunch failed: {}", e);
                // Close the duplicated FDs on failure
                for fd in fds {
                    unsafe { libc::close(fd) };
                }
            }
        }
    }
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

/// Run the native macOS application
pub fn run() {
    // Parse command-line arguments first
    let args = Args::parse();

    // Initialize logging with capture buffer for in-app log viewing
    crate::log_capture::init();

    // Install signal handler for better crash debugging
    #[cfg(unix)]
    unsafe {
        use std::io::Write;
        extern "C" fn crash_handler(sig: libc::c_int) {
            let _ = writeln!(std::io::stderr(), "\n=== CRASH: Signal {} ===", sig);
            let bt = std::backtrace::Backtrace::force_capture();
            let _ = writeln!(std::io::stderr(), "{}", bt);
            std::process::abort();
        }
        // Cast to function pointer type first, then to sighandler_t
        let handler: extern "C" fn(libc::c_int) = crash_handler;
        libc::signal(libc::SIGSEGV, handler as libc::sighandler_t);
        libc::signal(libc::SIGBUS, handler as libc::sighandler_t);
    }

    log::info!("Starting cterm (native macOS)");

    // Check if we're in upgrade receiver mode
    #[cfg(unix)]
    if let Some(fd) = args.upgrade_receiver {
        log::info!("Running in upgrade receiver mode with FD {}", fd);
        let exit_code = crate::upgrade_receiver::run_receiver(fd);
        std::process::exit(exit_code);
    }

    // Check if we should start with watchdog for crash recovery
    #[cfg(unix)]
    if args.supervised.is_none() && args.crash_recovery.is_none() && !args.no_watchdog {
        // We're not supervised and watchdog is not disabled - start watchdog
        log::info!("Starting watchdog for crash recovery...");

        let binary = std::env::current_exe().expect("Failed to get current executable");
        let other_args: Vec<String> = std::env::args().skip(1).collect();

        match cterm_app::run_watchdog(&binary, &other_args) {
            Ok(exit_code) => std::process::exit(exit_code),
            Err(e) => {
                log::error!("Watchdog failed: {}, running without crash recovery", e);
                // Fall through to normal startup
            }
        }
    }

    // Handle crash recovery mode - receive FDs from watchdog
    #[cfg(unix)]
    let recovery_fds = if let Some(fd) = args.crash_recovery {
        log::info!("Running in crash recovery mode (FD {})", fd);
        WATCHDOG_FD.store(fd, std::sync::atomic::Ordering::SeqCst);

        match cterm_app::receive_recovery_fds(fd) {
            Ok(fds) => {
                log::info!("Received {} PTY FDs for recovery", fds.len());
                Some(fds)
            }
            Err(e) => {
                log::error!("Failed to receive recovery FDs: {}", e);
                None
            }
        }
    } else {
        None
    };

    #[cfg(unix)]
    if let Some(fd) = args.supervised {
        log::info!("Running under watchdog supervision (FD {})", fd);
        // Store watchdog FD for later use (registering PTYs, shutdown notification)
        WATCHDOG_FD.store(fd, std::sync::atomic::Ordering::SeqCst);
    }

    // Store recovery FDs for use during window creation
    #[cfg(unix)]
    if let Some(fds) = recovery_fds {
        RECOVERY_FDS.with(|r| {
            *r.borrow_mut() = fds;
        });
    }

    // Store args for later access
    let _ = APP_ARGS.set(args);

    run_app_internal();
}

/// Internal function to run the Cocoa application
/// Called by both run() and upgrade_receiver after setup
pub fn run_app_internal() {
    // Get main thread marker - this must be called on the main thread
    let mtm = MainThreadMarker::new().expect("Must be called on main thread");

    // Load configuration
    let config = load_config().unwrap_or_else(|e| {
        log::warn!("Failed to load config, using defaults: {}", e);
        Config::default()
    });

    // Get theme
    let theme = get_theme(&config);

    // Get the shared application instance
    let app = NSApplication::sharedApplication(mtm);

    // Set activation policy to regular (shows in Dock)
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    // Create and set the application delegate
    let delegate = AppDelegate::new(mtm, config, theme);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    // Create the menu bar
    let menu_bar = menu::create_menu_bar(mtm);
    app.setMainMenu(Some(&menu_bar));

    log::info!("Starting main run loop");

    // Run the main event loop
    app.run();
}
