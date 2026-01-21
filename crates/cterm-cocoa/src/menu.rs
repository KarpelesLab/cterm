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

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    // Close Tab
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Close Tab",
        Some(sel!(closeTab:)),
        "w",
        NSEventModifierFlags::Command,
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

    let menu_item = NSMenuItem::new(mtm);
    menu_item.setSubmenu(Some(&menu));
    menu_item
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

    // Tab navigation
    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Show Previous Tab",
        Some(sel!(previousTab:)),
        "[",
        NSEventModifierFlags::Command.union(NSEventModifierFlags::Shift),
    ));

    menu.addItem(&create_menu_item_with_key(
        mtm,
        "Show Next Tab",
        Some(sel!(nextTab:)),
        "]",
        NSEventModifierFlags::Command.union(NSEventModifierFlags::Shift),
    ));

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
