//! cterm - A high-performance terminal emulator
//!
//! Main entry point for the GTK4 application.

mod app;
mod window;
mod terminal_widget;
mod tab_bar;

use gtk4::prelude::*;
use gtk4::{Application, glib};

fn main() -> glib::ExitCode {
    // Initialize logging
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .init();

    log::info!("Starting cterm");

    // Create the GTK application
    let app = Application::builder()
        .application_id("com.cterm.terminal")
        .build();

    // Connect to the activate signal
    app.connect_activate(|app| {
        app::build_ui(app);
    });

    // Run the application
    app.run()
}
