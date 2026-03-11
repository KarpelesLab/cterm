//! Application menu system

use gtk4::{gio, glib};

/// Helper to create a menu item with a shortcut label displayed in the menu.
///
/// Sets the "accel" attribute for display only. Actual key handling is done
/// by our custom EventControllerKey; we do NOT call set_accels_for_action.
fn menu_item(label: &str, action: &str, accel: Option<&str>) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), Some(action));
    if let Some(accel) = accel {
        item.set_attribute_value("accel", Some(&glib::Variant::from(accel)));
    }
    item
}

/// Create the application menu model with options
///
/// If `show_debug` is true, includes a Debug submenu in the Help menu
/// with developer/testing options.
pub fn create_menu_model_with_options(show_debug: bool) -> gio::Menu {
    let menu = gio::Menu::new();

    // File menu
    let file_menu = gio::Menu::new();
    file_menu.append_item(&menu_item("New Tab", "win.new-tab", Some("<Ctrl><Shift>t")));
    file_menu.append_item(&menu_item(
        "New Window",
        "win.new-window",
        Some("<Ctrl><Shift>n"),
    ));
    file_menu.append_item(&menu_item(
        "Quick Open Template...",
        "win.quick-open",
        Some("<Ctrl><Shift>o"),
    ));

    // Docker submenu
    let docker_menu = gio::Menu::new();
    docker_menu.append(Some("Docker Terminal..."), Some("win.docker-picker"));
    file_menu.append_submenu(Some("Docker"), &docker_menu);

    // Session submenu (daemon)
    let session_menu = gio::Menu::new();
    session_menu.append(Some("Attach to Session..."), Some("win.attach-session"));
    session_menu.append(Some("SSH Remote..."), Some("win.ssh-connect"));
    file_menu.append_submenu(Some("Sessions"), &session_menu);

    file_menu.append(Some("Tab Templates..."), Some("win.tab-templates"));
    file_menu.append_item(&menu_item(
        "Close Tab",
        "win.close-tab",
        Some("<Ctrl><Shift>w"),
    ));
    file_menu.append(Some("Close Other Tabs"), Some("win.close-other-tabs"));
    file_menu.append_item(&menu_item("Quit", "win.quit", Some("<Ctrl><Shift>q")));
    menu.append_submenu(Some("File"), &file_menu);

    // Edit menu
    let edit_menu = gio::Menu::new();
    edit_menu.append_item(&menu_item("Copy", "win.copy", Some("<Ctrl><Shift>c")));
    edit_menu.append(Some("Copy as HTML"), Some("win.copy-html"));
    edit_menu.append_item(&menu_item("Paste", "win.paste", Some("<Ctrl><Shift>v")));
    edit_menu.append_item(&menu_item(
        "Select All",
        "win.select-all",
        Some("<Ctrl><Shift>a"),
    ));
    menu.append_submenu(Some("Edit"), &edit_menu);

    // Terminal menu
    let terminal_menu = gio::Menu::new();
    terminal_menu.append(Some("Set Title..."), Some("win.set-title"));
    terminal_menu.append(Some("Set Color..."), Some("win.set-color"));
    terminal_menu.append_item(&menu_item("Find...", "win.find", Some("<Ctrl><Shift>f")));

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

    // Tools menu
    let tools_menu = create_tools_submenu();
    menu.append_submenu(Some("Tools"), &tools_menu);

    // Tabs menu - will be populated dynamically
    let tabs_menu = gio::Menu::new();
    tabs_menu.append(Some("Previous Tab"), Some("win.prev-tab"));
    tabs_menu.append(Some("Next Tab"), Some("win.next-tab"));
    tabs_menu.append_item(&menu_item(
        "Next Alerted Tab",
        "win.next-alerted-tab",
        Some("<Ctrl><Shift>b"),
    ));
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
        debug_menu.append(Some("Re-launch ctermd"), Some("win.debug-relaunch-daemon"));
        debug_menu.append(Some("Dump State"), Some("win.debug-dump-state"));
        help_menu.append_submenu(Some("Debug"), &debug_menu);
    }

    menu.append_submenu(Some("Help"), &help_menu);

    menu
}

/// Create the tools submenu from loaded tool shortcuts
fn create_tools_submenu() -> gio::Menu {
    let menu = gio::Menu::new();
    if let Ok(shortcuts) = cterm_app::config::load_tool_shortcuts() {
        for (i, shortcut) in shortcuts.iter().enumerate() {
            let action = format!("win.run-tool-shortcut::{}", i);
            menu.append(Some(&shortcut.name), Some(&action));
        }
    }
    menu
}

/// Rebuild the Tools menu in the menu bar (called after preferences save).
/// Rebuilds the entire menu model and replaces it on the PopoverMenuBar.
#[allow(dead_code)]
pub fn rebuild_menu_bar(menu_bar: &gtk4::PopoverMenuBar, show_debug: bool) {
    let menu_model = create_menu_model_with_options(show_debug);
    menu_bar.set_menu_model(Some(&menu_model));
}
