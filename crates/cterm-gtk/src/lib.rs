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
mod remotes_dialog;
mod session_dialog;
mod tab_bar;
mod tab_templates_dialog;
mod terminal_widget;
mod update_dialog;
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

    /// Path to upgrade state file (internal use)
    #[arg(long, hide = true)]
    pub upgrade_state: Option<String>,
}

/// Global application arguments (accessible from window creation)
static APP_ARGS: std::sync::OnceLock<Args> = std::sync::OnceLock::new();

/// Executable path captured at startup (before the binary can be replaced on disk)
static EXE_PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Get the application arguments (call only after parse_args())
pub fn get_args() -> &'static Args {
    APP_ARGS.get().expect("Args not initialized")
}

/// Get the executable path captured at startup
pub fn get_exe_path() -> &'static std::path::Path {
    EXE_PATH.get().expect("Exe path not initialized")
}

/// Run the GTK4 application
pub fn run() {
    // Capture executable path early, before the binary can be replaced on disk.
    // On Linux, /proc/self/exe appends " (deleted)" after the binary is overwritten,
    // so we must resolve it now while it's still valid.
    if let Ok(exe) = std::env::current_exe() {
        let _ = EXE_PATH.set(exe);
    }

    // Parse command-line arguments first (before GTK consumes them)
    let args = Args::parse();

    // Initialize logging with capture for in-app viewing
    cterm_app::log_capture::init();

    // Save the original FD limit before raising it, so child processes can restore it
    #[cfg(unix)]
    cterm_core::save_original_nofile_limit();

    // Raise the file descriptor limit so we can handle many tabs + upgrades
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

    log::info!("Starting cterm");

    // Check if we're in upgrade receiver mode
    if let Some(ref state_path) = args.upgrade_state {
        log::info!(
            "Running in upgrade receiver mode with state file {}",
            state_path
        );
        let exit_code = upgrade_receiver::run_receiver(state_path);
        std::process::exit(exit_code.value());
    }

    // Store args for later access
    let _ = APP_ARGS.set(args);

    // Initialize Adwaita theme engine for proper widget styling
    #[cfg(feature = "adwaita")]
    let _ = libadwaita::init();

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
    // the command line (which contains flags that GTK doesn't know)
    let exit_code = app.run_with_args(&[] as &[&str]);
    std::process::exit(exit_code.value());
}
