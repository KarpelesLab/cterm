//! Application setup and management for macOS
//!
//! Handles NSApplication lifecycle and main event loop.

use clap::Parser;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use std::path::PathBuf;

use cterm_app::config::{load_config, Config, RemoteConfig};
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

    /// Receive upgrade state from a file path (internal use)
    #[arg(long, hide = true)]
    pub upgrade_state: Option<String>,
}

/// Global application arguments (accessible from window creation)
static APP_ARGS: std::sync::OnceLock<Args> = std::sync::OnceLock::new();

/// Thread-local storage for upgrade state (used during seamless upgrade)
thread_local! {
    static UPGRADE_STATE: std::cell::RefCell<Option<cterm_app::upgrade::UpgradeState>> =
        const { std::cell::RefCell::new(None) };
}

/// Take upgrade state (consumes it)
pub fn take_upgrade_state() -> Option<cterm_app::upgrade::UpgradeState> {
    UPGRADE_STATE.with(|s| s.borrow_mut().take())
}

/// Store upgrade state for use during app launch
pub fn set_upgrade_state(state: cterm_app::upgrade::UpgradeState) {
    UPGRADE_STATE.with(|s| {
        *s.borrow_mut() = Some(state);
    });
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
    /// Manages cached SSH connections to remote ctermd instances
    remote_manager: cterm_client::RemoteManager,
    /// Set to true during relaunch to skip close confirmation
    is_relaunching: std::cell::Cell<bool>,
    /// Count of windows with active bell notifications
    bell_count: std::cell::Cell<u32>,
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

            // Check for seamless upgrade state
            if let Some(upgrade_state) = take_upgrade_state() {
                log::info!(
                    "Restoring {} window(s) from upgrade state",
                    upgrade_state.windows.len()
                );
                let config = self.ivars().config.clone();
                let theme = self.ivars().theme.clone();

                // Reconnect to daemon sessions from upgrade state
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();

                if let Ok(rt) = rt {
                    for window_state in upgrade_state.windows {
                        for tab_state in &window_state.tabs {
                            if let Some(ref session_id) = tab_state.session_id {
                                match rt.block_on(async {
                                    let conn = cterm_client::DaemonConnection::connect_local().await?;
                                    conn.attach_session(session_id, 80, 24).await
                                }) {
                                    Ok((handle, screen)) => {
                                        let recon = cterm_app::daemon_reconnect::ReconnectedSession {
                                            handle,
                                            title: tab_state.title.clone(),
                                            custom_title: tab_state.custom_title.clone().unwrap_or_default(),
                                            tab_color: tab_state.color.clone().unwrap_or_default(),
                                            template_name: tab_state.template_name.clone().unwrap_or_default(),
                                            screen,
                                        };
                                        let window = CtermWindow::from_daemon_with_screen(mtm, &config, &theme, recon);

                                        // Restore window frame from saved state
                                        let frame = NSRect::new(
                                            NSPoint::new(window_state.x as f64, window_state.y as f64),
                                            NSSize::new(window_state.width as f64, window_state.height as f64),
                                        );
                                        window.setFrame_display(frame, true);

                                        // Restore tab color
                                        if let Some(ref color) = tab_state.color {
                                            window.set_tab_color(Some(color));
                                        }

                                        // Restore template name
                                        if let Some(ref tpl_name) = tab_state.template_name {
                                            if let Some(tv) = window.active_terminal() {
                                                tv.set_template_name(Some(tpl_name.clone()));
                                            }
                                        }

                                        // Restore fullscreen state
                                        if window_state.fullscreen {
                                            window.toggleFullScreen(None);
                                        }

                                        self.ivars().windows.borrow_mut().push(window.clone());
                                        window.makeKeyAndOrderFront(None);
                                    }
                                    Err(e) => {
                                        log::error!("Failed to reconnect session {}: {}", session_id, e);
                                    }
                                }
                            }
                        }
                    }

                    if !self.ivars().windows.borrow().is_empty() {
                        #[allow(deprecated)]
                        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
                        return;
                    }
                }
            }

            // Try to reconnect to existing daemon sessions
            {
                let config = self.ivars().config.clone();
                let theme = self.ivars().theme.clone();

                // Check if daemon has existing sessions (non-blocking, don't auto-start)
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();

                if let Ok(rt) = rt {
                    let check = rt.block_on(cterm_app::daemon_reconnect::check_daemon_sessions());
                    if let cterm_app::daemon_reconnect::ReconnectCheck::Available(sessions) = check {
                        let running_count = sessions.iter().filter(|s| s.running).count();
                        if running_count > 0 {
                            log::info!("Found {} running daemon sessions, reconnecting...", running_count);
                            // Reconnect to all sessions
                            let reconnected = rt.block_on(cterm_app::daemon_reconnect::reconnect_all_sessions());
                            if let Ok(reconnected) = reconnected {
                                if !reconnected.is_empty() {
                                    let mut first = true;
                                    for recon in reconnected {
                                        let window = CtermWindow::from_daemon_with_screen(
                                            mtm, &config, &theme, recon,
                                        );
                                        self.ivars().windows.borrow_mut().push(window.clone());
                                        if first {
                                            window.makeKeyAndOrderFront(None);
                                            first = false;
                                        } else {
                                            // Add as tab to first window
                                            if let Some(first_win) = self.ivars().windows.borrow().first().cloned() {
                                                first_win.addTabbedWindow_ordered(
                                                    &window,
                                                    objc2_app_kit::NSWindowOrderingMode::Above,
                                                );
                                            }
                                            window.makeKeyAndOrderFront(None);
                                        }
                                    }
                                    // Skip normal startup since we reconnected to existing sessions
                                    #[allow(deprecated)]
                                    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
                                    log::info!("Reconnected to daemon sessions, skipping normal startup");
                                    return;
                                }
                            }
                        }
                    }
                }
            }

            // Normal startup - create the main window
            log::debug!("Creating main window...");
            let window = CtermWindow::new(mtm, &self.ivars().config, &self.ivars().theme);
            log::debug!("Main window created");

            // Store window reference
            self.ivars().windows.borrow_mut().push(window.clone());
            log::debug!("Window stored in windows list");

            // Show the window
            window.makeKeyAndOrderFront(None);
            log::info!("Window shown (makeKeyAndOrderFront)");

            // Activate the app to bring window to front
            #[allow(deprecated)]
            NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
            log::debug!("App activated");
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _sender: &NSApplication) -> bool {
            true
        }

        #[unsafe(method(applicationShouldTerminate:))]
        fn application_should_terminate(
            &self,
            _sender: &NSApplication,
        ) -> objc2_app_kit::NSApplicationTerminateReply {
            use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSApplicationTerminateReply};

            // Skip confirmation during relaunch
            if self.ivars().is_relaunching.get() {
                return NSApplicationTerminateReply::TerminateNow;
            }

            // Check if config says to confirm close with running processes
            if !self.ivars().config.general.confirm_close_with_running {
                return NSApplicationTerminateReply::TerminateNow;
            }

            // Collect all windows with running processes
            #[cfg(unix)]
            let running_processes: Vec<String> = {
                let windows = self.ivars().windows.borrow();
                windows
                    .iter()
                    .filter_map(|window| {
                        if let Some(terminal) = window.active_terminal() {
                            if terminal.has_foreground_process() {
                                return terminal.foreground_process_name();
                            }
                        }
                        None
                    })
                    .collect()
            };

            #[cfg(not(unix))]
            let running_processes: Vec<String> = Vec::new();

            if running_processes.is_empty() {
                return NSApplicationTerminateReply::TerminateNow;
            }

            // Show confirmation dialog
            let mtm = MainThreadMarker::from(self);
            let alert = NSAlert::new(mtm);

            let message = if running_processes.len() == 1 {
                format!("\"{}\" is still running", running_processes[0])
            } else {
                format!("{} processes are still running", running_processes.len())
            };

            alert.setMessageText(&NSString::from_str(&message));
            alert.setInformativeText(&NSString::from_str(
                "Quitting will terminate the running process(es). Are you sure you want to quit?",
            ));
            alert.setAlertStyle(NSAlertStyle::Warning);

            alert.addButtonWithTitle(&NSString::from_str("Quit"));
            alert.addButtonWithTitle(&NSString::from_str("Cancel"));

            let response = alert.runModal();
            if response == NSAlertFirstButtonReturn {
                NSApplicationTerminateReply::TerminateNow
            } else {
                NSApplicationTerminateReply::TerminateCancel
            }
        }
    }

    // Menu action handlers
    impl AppDelegate {
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
            let remote_names: Vec<String> = self
                .ivars()
                .config
                .remotes
                .iter()
                .map(|r| r.name.clone())
                .collect();
            crate::tab_templates::show_tab_templates(mtm, templates, remote_names);
        }

        #[unsafe(method(checkForUpdates:))]
        fn action_check_for_updates(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let mtm = MainThreadMarker::from(self);
            crate::update_dialog::check_for_updates_sync(mtm);
        }

        #[unsafe(method(showQuickOpen:))]
        fn action_show_quick_open(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let mtm = MainThreadMarker::from(self);
            let app = NSApplication::sharedApplication(mtm);

            // Get the key window
            if let Some(key_window) = app.keyWindow() {
                // Check if it's a CtermWindow
                let is_cterm: bool = unsafe { msg_send![&key_window, isKindOfClass: objc2::class!(CtermWindow)] };
                if is_cterm {
                    let cterm_window: &CtermWindow = unsafe { &*(&*key_window as *const NSWindow as *const CtermWindow) };
                    cterm_window.show_quick_open();
                }
            }
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

        #[unsafe(method(runToolShortcut:))]
        fn action_run_tool_shortcut(&self, sender: Option<&objc2::runtime::AnyObject>) {
            use objc2_app_kit::NSMenuItem;

            if let Some(sender) = sender {
                let item: &NSMenuItem = unsafe { &*(sender as *const _ as *const NSMenuItem) };
                let index = item.tag() as usize;

                if let Ok(shortcuts) = cterm_app::config::load_tool_shortcuts() {
                    if let Some(shortcut) = shortcuts.get(index) {
                        // Get CWD from active terminal in the key window
                        let mtm = MainThreadMarker::from(self);
                        let app = NSApplication::sharedApplication(mtm);
                        let cwd = app.keyWindow().and_then(|key_window| {
                            let is_cterm: bool = unsafe {
                                msg_send![&key_window, isKindOfClass: objc2::class!(CtermWindow)]
                            };
                            if is_cterm {
                                let cterm_window: &CtermWindow = unsafe {
                                    &*(&*key_window as *const NSWindow as *const CtermWindow)
                                };
                                #[cfg(unix)]
                                {
                                    cterm_window
                                        .active_terminal()
                                        .and_then(|t| t.foreground_cwd())
                                }
                                #[cfg(not(unix))]
                                {
                                    let _ = cterm_window;
                                    None
                                }
                            } else {
                                None
                            }
                        });

                        let cwd = cwd.unwrap_or_else(|| {
                            std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
                        });

                        if let Err(e) =
                            shortcut.execute(std::path::Path::new(&cwd))
                        {
                            // Show error alert
                            let alert = objc2_app_kit::NSAlert::new(mtm);
                            alert.setMessageText(&NSString::from_str(&format!(
                                "Failed to launch \"{}\"",
                                shortcut.name
                            )));
                            alert.setInformativeText(&NSString::from_str(&format!(
                                "Command '{}' failed: {}",
                                shortcut.command, e
                            )));
                            alert.setAlertStyle(
                                objc2_app_kit::NSAlertStyle::Warning,
                            );
                            alert.addButtonWithTitle(&NSString::from_str("OK"));
                            alert.runModal();
                        }
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

        /// Close all tabs except the current one
        #[unsafe(method(closeOtherTabs:))]
        fn action_close_other_tabs(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let mtm = MainThreadMarker::from(self);
            let app = NSApplication::sharedApplication(mtm);

            // Get current key window
            if let Some(key_window) = app.keyWindow() {
                // Get all tabbed windows
                let tabbed_windows: Option<objc2::rc::Retained<objc2_foundation::NSArray<NSWindow>>> =
                    unsafe { msg_send![&key_window, tabbedWindows] };

                if let Some(windows) = tabbed_windows {
                    let current_ptr = objc2::rc::Retained::as_ptr(&key_window);
                    let windows_to_close: Vec<_> = windows
                        .iter()
                        .filter(|w| objc2::rc::Retained::as_ptr(w) != current_ptr)
                        .collect();

                    for window in windows_to_close {
                        // Use performClose to trigger windowShouldClose: for process check
                        window.performClose(None);
                    }
                    log::info!("Closed other tabs");
                }
            }
        }

        /// Select the next tab that has an active bell indicator
        #[unsafe(method(selectNextAlertedTab:))]
        fn action_select_next_alerted_tab(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let mtm = MainThreadMarker::from(self);
            let app = NSApplication::sharedApplication(mtm);

            if let Some(key_window) = app.keyWindow() {
                let tabbed_windows: Option<
                    objc2::rc::Retained<objc2_foundation::NSArray<NSWindow>>,
                > = unsafe { msg_send![&key_window, tabbedWindows] };

                if let Some(windows) = tabbed_windows {
                    let count = windows.len();
                    if count == 0 {
                        return;
                    }

                    // Find current window's index
                    let current_ptr = objc2::rc::Retained::as_ptr(&key_window);
                    let current_index = windows
                        .iter()
                        .position(|w| objc2::rc::Retained::as_ptr(&w) == current_ptr)
                        .unwrap_or(0);

                    // Search from current_index + 1, wrapping around
                    for offset in 1..count {
                        let idx = (current_index + offset) % count;
                        if let Some(window) = windows.iter().nth(idx) {
                            let window_ptr =
                                objc2::rc::Retained::as_ptr(&window) as *const CtermWindow;
                            let cterm_window: &CtermWindow = unsafe { &*window_ptr };
                            if cterm_window.has_bell() {
                                window.makeKeyAndOrderFront(None);
                                log::debug!("Switched to alerted tab at index {}", idx);
                                return;
                            }
                        }
                    }

                    log::debug!("No alerted tabs found");
                }
            }
        }

        /// Select a tab by number (1-8 for specific tabs, 9 for last tab)
        #[unsafe(method(selectTabByNumber:))]
        fn action_select_tab_by_number(&self, sender: Option<&objc2::runtime::AnyObject>) {
            let Some(sender) = sender else { return };
            let mtm = MainThreadMarker::from(self);
            let app = NSApplication::sharedApplication(mtm);

            // Get the tag (1-9) from the menu item
            let tag: isize = unsafe { msg_send![sender, tag] };

            // Get current key window
            if let Some(key_window) = app.keyWindow() {
                // Get all tabbed windows
                let tabbed_windows: Option<objc2::rc::Retained<objc2_foundation::NSArray<NSWindow>>> =
                    unsafe { msg_send![&key_window, tabbedWindows] };

                if let Some(windows) = tabbed_windows {
                    let count = windows.len();
                    if count == 0 {
                        return;
                    }

                    // Determine which tab to select
                    let index = if tag == 9 {
                        // Cmd+9 selects the last tab
                        count - 1
                    } else {
                        // Cmd+1 through Cmd+8 select tabs 0-7
                        (tag as usize).saturating_sub(1)
                    };

                    // Select the tab if it exists
                    if let Some(window) = windows.iter().nth(index) {
                        window.makeKeyAndOrderFront(None);
                        log::debug!("Selected tab {}", index + 1);
                    }
                }
            }
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

        /// Show session picker and attach to a daemon session
        #[unsafe(method(attachToSession:))]
        fn action_attach_to_session(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let mtm = MainThreadMarker::from(self);
            let config = self.ivars().config.clone();
            let theme = self.ivars().theme.clone();

            // Run session listing in background
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();

                let sessions = match rt {
                    Ok(rt) => rt.block_on(async {
                        let conn = cterm_client::DaemonConnection::connect_local().await?;
                        conn.list_sessions().await
                    }),
                    Err(e) => {
                        log::error!("Failed to create runtime: {}", e);
                        return;
                    }
                };

                match sessions {
                    Ok(sessions) => {
                        // For now, attach to the first running session
                        if let Some(session_info) = sessions.iter().find(|s| s.running) {
                            let session_id = session_info.session_id.clone();
                            let cols = session_info.cols;
                            let rows = session_info.rows;

                            let rt = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build();

                            if let Ok(rt) = rt {
                                match rt.block_on(async {
                                    let conn =
                                        cterm_client::DaemonConnection::connect_local().await?;
                                    let (handle, _) =
                                        conn.attach_session(&session_id, cols, rows).await?;
                                    Ok::<_, cterm_client::ClientError>(handle)
                                }) {
                                    Ok(handle) => {
                                        // Create the tab on the main thread
                                        dispatch2::Queue::main().exec_async(move || {
                                            let mtm = unsafe {
                                                MainThreadMarker::new_unchecked()
                                            };
                                            let window = CtermWindow::from_daemon(
                                                mtm, &config, &theme, handle,
                                            );
                                            window.makeKeyAndOrderFront(None);
                                            let app = NSApplication::sharedApplication(mtm);
                                            if let Some(delegate) = app.delegate() {
                                                let _: () = unsafe {
                                                    msg_send![&*delegate, registerWindow: &*window]
                                                };
                                            }
                                        });
                                    }
                                    Err(e) => log::error!("Failed to attach: {}", e),
                                }
                            }
                        } else {
                            log::info!("No running daemon sessions to attach to");
                        }
                    }
                    Err(e) => log::error!("Failed to list sessions: {}", e),
                }
            });
        }

        /// Show SSH connection dialog
        #[unsafe(method(sshConnect:))]
        fn action_ssh_connect(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let mtm = MainThreadMarker::from(self);
            let remote_manager = self.ivars().remote_manager.clone();
            let config = self.ivars().config.clone();
            let theme = self.ivars().theme.clone();

            // Show a simple input dialog for the SSH host
            let alert = objc2_app_kit::NSAlert::new(mtm);
            alert.setMessageText(&NSString::from_str("SSH Remote Terminal"));
            alert.setInformativeText(&NSString::from_str("Enter host (e.g. user@hostname):"));
            alert.addButtonWithTitle(&NSString::from_str("Connect"));
            alert.addButtonWithTitle(&NSString::from_str("Cancel"));

            let input = unsafe {
                let field = objc2_app_kit::NSTextField::new(mtm);
                field.setFrame(objc2_foundation::NSRect::new(
                    objc2_foundation::NSPoint::new(0.0, 0.0),
                    objc2_foundation::NSSize::new(300.0, 24.0),
                ));
                field.setPlaceholderString(Some(&NSString::from_str("user@hostname")));
                alert.setAccessoryView(Some(&field));
                field
            };

            let response = unsafe { alert.runModal() };
            if response != objc2_app_kit::NSAlertFirstButtonReturn {
                return;
            }

            let host = input.stringValue().to_string();
            if host.is_empty() {
                return;
            }

            log::info!("SSH connect to: {}", host);

            let app = NSApplication::sharedApplication(mtm);
            let key_window = app.keyWindow();
            let window_ptr = key_window
                .as_ref()
                .map(|w| Retained::as_ptr(w) as *const CtermWindow as usize);

            // Connect to remote daemon in background, attach to all existing sessions
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();

                let result = match rt {
                    Ok(rt) => rt.block_on(async {
                        let (conn, _tunnel) =
                            cterm_client::DaemonConnection::connect_ssh(&host, true).await?;

                        // Attach to all existing running sessions
                        let mut sessions =
                            cterm_app::daemon_reconnect::reconnect_all_sessions_on(&conn)
                                .await?;

                        // If no sessions exist, create one
                        if sessions.is_empty() {
                            let handle = conn
                                .create_session(cterm_client::CreateSessionOpts {
                                    cols: 80,
                                    rows: 24,
                                    ..Default::default()
                                })
                                .await?;
                            sessions.push(cterm_app::daemon_reconnect::ReconnectedSession {
                                handle,
                                title: String::new(),
                                custom_title: String::new(),
                                tab_color: String::new(),
                                template_name: String::new(),
                                screen: None,
                            });
                        }

                        Ok::<_, cterm_client::ClientError>(sessions)
                    }),
                    Err(e) => Err(cterm_client::ClientError::Connection(e.to_string())),
                };

                match result {
                    Ok(sessions) => {
                        let session_count = sessions.len();
                        dispatch2::Queue::main().exec_async(move || {
                            let mtm = unsafe { MainThreadMarker::new_unchecked() };

                            for recon in sessions {
                                let title = if !recon.custom_title.is_empty() {
                                    recon.custom_title.clone()
                                } else if !recon.title.is_empty() {
                                    recon.title.clone()
                                } else {
                                    host.clone()
                                };

                                let tab_color = if recon.tab_color.is_empty() {
                                    Some("#22c55e".to_string()) // Green for remote
                                } else {
                                    Some(recon.tab_color.clone())
                                };

                                let window = CtermWindow::from_daemon_with_screen(
                                    mtm, &config, &theme, recon,
                                );
                                window.setTitle(&NSString::from_str(&title));

                                if let Some(ref c) = tab_color {
                                    window.set_tab_color(Some(c));
                                }

                                // Add as tab to the current key window
                                if let Some(ptr) = window_ptr {
                                    let parent: &CtermWindow =
                                        unsafe { &*(ptr as *const CtermWindow) };
                                    parent.addTabbedWindow_ordered(
                                        &window,
                                        objc2_app_kit::NSWindowOrderingMode::Above,
                                    );
                                }
                                window.makeKeyAndOrderFront(None);

                                let app = NSApplication::sharedApplication(mtm);
                                if let Some(delegate) = app.delegate() {
                                    let _: () = unsafe {
                                        msg_send![&*delegate, registerWindow: &*window]
                                    };
                                }
                            }

                            log::info!(
                                "SSH connected to {}: attached {} session(s)",
                                host,
                                session_count
                            );
                        });
                    }
                    Err(e) => {
                        log::error!("Failed to connect via SSH: {}", e);
                    }
                }
            });
        }

        /// Show remotes management dialog
        #[unsafe(method(manageRemotes:))]
        fn action_manage_remotes(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let mtm = MainThreadMarker::from(self);
            let config = self.ivars().config.clone();
            crate::remotes_dialog::show_remotes_dialog(mtm, config);
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

        /// Debug menu: Relaunch ctermd daemon (exec-in-place, preserving PTY FDs)
        #[unsafe(method(debugRelaunchDaemon:))]
        fn action_debug_relaunch_daemon(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            log::info!("Debug: Requesting ctermd relaunch");
            // Send the relaunch request in a background thread
            std::thread::spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create tokio runtime");
                rt.block_on(async {
                    let socket_path = cterm_client::default_socket_path();
                    match cterm_client::DaemonConnection::connect_unix(&socket_path, false).await {
                        Ok(conn) => {
                            // The daemon will exec() so the connection will drop.
                            // We ignore the error from the dropped connection.
                            match conn.relaunch_daemon("").await {
                                Ok(resp) => {
                                    if resp.success {
                                        log::info!("ctermd relaunch succeeded");
                                    } else {
                                        log::error!("ctermd relaunch failed: {}", resp.reason);
                                    }
                                }
                                Err(e) => {
                                    // Connection drop during exec is expected
                                    log::info!("ctermd relaunch in progress (connection dropped: {})", e);
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to connect to ctermd for relaunch: {}", e);
                        }
                    }
                });
            });
        }

        /// Debug menu: Kill local ctermd daemon (force shutdown)
        #[unsafe(method(killLocalDaemon:))]
        fn action_kill_local_daemon(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            log::info!("Debug: Requesting ctermd force shutdown");
            std::thread::spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("Failed to create tokio runtime");
                rt.block_on(async {
                    let socket_path = cterm_client::default_socket_path();
                    match cterm_client::DaemonConnection::connect_unix(&socket_path, false).await {
                        Ok(conn) => match conn.shutdown(true).await {
                            Ok(resp) => {
                                if resp.success {
                                    log::info!("ctermd shutdown succeeded");
                                } else {
                                    log::error!("ctermd shutdown failed: {}", resp.reason);
                                }
                            }
                            Err(e) => {
                                log::info!(
                                    "ctermd shutdown in progress (connection dropped: {})",
                                    e
                                );
                            }
                        },
                        Err(e) => {
                            log::error!("Failed to connect to ctermd for shutdown: {}", e);
                        }
                    }
                });
            });
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
            remote_manager: cterm_client::RemoteManager::new(),
            is_relaunching: std::cell::Cell::new(false),
            bell_count: std::cell::Cell::new(0),
        });
        unsafe { msg_send![super(this), init] }
    }

    /// Increment the bell count and update dock badge
    pub fn increment_bell_count(&self) {
        let count = self.ivars().bell_count.get() + 1;
        self.ivars().bell_count.set(count);
        self.update_dock_badge(count);
    }

    /// Decrement the bell count and update dock badge
    pub fn decrement_bell_count(&self) {
        let count = self.ivars().bell_count.get().saturating_sub(1);
        self.ivars().bell_count.set(count);
        self.update_dock_badge(count);
    }

    /// Update the dock badge with the current bell count
    fn update_dock_badge(&self, count: u32) {
        let mtm = MainThreadMarker::from(self);
        let app = NSApplication::sharedApplication(mtm);
        unsafe {
            let dock_tile: Retained<objc2::runtime::AnyObject> = msg_send![&app, dockTile];
            if count > 0 {
                let badge = NSString::from_str(&count.to_string());
                let _: () = msg_send![&dock_tile, setBadgeLabel: &*badge];
            } else {
                let null: *const NSString = std::ptr::null();
                let _: () = msg_send![&dock_tile, setBadgeLabel: null];
            }
        }
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

        // Prepare working directory (clone from git if needed)
        if let Some(ref working_dir) = template.working_directory {
            if let Err(e) =
                cterm_app::prepare_working_directory(working_dir, template.git_remote.as_deref())
            {
                log::error!("Failed to prepare working directory: {}", e);
            }
        }

        let config = self.ivars().config.clone();
        let theme = self.ivars().theme.clone();

        let opts = cterm_client::CreateSessionOpts {
            cols: 80,
            rows: 24,
            shell: template
                .command
                .clone()
                .or_else(|| config.general.default_shell.clone()),
            args: if template.args.is_empty() && template.command.is_none() {
                config.general.shell_args.clone()
            } else {
                template.args.clone()
            },
            cwd: template
                .working_directory
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            env: template
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            ..Default::default()
        };

        let template_name = template.name.clone();
        let template_color = template.color.clone();
        let template_bg_color = template.background_color.clone();

        // Resolve remote connection info if template targets a remote host
        let remote_cfg = template
            .remote
            .as_ref()
            .and_then(|name| config.find_remote(name).cloned());

        if let Some(ref remote_name) = template.remote {
            if remote_cfg.is_none() {
                log::error!(
                    "Template '{}' references unknown remote '{}'",
                    template.name,
                    remote_name
                );
            }
        }

        // Check if this is a mosh connection
        let is_mosh = remote_cfg
            .as_ref()
            .is_some_and(|r| r.method == cterm_app::config::ConnectionMethod::Mosh);

        let app = NSApplication::sharedApplication(mtm);

        if is_mosh {
            let remote = remote_cfg.unwrap();
            let mosh_config = mosh_config_from_remote(&remote, 80, 24);

            if let Some(key_window) = app.keyWindow() {
                let window_ptr = Retained::as_ptr(&key_window) as *const CtermWindow;
                let cterm_window: &CtermWindow = unsafe { &*window_ptr };
                cterm_window.spawn_mosh_tab(
                    mosh_config,
                    Some(template_name),
                    template_color,
                    template_bg_color,
                );
            } else {
                // Spawn mosh in background, create window on completion
                let config_clone = config.clone();
                let theme_clone = theme.clone();
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build();
                    let result = match rt {
                        Ok(rt) => rt.block_on(async {
                            cterm_mosh::MoshSession::connect(mosh_config).await
                        }),
                        Err(e) => Err(cterm_mosh::MoshError::SshFailed(e.to_string())),
                    };
                    match result {
                        Ok(session) => {
                            dispatch2::Queue::main().exec_async(move || {
                                let mtm = unsafe { MainThreadMarker::new_unchecked() };
                                let window = CtermWindow::from_mosh(
                                    mtm,
                                    &config_clone,
                                    &theme_clone,
                                    session,
                                );
                                window.setTitle(&NSString::from_str(&template_name));
                                if let Some(ref c) = template_color {
                                    window.set_tab_color(Some(c));
                                }
                                window.makeKeyAndOrderFront(None);

                                let app = NSApplication::sharedApplication(mtm);
                                if let Some(delegate) = app.delegate() {
                                    let _: () =
                                        unsafe { msg_send![&*delegate, registerWindow: &*window] };
                                }
                            });
                        }
                        Err(e) => {
                            log::error!("Failed to create mosh session: {}", e);
                        }
                    }
                });
            }
        } else {
            // Daemon connection (original path)
            let remote = remote_cfg.and_then(|rc| {
                template.remote.as_ref().map(|name| {
                    (
                        self.ivars().remote_manager.clone(),
                        name.clone(),
                        rc.host.clone(),
                        rc.ssh_compression,
                    )
                })
            });

            if let Some(key_window) = app.keyWindow() {
                let window_ptr = Retained::as_ptr(&key_window) as *const CtermWindow;
                let cterm_window: &CtermWindow = unsafe { &*window_ptr };
                cterm_window.spawn_daemon_tab(
                    opts,
                    Some(template_name),
                    template_color,
                    template_bg_color,
                    remote,
                    None,
                );
            } else {
                // No key window — create a new standalone daemon-backed window
                let window = CtermWindow::new_daemon(
                    mtm,
                    &config,
                    &theme,
                    opts,
                    template_name,
                    template_color,
                    template_bg_color,
                );
                self.ivars().windows.borrow_mut().push(window.clone());
                window.makeKeyAndOrderFront(None);
            }
        }
    }

    /// Perform a seamless relaunch, preserving all windows and tabs
    ///
    /// Since all terminal sessions live in the ctermd daemon, upgrading only
    /// requires saving the window/tab layout with session IDs. The daemon
    /// keeps sessions alive across cterm restarts.
    #[cfg(unix)]
    pub fn perform_relaunch(&self) {
        use cterm_app::upgrade::{
            execute_upgrade, TabUpgradeState, UpgradeState, WindowUpgradeState,
        };

        let binary = match std::env::current_exe() {
            Ok(path) => path,
            Err(e) => {
                log::error!("Failed to get current executable path: {}", e);
                return;
            }
        };

        log::info!("Performing seamless relaunch: {}", binary.display());

        let mut upgrade_state = UpgradeState::new();

        // Get windows in tab order using macOS native tabbedWindows
        let windows = self.ivars().windows.borrow();
        let ordered_windows: Vec<Retained<CtermWindow>> =
            if let Some(first_window) = windows.first() {
                let tabbed: Option<Retained<objc2_foundation::NSArray<NSWindow>>> =
                    unsafe { msg_send![&**first_window, tabbedWindows] };

                if let Some(tabbed_windows) = tabbed {
                    tabbed_windows
                        .iter()
                        .filter_map(|nswin| {
                            let nswin_ptr = Retained::as_ptr(&nswin);
                            windows
                                .iter()
                                .find(|w| Retained::as_ptr(*w) as *const NSWindow == nswin_ptr)
                                .cloned()
                        })
                        .collect()
                } else {
                    windows.iter().cloned().collect()
                }
            } else {
                Vec::new()
            };
        drop(windows);

        for window in ordered_windows.iter() {
            let mut window_state = WindowUpgradeState::new();

            let frame = window.frame();
            window_state.x = frame.origin.x as i32;
            window_state.y = frame.origin.y as i32;
            window_state.width = frame.size.width as i32;
            window_state.height = frame.size.height as i32;
            window_state.fullscreen = window.styleMask().contains(NSWindowStyleMask::FullScreen);

            if let Some(terminal_view) = window.active_terminal() {
                let mut tab_state = TabUpgradeState::new(0);
                let title = window.title().to_string();
                tab_state.title = title.clone();
                if terminal_view.is_title_locked() {
                    tab_state.custom_title = Some(title);
                }
                tab_state.template_name = terminal_view.template_name();
                tab_state.color = window.tab_color();
                tab_state.session_id = terminal_view.session_id();
                tab_state.cwd = terminal_view
                    .terminal()
                    .lock()
                    .foreground_cwd()
                    .map(|p| p.to_string_lossy().into_owned());

                window_state.tabs.push(tab_state);
            }

            if !window_state.tabs.is_empty() {
                upgrade_state.windows.push(window_state);
            }
        }

        if upgrade_state.windows.is_empty() {
            log::warn!("No terminals to preserve in relaunch");
            return;
        }

        log::info!(
            "Relaunch state collected: {} windows",
            upgrade_state.windows.len(),
        );

        match execute_upgrade(&binary, &upgrade_state) {
            Ok(()) => {
                log::info!("Relaunch successful, terminating old process");
                self.ivars().is_relaunching.set(true);
                let mtm = MainThreadMarker::from(self);
                let app = NSApplication::sharedApplication(mtm);
                app.terminate(None);
            }
            Err(e) => {
                log::error!("Relaunch failed: {}", e);
            }
        }
    }
}

/// Get the theme based on configuration
fn get_theme(config: &Config) -> Theme {
    cterm_app::resolve_theme(config)
}

/// Run the native macOS application
pub fn run() {
    // Parse command-line arguments first
    let args = Args::parse();

    // Initialize logging with capture buffer for in-app log viewing
    crate::log_capture::init();

    // Save the original FD limit before raising it, so child processes can restore it
    #[cfg(unix)]
    cterm_core::save_original_nofile_limit();

    // Raise the file descriptor limit so we can handle many tabs + upgrades.
    // The default macOS soft limit is 256, which is too low for heavy use.
    #[cfg(unix)]
    {
        let mut rlim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) } == 0 {
            let new_cur = rlim.rlim_max.min(10240);
            if new_cur > rlim.rlim_cur {
                rlim.rlim_cur = new_cur;
                unsafe {
                    libc::setrlimit(libc::RLIMIT_NOFILE, &rlim);
                }
            }
        }
    }

    // Install signal handler for crash debugging
    // Uses only async-signal-safe operations (raw write + abort)
    #[cfg(unix)]
    unsafe {
        extern "C" fn crash_handler(sig: libc::c_int) {
            // Only use async-signal-safe functions in signal handlers
            let msg: &[u8] = match sig {
                libc::SIGSEGV => b"\n=== CRASH: SIGSEGV ===\n",
                libc::SIGBUS => b"\n=== CRASH: SIGBUS ===\n",
                _ => b"\n=== CRASH: Unknown signal ===\n",
            };
            unsafe {
                let _ = libc::write(2, msg.as_ptr() as *const _, msg.len());
                libc::abort();
            }
        }
        let handler: extern "C" fn(libc::c_int) = crash_handler;
        libc::signal(libc::SIGSEGV, handler as libc::sighandler_t);
        libc::signal(libc::SIGBUS, handler as libc::sighandler_t);
    }

    log::info!("Starting cterm (native macOS)");

    // Check if we're in upgrade receiver mode
    if let Some(ref state_path) = args.upgrade_state {
        log::info!(
            "Running in upgrade receiver mode with state file: {}",
            state_path
        );
        let exit_code = crate::upgrade_receiver::run_receiver(state_path);
        std::process::exit(exit_code);
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

    // Perform background git sync before loading config
    if cterm_app::background_sync() {
        log::info!("Configuration was updated from git remote");
    }

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

/// Build a MoshConfig from a RemoteConfig.
fn mosh_config_from_remote(remote: &RemoteConfig, cols: u16, rows: u16) -> cterm_mosh::MoshConfig {
    cterm_mosh::MoshConfig {
        host: remote.host.clone(),
        cols,
        rows,
        locale: Some("en_US.UTF-8".to_string()),
        term: Some("xterm-256color".to_string()),
        ssh_args: Vec::new(),
    }
}
