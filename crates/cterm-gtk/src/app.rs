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

    // Try to reconnect to existing daemon sessions before creating a new one
    let reconnected = {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();

        rt.ok().and_then(|rt| {
            let check = rt.block_on(cterm_app::daemon_reconnect::check_daemon_sessions());
            if let cterm_app::daemon_reconnect::ReconnectCheck::Available(sessions) = check {
                let running_count = sessions.iter().filter(|s| s.running).count();
                if running_count > 0 {
                    log::info!(
                        "Found {} running daemon sessions, reconnecting...",
                        running_count
                    );
                    rt.block_on(cterm_app::daemon_reconnect::reconnect_all_sessions())
                        .ok()
                        .filter(|r| !r.is_empty())
                } else {
                    None
                }
            } else {
                None
            }
        })
    };

    if let Some(reconnected) = reconnected {
        // Create window without initial tab, then add reconnected sessions as tabs
        let window = CtermWindow::new_empty(app, &config, &theme);
        for recon in reconnected {
            window.add_reconnected_tab(recon, None);
        }
        log::info!("Reconnected to daemon sessions, skipping normal startup");
        window.present();
    } else {
        // Normal startup - create the main window with a fresh session
        let window = CtermWindow::new(app, &config, &theme);
        window.present();
    }
}

/// Get the theme based on configuration
fn get_theme(config: &Config) -> Theme {
    cterm_app::resolve_theme(config)
}

/// Format an Rgb color as a CSS rgb() value
fn rgb_css(c: &cterm_core::color::Rgb) -> String {
    format!("rgb({},{},{})", c.r, c.g, c.b)
}

/// Apply CSS styling to the application
/// Uses theme colors for terminal-specific elements, leaving system defaults for dialogs, menus, etc.
fn apply_css(theme: &Theme) {
    let provider = CssProvider::new();
    let ui = &theme.ui;

    let tab_bar_bg = rgb_css(&ui.tab_bar_background);
    let tab_active_bg = rgb_css(&ui.tab_active_background);
    let tab_active_text = rgb_css(&ui.tab_active_text);
    let tab_inactive_text = rgb_css(&ui.tab_inactive_text);
    let border = rgb_css(&ui.border);

    let css = format!(
        r#"
        /* Terminal drawing area - background handled by Cairo drawing */
        .terminal {{
            padding: 0;
        }}

        /* Tab bar styling */
        .tab-bar {{
            background-color: {tab_bar_bg};
            padding: 4px 4px 0 4px;
            border-bottom: 1px solid {border};
        }}

        .tab-bar button {{
            border: none;
            border-radius: 6px 6px 2px 2px;
            padding: 4px 12px;
            margin: 0 1px;
            min-height: 0;
            background-color: transparent;
            color: {tab_inactive_text};
            transition: background-color 150ms ease-in-out,
                        color 150ms ease-in-out;
        }}

        .tab-bar button:hover {{
            background-color: alpha({tab_active_bg}, 0.5);
            color: {tab_active_text};
        }}

        .tab-bar button.active {{
            background-color: {tab_active_bg};
            color: {tab_active_text};
        }}

        .tab-bar button.active:hover {{
            background-color: {tab_active_bg};
        }}

        .tab-bar button.has-unread {{
            font-weight: bold;
            color: {tab_active_text};
        }}

        .tab-close-button {{
            padding: 0 2px;
            min-width: 16px;
            min-height: 16px;
            border-radius: 4px;
            opacity: 0;
            transition: opacity 150ms ease-in-out,
                        background-color 150ms ease-in-out;
        }}

        .tab-bar button:hover .tab-close-button {{
            opacity: 0.7;
        }}

        .tab-bar button.active .tab-close-button {{
            opacity: 0.7;
        }}

        .tab-close-button:hover {{
            background-color: alpha(rgb(255,80,80), 0.3);
            opacity: 1.0;
        }}

        /* New tab button */
        .new-tab-button {{
            padding: 4px 8px;
            border-radius: 4px;
            min-height: 0;
            margin: 0 2px 4px 2px;
            color: {tab_inactive_text};
            background-color: transparent;
            transition: background-color 150ms ease-in-out,
                        color 150ms ease-in-out;
        }}

        .new-tab-button:hover {{
            color: {tab_active_text};
            background-color: alpha({tab_active_bg}, 0.5);
        }}

        /* Notification bar */
        .notification-bar {{
            background-color: {tab_bar_bg};
            border-bottom: 1px solid {border};
            padding: 4px 8px;
        }}

        .notification-bar label {{
            color: {tab_active_text};
        }}

        .notification-bar button {{
            min-height: 0;
            padding: 2px 12px;
            border-radius: 4px;
        }}

        /* Remove popover shadow - alpha compositing is unreliable on X11 */
        popover {{
            margin: 0;
        }}
        popover > contents {{
            box-shadow: none;
        }}
        "#
    );

    provider.load_from_data(&css);

    // Apply to the default display
    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
