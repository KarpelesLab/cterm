//! Application setup and management for macOS
//!
//! Handles NSApplication lifecycle and main event loop.

use clap::Parser;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate};
use objc2_foundation::{MainThreadMarker, NSNotification, NSObject, NSObjectProtocol};
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
}

/// Global application arguments (accessible from window creation)
static APP_ARGS: std::sync::OnceLock<Args> = std::sync::OnceLock::new();

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

            // Create the main window
            let mtm = MainThreadMarker::from(self);
            let window = CtermWindow::new(mtm, &self.ivars().config, &self.ivars().theme);

            // Store window reference
            self.ivars().windows.borrow_mut().push(window.clone());

            // Show the window
            window.makeKeyAndOrderFront(None);
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _sender: &NSApplication) -> bool {
            true
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

    log::info!("Starting cterm (native macOS)");

    // Check if we're in upgrade receiver mode
    #[cfg(unix)]
    if let Some(fd) = args.upgrade_receiver {
        log::info!("Running in upgrade receiver mode with FD {}", fd);
        let exit_code = crate::upgrade_receiver::run_receiver(fd);
        std::process::exit(exit_code);
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
