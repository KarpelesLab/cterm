//! Application setup and management

use gtk4::{gdk, Application, CssProvider};

use cterm_app::config::{load_config, Config};
use cterm_ui::theme::Theme;

use crate::window::CtermWindow;

/// Build the main UI
pub fn build_ui(app: &Application) {
    // Perform background git sync before loading config
    if cterm_app::background_sync() {
        log::info!("Configuration was updated from git remote");
    }

    // Load configuration
    let config = load_config().unwrap_or_else(|e| {
        log::warn!("Failed to load config, using defaults: {}", e);
        Config::default()
    });

    // Load theme
    let theme = get_theme(&config);

    // Apply CSS styling
    apply_css(&theme);

    // Create the main window
    let window = CtermWindow::new(app, &config, &theme);
    window.present();
}

/// Get the theme based on configuration
fn get_theme(config: &Config) -> Theme {
    cterm_app::resolve_theme(config)
}

/// Apply CSS styling to the application
/// Only styles terminal-specific elements, leaving system defaults for dialogs, menus, etc.
fn apply_css(_theme: &Theme) {
    let provider = CssProvider::new();

    // Only style terminal-specific elements
    // Menu bar, dialogs, and preferences use system defaults
    let css = r#"
        /* Terminal drawing area - background handled by Cairo drawing */
        .terminal {
            padding: 0;
        }

        /* Tab bar styling - compact height */
        .tab-bar {
            padding: 1px 2px;
        }

        .tab-bar button {
            border: none;
            border-radius: 3px;
            padding: 2px 8px;
            margin: 1px;
            min-height: 0;
        }

        .tab-bar button.has-unread {
            font-weight: bold;
        }

        .tab-close-button {
            padding: 0px 2px;
            min-width: 14px;
            min-height: 14px;
            border-radius: 50%;
        }

        .tab-close-button:hover {
            background: alpha(red, 0.2);
        }

        /* New tab button */
        .new-tab-button {
            padding: 2px 6px;
            border-radius: 3px;
            min-height: 0;
        }
        "#;

    provider.load_from_data(css);

    // Apply to the default display
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
