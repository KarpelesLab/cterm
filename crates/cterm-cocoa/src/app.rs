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

/// Take recovery FDs (consumes them)
#[cfg(unix)]
pub fn take_recovery_fds() -> Vec<cterm_app::RecoveredFd> {
    RECOVERY_FDS.with(|r| std::mem::take(&mut *r.borrow_mut()))
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
                        log::info!("User wants to report crash - TODO: implement crash reporting");
                        // TODO: Implement actual crash reporting (e.g., open GitHub issue URL)
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
                            log::info!(
                                "Restored display state for terminal (watchdog_fd_id={})",
                                recovered.id
                            );
                        }
                        // Restore window title
                        window.setTitle(&NSString::from_str(&tab_state.title));
                    }

                    self.ivars().windows.borrow_mut().push(window.clone());

                    if i == 0 {
                        // Make first window key
                        window.makeKeyAndOrderFront(None);
                    } else {
                        // Add additional windows as tabs
                        if let Some(first_window) = self.ivars().windows.borrow().first() {
                            first_window.addTabbedWindow_ordered(
                                &window,
                                objc2_app_kit::NSWindowOrderingMode::Above,
                            );
                        }
                        window.orderFront(None);
                    }
                }

                // Clear crash state file after successful recovery
                let _ = cterm_app::clear_crash_state();

                // Start periodic state saving
                self.start_state_save_timer(mtm);

                return;
            }

            // Normal startup - create the main window
            let window = CtermWindow::new(mtm, &self.ivars().config, &self.ivars().theme);

            // Store window reference
            self.ivars().windows.borrow_mut().push(window.clone());

            // Show the window
            window.makeKeyAndOrderFront(None);

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
    }
);

impl AppDelegate {
    pub fn new(mtm: MainThreadMarker, config: Config, theme: Theme) -> Retained<Self> {
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(AppDelegateIvars {
            config,
            theme,
            windows: std::cell::RefCell::new(Vec::new()),
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
        window.makeKeyAndOrderFront(None);
        log::info!("Created new tab from template: {}", template.name);
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

        let crash_state = CrashState::new(upgrade_state);

        if let Err(e) = write_crash_state(&crash_state) {
            log::error!("Failed to save crash state: {}", e);
        } else {
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
            let timer = NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                interval,
                self,
                sel!(saveStateTimer:),
                None,
                true,
            );

            // Keep timer alive by adding to run loop (it's already added by scheduledTimer...)
            let _ = timer;
        }

        log::info!("Started crash state save timer (interval: {}s)", interval);
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

    // Initialize logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

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
        libc::signal(libc::SIGSEGV, crash_handler as libc::sighandler_t);
        libc::signal(libc::SIGBUS, crash_handler as libc::sighandler_t);
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

    // Activate the app
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);

    log::info!("Starting main run loop");

    // Run the main event loop
    app.run();
}
