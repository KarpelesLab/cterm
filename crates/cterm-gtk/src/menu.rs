//! Application menu system

use gtk4::gio;

/// Create the application menu model
pub fn create_menu_model() -> gio::Menu {
    create_menu_model_with_options(false)
}

/// Create the application menu model with options
///
/// If `show_debug` is true, includes a Debug submenu in the Help menu
/// with developer/testing options.
pub fn create_menu_model_with_options(show_debug: bool) -> gio::Menu {
    let menu = gio::Menu::new();

    // File menu
    let file_menu = gio::Menu::new();
    file_menu.append(Some("New Tab"), Some("win.new-tab"));
    file_menu.append(Some("New Window"), Some("win.new-window"));
    file_menu.append(Some("Quick Open Template..."), Some("win.quick-open"));

    // Docker submenu
    let docker_menu = gio::Menu::new();
    docker_menu.append(Some("Docker Terminal..."), Some("win.docker-picker"));
    file_menu.append_submenu(Some("Docker"), &docker_menu);

    file_menu.append(Some("Tab Templates..."), Some("win.tab-templates"));
    file_menu.append(Some("Close Tab"), Some("win.close-tab"));
    file_menu.append(Some("Close Other Tabs"), Some("win.close-other-tabs"));
    file_menu.append(Some("Quit"), Some("win.quit"));
    menu.append_submenu(Some("File"), &file_menu);

    // Edit menu
    let edit_menu = gio::Menu::new();
    edit_menu.append(Some("Copy"), Some("win.copy"));
    edit_menu.append(Some("Copy as HTML"), Some("win.copy-html"));
    edit_menu.append(Some("Paste"), Some("win.paste"));
    edit_menu.append(Some("Select All"), Some("win.select-all"));
    menu.append_submenu(Some("Edit"), &edit_menu);

    // Terminal menu
    let terminal_menu = gio::Menu::new();
    terminal_menu.append(Some("Set Title..."), Some("win.set-title"));
    terminal_menu.append(Some("Set Color..."), Some("win.set-color"));
    terminal_menu.append(Some("Find..."), Some("win.find"));

    // Encoding submenu
    let encoding_menu = gio::Menu::new();
    encoding_menu.append(Some("UTF-8"), Some("win.set-encoding::utf8"));
    encoding_menu.append(Some("ISO-8859-1"), Some("win.set-encoding::iso8859-1"));
    encoding_menu.append(Some("ISO-8859-15"), Some("win.set-encoding::iso8859-15"));
    terminal_menu.append_submenu(Some("Set Encoding"), &encoding_menu);

    // Signal submenu
    let signal_menu = gio::Menu::new();
    signal_menu.append(Some("SIGHUP (1)"), Some("win.send-signal::1"));
    signal_menu.append(Some("SIGINT (2)"), Some("win.send-signal::2"));
    signal_menu.append(Some("SIGQUIT (3)"), Some("win.send-signal::3"));
    signal_menu.append(Some("SIGTERM (15)"), Some("win.send-signal::15"));
    signal_menu.append(Some("SIGKILL (9)"), Some("win.send-signal::9"));
    signal_menu.append(Some("SIGUSR1 (10)"), Some("win.send-signal::10"));
    signal_menu.append(Some("SIGUSR2 (12)"), Some("win.send-signal::12"));
    terminal_menu.append_submenu(Some("Send Signal"), &signal_menu);

    terminal_menu.append(Some("Reset"), Some("win.reset"));
    terminal_menu.append(Some("Clear Scrollback && Reset"), Some("win.clear-reset"));
    menu.append_submenu(Some("Terminal"), &terminal_menu);

    // Tabs menu - will be populated dynamically
    let tabs_menu = gio::Menu::new();
    tabs_menu.append(Some("Previous Tab"), Some("win.prev-tab"));
    tabs_menu.append(Some("Next Tab"), Some("win.next-tab"));
    tabs_menu.append(Some("Next Alerted Tab"), Some("win.next-alerted-tab"));
    // Tab list section will be added dynamically
    menu.append_submenu(Some("Tabs"), &tabs_menu);

    // Help menu
    let help_menu = gio::Menu::new();
    help_menu.append(Some("Preferences..."), Some("win.preferences"));
    help_menu.append(Some("Check for Updates..."), Some("win.check-updates"));
    help_menu.append(Some("About"), Some("win.about"));

    // Debug submenu (hidden unless Shift is held when opening menu)
    if show_debug {
        let debug_menu = gio::Menu::new();
        debug_menu.append(Some("View Logs"), Some("win.view-logs"));
        debug_menu.append(Some("Re-launch cterm"), Some("win.debug-relaunch"));
        debug_menu.append(Some("Dump State"), Some("win.debug-dump-state"));
        help_menu.append_submenu(Some("Debug"), &debug_menu);
    }

    menu.append_submenu(Some("Help"), &help_menu);

    menu
}

/// Create the tabs submenu (called when tabs change)
#[allow(dead_code)]
pub fn create_tabs_submenu(tab_names: &[(u64, String)]) -> gio::Menu {
    let menu = gio::Menu::new();

    menu.append(Some("Previous Tab"), Some("win.prev-tab"));
    menu.append(Some("Next Tab"), Some("win.next-tab"));
    menu.append(Some("Next Alerted Tab"), Some("win.next-alerted-tab"));

    if !tab_names.is_empty() {
        let tabs_section = gio::Menu::new();
        for (idx, (id, name)) in tab_names.iter().enumerate() {
            let action = format!("win.switch-tab::{}", id);
            let label = format!("{}. {}", idx + 1, name);
            tabs_section.append(Some(&label), Some(&action));
        }
        menu.append_section(None, &tabs_section);
    }

    menu
}
