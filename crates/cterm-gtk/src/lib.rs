//! cterm-gtk: GTK4 UI for cterm
//!
//! This crate implements the cterm terminal emulator UI using GTK4.

mod app;
mod dialogs;
mod docker_dialog;
mod file_transfer;
mod log_viewer;
mod menu;
mod notification_bar;
mod quick_open;
mod tab_bar;
mod tab_templates_dialog;
mod terminal_widget;
mod update_dialog;
#[cfg(unix)]
mod upgrade_receiver;
mod window;

use clap::Parser;
use gtk4::prelude::*;
use gtk4::Application;
use std::path::PathBuf;

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

    /// Disable watchdog supervision (run directly without crash recovery)
    #[arg(long)]
    pub no_watchdog: bool,
}

/// Global application arguments (accessible from window creation)
static APP_ARGS: std::sync::OnceLock<Args> = std::sync::OnceLock::new();

/// Get the application arguments (call only after parse_args())
pub fn get_args() -> &'static Args {
    APP_ARGS.get().expect("Args not initialized")
}

/// Run the GTK4 application
pub fn run() {
    // Parse command-line arguments first (before GTK consumes them)
    let args = Args::parse();

    // Initialize logging with capture for in-app viewing
    cterm_app::log_capture::init();

    log::info!("Starting cterm");

    // Check if we're in upgrade receiver mode
    #[cfg(unix)]
    if let Some(fd) = args.upgrade_receiver {
        log::info!("Running in upgrade receiver mode with FD {}", fd);
        let exit_code = upgrade_receiver::run_receiver(fd);
        std::process::exit(exit_code.value());
    }

    // Store args for later access
    let _ = APP_ARGS.set(args);

    // Create the GTK application
    let app = Application::builder()
        .application_id("com.cterm.terminal")
        .build();

    // Connect to the activate signal
    app.connect_activate(|app| {
        app::build_ui(app);
    });

    // Run the application
    // Use run_with_args with empty args to prevent GTK from parsing
    // the command line (which contains --no-watchdog that GTK doesn't know)
    let exit_code = app.run_with_args(&[] as &[&str]);
    std::process::exit(exit_code.value());
}
