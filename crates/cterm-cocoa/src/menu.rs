//! Menu bar implementation for macOS
//!
//! Creates the standard macOS menu bar with File, Edit, View, etc.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::sel;
use objc2_app_kit::{NSEventModifierFlags, NSMenu, NSMenuItem};
use objc2_foundation::{MainThreadMarker, NSString};

// Thread-local storage for the debug menu item (must be accessed on main thread)
thread_local! {
    static DEBUG_MENU_ITEM: RefCell<Option<Retained<NSMenuItem>>> = const { RefCell::new(None) };
}

/// Store the debug menu item for later show/hide
fn set_debug_menu_item(item: Retained<NSMenuItem>) {
    DEBUG_MENU_ITEM.with(|cell| {
        *cell.borrow_mut() = Some(item);
    });
}

/// Show or hide the debug menu
pub fn set_debug_menu_visible(visible: bool) {
    DEBUG_MENU_ITEM.with(|cell| {
        if let Some(ref item) = *cell.borrow() {
            item.setHidden(!visible);
        }
    });
}

/// Check if the debug menu is currently visible
pub fn is_debug_menu_visible() -> bool {
    DEBUG_MENU_ITEM.with(|cell| {
        if let Some(ref item) = *cell.borrow() {
            !item.isHidden()
        } else {
            false
        }
    })
}

/// Create the main menu bar
pub fn create_menu_bar(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let menu_bar = NSMenu::new(mtm);

    // Application menu (cterm)
    menu_bar.addItem(&create_app_menu(mtm));

    // File menu
    menu_bar.addItem(&create_file_menu(mtm));

    // Edit menu
    menu_bar.addItem(&create_edit_menu(mtm));

    // View menu
    menu_bar.addItem(&create_view_menu(mtm));

    // Terminal menu
    menu_bar.addItem(&create_terminal_menu(mtm));

    // Tools menu
    menu_bar.addItem(&create_tools_menu(mtm));

    // Window menu
    menu_bar.addItem(&create_window_menu(mtm));

    // Help menu
    menu_bar.addItem(&create_help_menu(mtm));

    menu_bar
}

fn create_app_menu(mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    let menu = NSMenu::new(mtm);
    menu.setTitle(&NSString::from_str("cterm"));

    // About cterm
    menu.addItem(&create_menu_item(
        mtm,
        "About cterm",
        Some(sel!(orderFrontStandardAboutPanel:)),
        "",
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Preferences
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Preferences...",
        Some(sel!(showPreferences:)),
        ",",
        NSEventModifierFlags::Command,
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Services submenu (standard macOS)
    let services_item = NSMenuItem::new(mtm);
    services_item.setTitle(&NSString::from_str("Services"));
    let services_menu = NSMenu::new(mtm);
    services_item.setSubmenu(Some(&services_menu));
    menu.addItem(&services_item);

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Hide/Show
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Hide cterm",
        Some(sel!(hide:)),
        "h",
        NSEventModifierFlags::Command,
    ));

    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Hide Others",
        Some(sel!(hideOtherApplications:)),
        "h",
        NSEventModifierFlags::Command.union(NSEventModifierFlags::Option),
    ));

    menu.addItem(&create_menu_item(
        mtm,
        "Show All",
        Some(sel!(unhideAllApplications:)),
        "",
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Quit
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Quit cterm",
        Some(sel!(terminate:)),
        "q",
        NSEventModifierFlags::Command,
    ));

    let menu_item = NSMenuItem::new(mtm);
    menu_item.setSubmenu(Some(&menu));
    menu_item
}

fn create_file_menu(mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    let menu = NSMenu::new(mtm);
    menu.setTitle(&NSString::from_str("File"));

    // New Tab
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "New Tab",
        Some(sel!(newTab:)),
        "t",
        NSEventModifierFlags::Command,
    ));

    // New Window
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "New Window",
        Some(sel!(newWindow:)),
        "n",
        NSEventModifierFlags::Command,
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Quick Open Template
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Quick Open Template...",
        Some(sel!(showQuickOpen:)),
        "g",
        NSEventModifierFlags::Command,
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Tab Templates submenu
    let templates_submenu = NSMenu::new(mtm);
    templates_submenu.setTitle(&NSString::from_str("Tab Templates"));

    // Load templates to populate submenu
    if let Ok(templates) = cterm_app::config::load_sticky_tabs() {
        for (i, template) in templates.iter().enumerate() {
            let item = NSMenuItem::new(mtm);
            item.setTitle(&NSString::from_str(&template.name));
            unsafe { item.setAction(Some(sel!(openTabTemplate:))) };
            item.setTag(i as isize);

            // Add keyboard shortcut for first 9 templates (Cmd+1 through Cmd+9)
            if i < 9 {
                item.setKeyEquivalent(&NSString::from_str(&format!("{}", i + 1)));
                item.setKeyEquivalentModifierMask(
                    NSEventModifierFlags::Command.union(NSEventModifierFlags::Option),
                );
            }

            templates_submenu.addItem(&item);
        }

        if !templates.is_empty() {
            templates_submenu.addItem(&NSMenuItem::separatorItem(mtm));
        }
    }

    // Manage Templates...
    templates_submenu.addItem(&create_menu_item(
        mtm,
        "Manage Templates...",
        Some(sel!(showTabTemplates:)),
        "",
    ));

    let templates_item = NSMenuItem::new(mtm);
    templates_item.setTitle(&NSString::from_str("Tab Templates"));
    templates_item.setSubmenu(Some(&templates_submenu));
    menu.addItem(&templates_item);

    // Open in Container (Docker devcontainer)
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Open in Container",
        Some(sel!(openInContainer:)),
        "d",
        NSEventModifierFlags::Command.union(NSEventModifierFlags::Shift),
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Close Tab
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Close Tab",
        Some(sel!(closeTab:)),
        "w",
        NSEventModifierFlags::Command,
    ));

    // Close Other Tabs
    menu.addItem(&create_menu_item(
        mtm,
        "Close Other Tabs",
        Some(sel!(closeOtherTabs:)),
        "",
    ));

    // Close Window
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Close Window",
        Some(sel!(performClose:)),
        "w",
        NSEventModifierFlags::Command.union(NSEventModifierFlags::Shift),
    ));

    let menu_item = NSMenuItem::new(mtm);
    menu_item.setSubmenu(Some(&menu));
    menu_item
}

fn create_edit_menu(mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    let menu = NSMenu::new(mtm);
    menu.setTitle(&NSString::from_str("Edit"));

    // Undo/Redo (standard but usually disabled in terminal)
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Undo",
        Some(sel!(undo:)),
        "z",
        NSEventModifierFlags::Command,
    ));

    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Redo",
        Some(sel!(redo:)),
        "z",
        NSEventModifierFlags::Command.union(NSEventModifierFlags::Shift),
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Cut/Copy/Paste
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Cut",
        Some(sel!(cut:)),
        "x",
        NSEventModifierFlags::Command,
    ));

    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Copy",
        Some(sel!(copy:)),
        "c",
        NSEventModifierFlags::Command,
    ));

    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Copy as HTML",
        Some(sel!(copyAsHTML:)),
        "c",
        NSEventModifierFlags::Command.union(NSEventModifierFlags::Shift),
    ));

    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Paste",
        Some(sel!(paste:)),
        "v",
        NSEventModifierFlags::Command,
    ));

    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Select All",
        Some(sel!(selectAll:)),
        "a",
        NSEventModifierFlags::Command,
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Find
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Find...",
        Some(sel!(performFindPanelAction:)),
        "f",
        NSEventModifierFlags::Command,
    ));

    let menu_item = NSMenuItem::new(mtm);
    menu_item.setSubmenu(Some(&menu));
    menu_item
}

fn create_view_menu(mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    let menu = NSMenu::new(mtm);
    menu.setTitle(&NSString::from_str("View"));

    // Zoom
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Zoom In",
        Some(sel!(zoomIn:)),
        "+",
        NSEventModifierFlags::Command,
    ));

    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Zoom Out",
        Some(sel!(zoomOut:)),
        "-",
        NSEventModifierFlags::Command,
    ));

    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Reset Zoom",
        Some(sel!(zoomReset:)),
        "0",
        NSEventModifierFlags::Command,
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Fullscreen
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Toggle Full Screen",
        Some(sel!(toggleFullScreen:)),
        "f",
        NSEventModifierFlags::Command.union(NSEventModifierFlags::Control),
    ));

    let menu_item = NSMenuItem::new(mtm);
    menu_item.setSubmenu(Some(&menu));
    menu_item
}

fn create_terminal_menu(mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    let menu = NSMenu::new(mtm);
    menu.setTitle(&NSString::from_str("Terminal"));

    // Reset
    menu.addItem(&create_menu_item(
        mtm,
        "Reset",
        Some(sel!(resetTerminal:)),
        "",
    ));

    menu.addItem(&create_menu_item(
        mtm,
        "Clear and Reset",
        Some(sel!(clearAndResetTerminal:)),
        "",
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Set Title
    menu.addItem(&create_menu_item(
        mtm,
        "Set Title...",
        Some(sel!(setTerminalTitle:)),
        "",
    ));

    // Set Tab Color
    menu.addItem(&create_menu_item(
        mtm,
        "Set Tab Color...",
        Some(sel!(setTabColor:)),
        "",
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Send Signal submenu
    let signal_menu = NSMenu::new(mtm);
    signal_menu.setTitle(&NSString::from_str("Send Signal"));

    signal_menu.addItem(&create_menu_item(
        mtm,
        "SIGHUP (1) - Hangup",
        Some(sel!(sendSignalHup:)),
        "",
    ));
    signal_menu.addItem(&create_menu_item(
        mtm,
        "SIGINT (2) - Interrupt",
        Some(sel!(sendSignalInt:)),
        "",
    ));
    signal_menu.addItem(&create_menu_item(
        mtm,
        "SIGQUIT (3) - Quit",
        Some(sel!(sendSignalQuit:)),
        "",
    ));
    signal_menu.addItem(&create_menu_item(
        mtm,
        "SIGTERM (15) - Terminate",
        Some(sel!(sendSignalTerm:)),
        "",
    ));
    signal_menu.addItem(&create_menu_item(
        mtm,
        "SIGKILL (9) - Kill",
        Some(sel!(sendSignalKill:)),
        "",
    ));
    signal_menu.addItem(&NSMenuItem::separatorItem(mtm));
    signal_menu.addItem(&create_menu_item(
        mtm,
        "SIGUSR1 (10)",
        Some(sel!(sendSignalUsr1:)),
        "",
    ));
    signal_menu.addItem(&create_menu_item(
        mtm,
        "SIGUSR2 (12)",
        Some(sel!(sendSignalUsr2:)),
        "",
    ));

    let signal_item = NSMenuItem::new(mtm);
    signal_item.setTitle(&NSString::from_str("Send Signal"));
    signal_item.setSubmenu(Some(&signal_menu));
    menu.addItem(&signal_item);

    let menu_item = NSMenuItem::new(mtm);
    menu_item.setSubmenu(Some(&menu));
    menu_item
}

fn create_tools_menu(mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    let menu = NSMenu::new(mtm);
    menu.setTitle(&NSString::from_str("Tools"));

    populate_tools_menu(&menu, mtm);

    let menu_item = NSMenuItem::new(mtm);
    menu_item.setSubmenu(Some(&menu));
    menu_item
}

/// Populate (or repopulate) the Tools menu with shortcut entries
fn populate_tools_menu(menu: &NSMenu, mtm: MainThreadMarker) {
    if let Ok(shortcuts) = cterm_app::config::load_tool_shortcuts() {
        for (i, shortcut) in shortcuts.iter().enumerate() {
            let item = NSMenuItem::new(mtm);
            item.setTitle(&NSString::from_str(&shortcut.name));
            unsafe { item.setAction(Some(sel!(runToolShortcut:))) };
            item.setTag(i as isize);
            menu.addItem(&item);
        }
    }
}

/// Rebuild the Tools menu items (called after preferences save)
pub fn rebuild_tools_menu(mtm: MainThreadMarker) {
    use objc2_app_kit::NSApplication;

    let app = NSApplication::sharedApplication(mtm);
    if let Some(main_menu) = app.mainMenu() {
        // Find the "Tools" menu
        let tools_title = NSString::from_str("Tools");
        let count = main_menu.numberOfItems();
        for i in 0..count {
            if let Some(item) = main_menu.itemAtIndex(i) {
                if let Some(submenu) = item.submenu() {
                    if submenu.title().to_string() == tools_title.to_string() {
                        submenu.removeAllItems();
                        populate_tools_menu(&submenu, mtm);
                        return;
                    }
                }
            }
        }
    }
}

fn create_window_menu(mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    let menu = NSMenu::new(mtm);
    menu.setTitle(&NSString::from_str("Window"));

    // Minimize
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Minimize",
        Some(sel!(performMiniaturize:)),
        "m",
        NSEventModifierFlags::Command,
    ));

    // Zoom (maximize)
    menu.addItem(&create_menu_item(mtm, "Zoom", Some(sel!(performZoom:)), ""));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Window positioning
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Fill",
        Some(sel!(windowFill:)),
        "f",
        NSEventModifierFlags::Control.union(NSEventModifierFlags::Command),
    ));

    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Center",
        Some(sel!(windowCenter:)),
        "c",
        NSEventModifierFlags::Control.union(NSEventModifierFlags::Command),
    ));

    // Move & Resize submenu
    let move_resize_menu = NSMenu::new(mtm);
    move_resize_menu.setTitle(&NSString::from_str("Move & Resize"));

    // Halves
    move_resize_menu.addItem(&create_menu_item_with_key(
        mtm,
        "Left Half",
        Some(sel!(windowLeftHalf:)),
        "\u{f702}", // Left arrow
        NSEventModifierFlags::Control.union(NSEventModifierFlags::Option),
    ));
    move_resize_menu.addItem(&create_menu_item_with_key(
        mtm,
        "Right Half",
        Some(sel!(windowRightHalf:)),
        "\u{f703}", // Right arrow
        NSEventModifierFlags::Control.union(NSEventModifierFlags::Option),
    ));
    move_resize_menu.addItem(&create_menu_item_with_key(
        mtm,
        "Top Half",
        Some(sel!(windowTopHalf:)),
        "\u{f700}", // Up arrow
        NSEventModifierFlags::Control.union(NSEventModifierFlags::Option),
    ));
    move_resize_menu.addItem(&create_menu_item_with_key(
        mtm,
        "Bottom Half",
        Some(sel!(windowBottomHalf:)),
        "\u{f701}", // Down arrow
        NSEventModifierFlags::Control.union(NSEventModifierFlags::Option),
    ));

    move_resize_menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Quarters
    move_resize_menu.addItem(&create_menu_item(
        mtm,
        "Top Left Quarter",
        Some(sel!(windowTopLeftQuarter:)),
        "",
    ));
    move_resize_menu.addItem(&create_menu_item(
        mtm,
        "Top Right Quarter",
        Some(sel!(windowTopRightQuarter:)),
        "",
    ));
    move_resize_menu.addItem(&create_menu_item(
        mtm,
        "Bottom Left Quarter",
        Some(sel!(windowBottomLeftQuarter:)),
        "",
    ));
    move_resize_menu.addItem(&create_menu_item(
        mtm,
        "Bottom Right Quarter",
        Some(sel!(windowBottomRightQuarter:)),
        "",
    ));

    let move_resize_item = NSMenuItem::new(mtm);
    move_resize_item.setTitle(&NSString::from_str("Move & Resize"));
    move_resize_item.setSubmenu(Some(&move_resize_menu));
    menu.addItem(&move_resize_item);

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Tab navigation (Ctrl+Tab and Ctrl+Shift+Tab)
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Show Previous Tab",
        Some(sel!(selectPreviousTab:)),
        "\t", // Tab character
        NSEventModifierFlags::Control.union(NSEventModifierFlags::Shift),
    ));

    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Show Next Tab",
        Some(sel!(selectNextTab:)),
        "\t", // Tab character
        NSEventModifierFlags::Control,
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Next Alerted Tab",
        Some(sel!(selectNextAlertedTab:)),
        "b",
        NSEventModifierFlags::Command.union(NSEventModifierFlags::Shift),
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Select Tab by number (Cmd+1 through Cmd+9)
    for i in 1..=9 {
        let title = if i == 9 {
            "Select Last Tab".to_string()
        } else {
            format!("Select Tab {}", i)
        };
        let item = NSMenuItem::new(mtm);
        item.setTitle(&NSString::from_str(&title));
        unsafe { item.setAction(Some(sel!(selectTabByNumber:))) };
        item.setTag(i as isize);
        item.setKeyEquivalent(&NSString::from_str(&format!("{}", i)));
        item.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
        menu.addItem(&item);
    }

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Bring All to Front
    menu.addItem(&create_menu_item(
        mtm,
        "Bring All to Front",
        Some(sel!(arrangeInFront:)),
        "",
    ));

    let menu_item = NSMenuItem::new(mtm);
    menu_item.setSubmenu(Some(&menu));
    menu_item
}

fn create_help_menu(mtm: MainThreadMarker) -> Retained<NSMenuItem> {
    let menu = NSMenu::new(mtm);
    menu.setTitle(&NSString::from_str("Help"));

    // Help item (macOS standard)
    menu.addItem(&create_menu_item(
        mtm,
        "cterm Help",
        Some(sel!(showHelp:)),
        "",
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Check for Updates
    menu.addItem(&create_menu_item(
        mtm,
        "Check for Updates...",
        Some(sel!(checkForUpdates:)),
        "",
    ));

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Debug submenu (hidden by default, shown when Shift is held)
    let debug_menu = NSMenu::new(mtm);
    debug_menu.setTitle(&NSString::from_str("Debug"));

    debug_menu.addItem(&create_menu_item(
        mtm,
        "Re-launch cterm",
        Some(sel!(debugRelaunch:)),
        "",
    ));

    debug_menu.addItem(&create_menu_item(
        mtm,
        "Dump State",
        Some(sel!(debugDumpState:)),
        "",
    ));

    debug_menu.addItem(&create_menu_item(
        mtm,
        "View Logs",
        Some(sel!(showLogs:)),
        "",
    ));

    debug_menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Log Level submenu
    let log_level_menu = NSMenu::new(mtm);
    log_level_menu.setTitle(&NSString::from_str("Log Level"));

    log_level_menu.addItem(&create_menu_item(
        mtm,
        "Error",
        Some(sel!(setLogLevelError:)),
        "",
    ));
    log_level_menu.addItem(&create_menu_item(
        mtm,
        "Warn",
        Some(sel!(setLogLevelWarn:)),
        "",
    ));
    log_level_menu.addItem(&create_menu_item(
        mtm,
        "Info",
        Some(sel!(setLogLevelInfo:)),
        "",
    ));
    log_level_menu.addItem(&create_menu_item(
        mtm,
        "Debug",
        Some(sel!(setLogLevelDebug:)),
        "",
    ));
    log_level_menu.addItem(&create_menu_item(
        mtm,
        "Trace",
        Some(sel!(setLogLevelTrace:)),
        "",
    ));

    let log_level_item = NSMenuItem::new(mtm);
    log_level_item.setTitle(&NSString::from_str("Log Level"));
    log_level_item.setSubmenu(Some(&log_level_menu));
    debug_menu.addItem(&log_level_item);

    debug_menu.addItem(&NSMenuItem::separatorItem(mtm));

    debug_menu.addItem(&create_menu_item(
        mtm,
        "Crash (Test Recovery)",
        Some(sel!(debugCrash:)),
        "",
    ));

    let debug_item = NSMenuItem::new(mtm);
    debug_item.setTitle(&NSString::from_str("Debug"));
    debug_item.setSubmenu(Some(&debug_menu));
    debug_item.setHidden(true); // Hidden by default
    menu.addItem(&debug_item);

    // Store the debug menu item globally so we can show/hide it
    set_debug_menu_item(debug_item);

    let menu_item = NSMenuItem::new(mtm);
    menu_item.setSubmenu(Some(&menu));
    menu_item
}

/// Create a menu item without keyboard shortcut
fn create_menu_item(
    mtm: MainThreadMarker,
    title: &str,
    action: Option<Sel>,
    key_equivalent: &str,
) -> Retained<NSMenuItem> {
    let item = NSMenuItem::new(mtm);
    item.setTitle(&NSString::from_str(title));
    if let Some(action) = action {
        unsafe { item.setAction(Some(action)) };
    }
    item.setKeyEquivalent(&NSString::from_str(key_equivalent));
    item
}

/// Create a menu item with keyboard shortcut and modifiers
fn create_menu_item_with_key(
    mtm: MainThreadMarker,
    title: &str,
    action: Option<Sel>,
    key_equivalent: &str,
    modifiers: NSEventModifierFlags,
) -> Retained<NSMenuItem> {
    let item = create_menu_item(mtm, title, action, key_equivalent);
    item.setKeyEquivalentModifierMask(modifiers);
    item
}
