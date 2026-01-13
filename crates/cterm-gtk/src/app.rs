//! Application setup and management

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, CssProvider, Orientation,
    gdk, gio, glib,
};
use parking_lot::Mutex;

use cterm_app::config::{load_config, Config};
use cterm_app::session::{Session, TabState, WindowState};
use cterm_app::shortcuts::ShortcutManager;
use cterm_ui::theme::Theme;

use crate::window::CtermWindow;

/// Build the main UI
pub fn build_ui(app: &Application) {
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

/// Apply CSS styling to the application
fn apply_css(theme: &Theme) {
    let provider = CssProvider::new();

    let css = format!(
        r#"
        /* Global styles */
        window {{
            background-color: {};
        }}

        /* Terminal area */
        .terminal {{
            background-color: {};
            padding: 4px;
        }}

        /* Tab bar */
        .tab-bar {{
            background-color: {};
            border-bottom: 1px solid {};
            padding: 2px 4px;
        }}

        .tab-bar button {{
            background: {};
            color: {};
            border: none;
            border-radius: 4px;
            padding: 4px 12px;
            margin: 2px;
            min-height: 24px;
        }}

        .tab-bar button:hover {{
            background: alpha({}, 0.1);
        }}

        .tab-bar button.active {{
            background: {};
            color: {};
        }}

        .tab-bar button.has-unread {{
            font-weight: bold;
        }}

        .tab-close-button {{
            padding: 2px;
            min-width: 16px;
            min-height: 16px;
            border-radius: 50%;
        }}

        .tab-close-button:hover {{
            background: alpha(red, 0.2);
        }}

        /* Scrollbar */
        scrollbar {{
            background: transparent;
        }}

        scrollbar slider {{
            background: {};
            border-radius: 4px;
            min-width: 8px;
            min-height: 8px;
        }}

        scrollbar slider:hover {{
            background: {};
        }}

        /* New tab button */
        .new-tab-button {{
            padding: 4px 8px;
            border-radius: 4px;
        }}

        .new-tab-button:hover {{
            background: alpha(white, 0.1);
        }}
        "#,
        rgb_to_css(&theme.colors.background),
        rgb_to_css(&theme.colors.background),
        rgb_to_css(&theme.ui.tab_bar_background),
        rgb_to_css(&theme.ui.border),
        rgb_to_css(&theme.ui.tab_inactive_background),
        rgb_to_css(&theme.ui.tab_inactive_text),
        rgb_to_css(&theme.ui.tab_active_text),
        rgb_to_css(&theme.ui.tab_active_background),
        rgb_to_css(&theme.ui.tab_active_text),
        rgb_to_css(&theme.ui.scrollbar),
        rgb_to_css(&theme.ui.scrollbar_hover),
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

/// Convert RGB to CSS color string
fn rgb_to_css(rgb: &cterm_core::color::Rgb) -> String {
    format!("rgb({}, {}, {})", rgb.r, rgb.g, rgb.b)
}
