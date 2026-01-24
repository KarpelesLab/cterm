//! Win32 menu creation and handling
//!
//! Creates the application menu bar with all menu items.

use std::ptr;

use winapi::shared::windef::HMENU;
use winapi::um::winuser::{
    AppendMenuW, CreateMenu, CreatePopupMenu, SetMenu, MF_POPUP, MF_SEPARATOR, MF_STRING,
};

/// Menu action identifiers
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    // File menu
    NewTab = 1001,
    NewWindow = 1002,
    CloseTab = 1003,
    CloseOtherTabs = 1004,
    DockerPicker = 1005,
    Quit = 1006,

    // Edit menu
    Copy = 2001,
    CopyHtml = 2002,
    Paste = 2003,
    SelectAll = 2004,

    // Terminal menu
    SetTitle = 3001,
    SetColor = 3002,
    Find = 3003,
    Reset = 3004,
    ClearReset = 3005,
    SendSignalInt = 3006,
    SendSignalKill = 3007,
    SendSignalHup = 3008,
    SendSignalTerm = 3009,

    // Tabs menu
    PrevTab = 4001,
    NextTab = 4002,
    Tab1 = 4011,
    Tab2 = 4012,
    Tab3 = 4013,
    Tab4 = 4014,
    Tab5 = 4015,
    Tab6 = 4016,
    Tab7 = 4017,
    Tab8 = 4018,
    Tab9 = 4019,

    // Help menu
    Preferences = 5001,
    CheckUpdates = 5002,
    TabTemplates = 5003,
    About = 5004,

    // Debug menu (shown when Shift is held)
    DebugRelaunch = 6001,
    DebugDumpState = 6002,
    ViewLogs = 6003,
}

impl MenuAction {
    /// Convert from u16 ID
    pub fn from_id(id: u16) -> Option<Self> {
        match id {
            1001 => Some(Self::NewTab),
            1002 => Some(Self::NewWindow),
            1003 => Some(Self::CloseTab),
            1004 => Some(Self::CloseOtherTabs),
            1005 => Some(Self::DockerPicker),
            1006 => Some(Self::Quit),
            2001 => Some(Self::Copy),
            2002 => Some(Self::CopyHtml),
            2003 => Some(Self::Paste),
            2004 => Some(Self::SelectAll),
            3001 => Some(Self::SetTitle),
            3002 => Some(Self::SetColor),
            3003 => Some(Self::Find),
            3004 => Some(Self::Reset),
            3005 => Some(Self::ClearReset),
            3006 => Some(Self::SendSignalInt),
            3007 => Some(Self::SendSignalKill),
            3008 => Some(Self::SendSignalHup),
            3009 => Some(Self::SendSignalTerm),
            4001 => Some(Self::PrevTab),
            4002 => Some(Self::NextTab),
            4011 => Some(Self::Tab1),
            4012 => Some(Self::Tab2),
            4013 => Some(Self::Tab3),
            4014 => Some(Self::Tab4),
            4015 => Some(Self::Tab5),
            4016 => Some(Self::Tab6),
            4017 => Some(Self::Tab7),
            4018 => Some(Self::Tab8),
            4019 => Some(Self::Tab9),
            5001 => Some(Self::Preferences),
            5002 => Some(Self::CheckUpdates),
            5003 => Some(Self::TabTemplates),
            5004 => Some(Self::About),
            6001 => Some(Self::DebugRelaunch),
            6002 => Some(Self::DebugDumpState),
            6003 => Some(Self::ViewLogs),
            _ => None,
        }
    }

    /// Get the ID for this action
    pub fn id(self) -> u16 {
        self as u16
    }
}

/// Convert a Rust string to a null-terminated wide string
fn to_wide_string(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Create the main menu bar
pub fn create_menu_bar(show_debug: bool) -> HMENU {
    unsafe {
        let menu_bar = CreateMenu();

        // File menu
        let file_menu = CreatePopupMenu();
        append_menu_item(file_menu, MenuAction::NewTab, "&New Tab\tCtrl+T");
        append_menu_item(file_menu, MenuAction::NewWindow, "New &Window\tCtrl+N");
        append_separator(file_menu);
        append_menu_item(file_menu, MenuAction::CloseTab, "&Close Tab\tCtrl+W");
        append_menu_item(file_menu, MenuAction::CloseOtherTabs, "Close &Other Tabs");
        append_separator(file_menu);
        append_menu_item(file_menu, MenuAction::DockerPicker, "&Docker...");
        append_separator(file_menu);
        append_menu_item(file_menu, MenuAction::Quit, "&Quit\tAlt+F4");
        append_popup_menu(menu_bar, file_menu, "&File");

        // Edit menu
        let edit_menu = CreatePopupMenu();
        append_menu_item(edit_menu, MenuAction::Copy, "&Copy\tCtrl+Shift+C");
        append_menu_item(edit_menu, MenuAction::CopyHtml, "Copy as &HTML");
        append_menu_item(edit_menu, MenuAction::Paste, "&Paste\tCtrl+Shift+V");
        append_separator(edit_menu);
        append_menu_item(
            edit_menu,
            MenuAction::SelectAll,
            "Select &All\tCtrl+Shift+A",
        );
        append_popup_menu(menu_bar, edit_menu, "&Edit");

        // Terminal menu
        let terminal_menu = CreatePopupMenu();
        append_menu_item(terminal_menu, MenuAction::SetTitle, "Set &Title...");
        append_menu_item(terminal_menu, MenuAction::SetColor, "Set &Color...");
        append_separator(terminal_menu);
        append_menu_item(terminal_menu, MenuAction::Find, "&Find...\tCtrl+Shift+F");
        append_separator(terminal_menu);

        // Signal submenu
        let signal_menu = CreatePopupMenu();
        append_menu_item(
            signal_menu,
            MenuAction::SendSignalInt,
            "&Interrupt (SIGINT)",
        );
        append_menu_item(signal_menu, MenuAction::SendSignalKill, "&Kill (SIGKILL)");
        append_menu_item(signal_menu, MenuAction::SendSignalHup, "&Hangup (SIGHUP)");
        append_menu_item(
            signal_menu,
            MenuAction::SendSignalTerm,
            "&Terminate (SIGTERM)",
        );
        append_popup_menu(terminal_menu, signal_menu, "Send &Signal");

        append_separator(terminal_menu);
        append_menu_item(terminal_menu, MenuAction::Reset, "&Reset Terminal");
        append_menu_item(terminal_menu, MenuAction::ClearReset, "Clear and R&eset");
        append_popup_menu(menu_bar, terminal_menu, "&Terminal");

        // Tabs menu
        let tabs_menu = CreatePopupMenu();
        append_menu_item(
            tabs_menu,
            MenuAction::PrevTab,
            "&Previous Tab\tCtrl+Shift+Tab",
        );
        append_menu_item(tabs_menu, MenuAction::NextTab, "&Next Tab\tCtrl+Tab");
        append_separator(tabs_menu);
        append_menu_item(tabs_menu, MenuAction::Tab1, "Tab &1\tAlt+1");
        append_menu_item(tabs_menu, MenuAction::Tab2, "Tab &2\tAlt+2");
        append_menu_item(tabs_menu, MenuAction::Tab3, "Tab &3\tAlt+3");
        append_menu_item(tabs_menu, MenuAction::Tab4, "Tab &4\tAlt+4");
        append_menu_item(tabs_menu, MenuAction::Tab5, "Tab &5\tAlt+5");
        append_menu_item(tabs_menu, MenuAction::Tab6, "Tab &6\tAlt+6");
        append_menu_item(tabs_menu, MenuAction::Tab7, "Tab &7\tAlt+7");
        append_menu_item(tabs_menu, MenuAction::Tab8, "Tab &8\tAlt+8");
        append_menu_item(tabs_menu, MenuAction::Tab9, "Tab &9\tAlt+9");
        append_popup_menu(menu_bar, tabs_menu, "T&abs");

        // Help menu
        let help_menu = CreatePopupMenu();
        append_menu_item(help_menu, MenuAction::Preferences, "&Preferences...");
        append_menu_item(help_menu, MenuAction::TabTemplates, "&Tab Templates...");
        append_separator(help_menu);
        append_menu_item(help_menu, MenuAction::CheckUpdates, "Check for &Updates...");
        append_separator(help_menu);
        append_menu_item(help_menu, MenuAction::About, "&About cterm");
        append_popup_menu(menu_bar, help_menu, "&Help");

        // Debug menu (only shown when Shift is held)
        if show_debug {
            let debug_menu = CreatePopupMenu();
            append_menu_item(debug_menu, MenuAction::ViewLogs, "&View Logs...");
            append_menu_item(debug_menu, MenuAction::DebugDumpState, "&Dump State");
            append_separator(debug_menu);
            append_menu_item(
                debug_menu,
                MenuAction::DebugRelaunch,
                "&Re-launch (Test Upgrade)",
            );
            append_popup_menu(menu_bar, debug_menu, "&Debug");
        }

        menu_bar
    }
}

/// Append a menu item to a menu
fn append_menu_item(menu: HMENU, action: MenuAction, text: &str) {
    let wide = to_wide_string(text);
    unsafe {
        AppendMenuW(menu, MF_STRING, action.id() as usize, wide.as_ptr());
    }
}

/// Append a separator to a menu
fn append_separator(menu: HMENU) {
    unsafe {
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
    }
}

/// Append a popup (submenu) to a menu
fn append_popup_menu(parent: HMENU, child: HMENU, text: &str) {
    let wide = to_wide_string(text);
    unsafe {
        AppendMenuW(parent, MF_POPUP, child as usize, wide.as_ptr());
    }
}

/// Set the menu bar for a window
pub fn set_window_menu(hwnd: winapi::shared::windef::HWND, menu: HMENU) {
    unsafe {
        SetMenu(hwnd, menu);
    }
}

/// Accelerator key definition
#[derive(Debug, Clone)]
pub struct Accelerator {
    pub action: MenuAction,
    pub key: u16,
    pub modifiers: AcceleratorModifiers,
}

bitflags::bitflags! {
    /// Accelerator key modifiers
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct AcceleratorModifiers: u8 {
        const CTRL = 1 << 0;
        const SHIFT = 1 << 1;
        const ALT = 1 << 2;
    }
}

/// Get the default accelerator table
pub fn get_accelerators() -> Vec<Accelerator> {
    use winapi::um::winuser::*;

    vec![
        // File menu
        Accelerator {
            action: MenuAction::NewTab,
            key: 'T' as u16,
            modifiers: AcceleratorModifiers::CTRL,
        },
        Accelerator {
            action: MenuAction::NewWindow,
            key: 'N' as u16,
            modifiers: AcceleratorModifiers::CTRL,
        },
        Accelerator {
            action: MenuAction::CloseTab,
            key: 'W' as u16,
            modifiers: AcceleratorModifiers::CTRL,
        },
        // Edit menu
        Accelerator {
            action: MenuAction::Copy,
            key: 'C' as u16,
            modifiers: AcceleratorModifiers::CTRL | AcceleratorModifiers::SHIFT,
        },
        Accelerator {
            action: MenuAction::Paste,
            key: 'V' as u16,
            modifiers: AcceleratorModifiers::CTRL | AcceleratorModifiers::SHIFT,
        },
        Accelerator {
            action: MenuAction::SelectAll,
            key: 'A' as u16,
            modifiers: AcceleratorModifiers::CTRL | AcceleratorModifiers::SHIFT,
        },
        // Terminal menu
        Accelerator {
            action: MenuAction::Find,
            key: 'F' as u16,
            modifiers: AcceleratorModifiers::CTRL | AcceleratorModifiers::SHIFT,
        },
        // Tabs menu
        Accelerator {
            action: MenuAction::PrevTab,
            key: VK_TAB as u16,
            modifiers: AcceleratorModifiers::CTRL | AcceleratorModifiers::SHIFT,
        },
        Accelerator {
            action: MenuAction::NextTab,
            key: VK_TAB as u16,
            modifiers: AcceleratorModifiers::CTRL,
        },
        Accelerator {
            action: MenuAction::Tab1,
            key: '1' as u16,
            modifiers: AcceleratorModifiers::ALT,
        },
        Accelerator {
            action: MenuAction::Tab2,
            key: '2' as u16,
            modifiers: AcceleratorModifiers::ALT,
        },
        Accelerator {
            action: MenuAction::Tab3,
            key: '3' as u16,
            modifiers: AcceleratorModifiers::ALT,
        },
        Accelerator {
            action: MenuAction::Tab4,
            key: '4' as u16,
            modifiers: AcceleratorModifiers::ALT,
        },
        Accelerator {
            action: MenuAction::Tab5,
            key: '5' as u16,
            modifiers: AcceleratorModifiers::ALT,
        },
        Accelerator {
            action: MenuAction::Tab6,
            key: '6' as u16,
            modifiers: AcceleratorModifiers::ALT,
        },
        Accelerator {
            action: MenuAction::Tab7,
            key: '7' as u16,
            modifiers: AcceleratorModifiers::ALT,
        },
        Accelerator {
            action: MenuAction::Tab8,
            key: '8' as u16,
            modifiers: AcceleratorModifiers::ALT,
        },
        Accelerator {
            action: MenuAction::Tab9,
            key: '9' as u16,
            modifiers: AcceleratorModifiers::ALT,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menu_action_roundtrip() {
        let action = MenuAction::NewTab;
        assert_eq!(MenuAction::from_id(action.id()), Some(action));
    }

    #[test]
    fn test_to_wide_string() {
        let wide = to_wide_string("Test");
        assert_eq!(wide.len(), 5); // "Test" + null terminator
        assert_eq!(wide[4], 0); // null terminator
    }
}
