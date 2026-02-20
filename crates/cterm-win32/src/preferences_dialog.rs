//! Preferences dialog for cterm settings
//!
//! Provides a multi-tab dialog for configuring general settings, appearance,
//! tab behavior, and keyboard shortcuts.

use std::cell::RefCell;
use std::ptr;

use winapi::shared::basetsd::INT_PTR;
use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::um::commctrl::*;
use winapi::um::winuser::*;

use std::time::{SystemTime, UNIX_EPOCH};

use crate::dialog_utils::*;
use cterm_app::config::{
    config_dir, Config, CursorStyleConfig, NewTabPosition, TabBarPosition, TabBarVisibility,
};
use cterm_app::{git_sync, PullResult};

// Control IDs - General tab
const IDC_TABS: i32 = 1001;
const IDC_SCROLLBACK_EDIT: i32 = 1010;
const IDC_SCROLLBACK_SPIN: i32 = 1011;
const IDC_CONFIRM_CLOSE: i32 = 1012;
const IDC_COPY_ON_SELECT: i32 = 1013;

// Control IDs - Appearance tab
const IDC_THEME: i32 = 1020;
const IDC_FONT_FAMILY: i32 = 1021;
const IDC_FONT_SIZE_EDIT: i32 = 1022;
const IDC_FONT_SIZE_SPIN: i32 = 1023;
const IDC_CURSOR_STYLE: i32 = 1024;
const IDC_CURSOR_BLINK: i32 = 1025;
const IDC_OPACITY_TRACK: i32 = 1026;
const IDC_OPACITY_LABEL: i32 = 1027;
const IDC_BOLD_BRIGHT: i32 = 1028;

// Control IDs - Tabs tab
const IDC_SHOW_TABBAR: i32 = 1030;
const IDC_TAB_POSITION: i32 = 1031;
const IDC_NEW_TAB_POS: i32 = 1032;
const IDC_SHOW_CLOSE_BTN: i32 = 1033;

// Control IDs - Shortcuts tab
const IDC_SHORTCUTS_LIST: i32 = 1040;

// Control IDs - Git Sync tab
const IDC_GIT_REMOTE: i32 = 1050;
const IDC_GIT_STATUS: i32 = 1051;
const IDC_GIT_BRANCH: i32 = 1052;
const IDC_GIT_LAST_SYNC: i32 = 1053;
const IDC_GIT_CHANGES: i32 = 1054;
const IDC_GIT_SYNC_NOW: i32 = 1055;

// Button IDs
const IDC_APPLY: i32 = 1099;

// Tab indices
const TAB_GENERAL: i32 = 0;
const TAB_APPEARANCE: i32 = 1;
const TAB_TABS: i32 = 2;
const TAB_SHORTCUTS: i32 = 3;
const TAB_GIT_SYNC: i32 = 4;

/// Dialog state
struct DialogState {
    config: Config,
    current_tab: i32,
    // Control handles for each tab
    general_controls: Vec<HWND>,
    appearance_controls: Vec<HWND>,
    tabs_controls: Vec<HWND>,
    shortcuts_controls: Vec<HWND>,
    git_sync_controls: Vec<HWND>,
}

// Thread-local storage for dialog state
thread_local! {
    static DIALOG_STATE: RefCell<Option<DialogState>> = const { RefCell::new(None) };
}

/// Show the preferences dialog
///
/// Returns true if settings were saved, false if cancelled.
pub fn show_preferences_dialog(parent: HWND) -> bool {
    // Load current config
    let config = cterm_app::load_config().unwrap_or_default();

    DIALOG_STATE.with(|s| {
        *s.borrow_mut() = Some(DialogState {
            config,
            current_tab: TAB_GENERAL,
            general_controls: Vec::new(),
            appearance_controls: Vec::new(),
            tabs_controls: Vec::new(),
            shortcuts_controls: Vec::new(),
            git_sync_controls: Vec::new(),
        });
    });

    // Build and show dialog
    let template = build_dialog_template();
    let ret = unsafe {
        DialogBoxIndirectParamW(
            ptr::null_mut(),
            template.as_ptr() as *const DLGTEMPLATE,
            parent,
            Some(dialog_proc),
            0,
        )
    };

    // Clean up state
    DIALOG_STATE.with(|s| {
        *s.borrow_mut() = None;
    });

    ret == IDOK as isize
}

/// Build the dialog template
fn build_dialog_template() -> Vec<u8> {
    let mut template = Vec::new();

    // Dialog dimensions (dialog units)
    let width: i16 = 340; // ~500 pixels
    let height: i16 = 280; // ~420 pixels

    let style = DS_MODALFRAME | DS_CENTER | WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_SETFONT;
    let ex_style = 0u32;
    let c_dit = 0u16;
    let x = 0i16;
    let y = 0i16;

    template.extend_from_slice(&style.to_le_bytes());
    template.extend_from_slice(&ex_style.to_le_bytes());
    template.extend_from_slice(&c_dit.to_le_bytes());
    template.extend_from_slice(&x.to_le_bytes());
    template.extend_from_slice(&y.to_le_bytes());
    template.extend_from_slice(&width.to_le_bytes());
    template.extend_from_slice(&height.to_le_bytes());

    // Menu (none)
    template.extend_from_slice(&[0u8, 0]);
    // Class (use default)
    template.extend_from_slice(&[0u8, 0]);
    // Title
    let title = to_wide("Preferences");
    for c in &title {
        template.extend_from_slice(&c.to_le_bytes());
    }

    // Font
    align_to_word(&mut template);
    template.extend_from_slice(&9u16.to_le_bytes());
    let font = to_wide("Segoe UI");
    for c in &font {
        template.extend_from_slice(&c.to_le_bytes());
    }

    template
}

fn align_to_word(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(2) {
        v.push(0);
    }
}

/// Dialog procedure
unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> INT_PTR {
    match msg {
        WM_INITDIALOG => {
            init_dialog(hwnd);
            1
        }
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as i32;
            handle_command(hwnd, id);
            1
        }
        WM_NOTIFY => {
            let nmhdr = lparam as *const NMHDR;
            if !nmhdr.is_null() {
                handle_notify(hwnd, &*nmhdr);
            }
            0
        }
        WM_HSCROLL => {
            // Trackbar changed
            let track = lparam as HWND;
            if !track.is_null() {
                handle_trackbar_change(hwnd, track);
            }
            0
        }
        WM_CLOSE => {
            EndDialog(hwnd, IDCANCEL as isize);
            1
        }
        _ => 0,
    }
}

/// Initialize the dialog
unsafe fn init_dialog(hwnd: HWND) {
    let mut rect = std::mem::zeroed();
    GetClientRect(hwnd, &mut rect);
    let dlg_width = rect.right - rect.left;
    let dlg_height = rect.bottom - rect.top;

    let margin = 10;
    let button_height = 25;
    let button_width = 75;
    let tab_height = 28;

    // Create tab control
    let tab_ctrl = create_tab_control(
        hwnd,
        IDC_TABS,
        margin,
        margin,
        dlg_width - margin * 2,
        tab_height,
    );
    add_tab(tab_ctrl, TAB_GENERAL, "General");
    add_tab(tab_ctrl, TAB_APPEARANCE, "Appearance");
    add_tab(tab_ctrl, TAB_TABS, "Tabs");
    add_tab(tab_ctrl, TAB_SHORTCUTS, "Shortcuts");
    add_tab(tab_ctrl, TAB_GIT_SYNC, "Git Sync");

    // Content area
    let content_top = margin + tab_height + 5;
    let content_height = dlg_height - content_top - button_height - margin * 2 - 5;

    // Create controls for each tab
    create_general_controls(
        hwnd,
        margin,
        content_top,
        dlg_width - margin * 2,
        content_height,
    );
    create_appearance_controls(
        hwnd,
        margin,
        content_top,
        dlg_width - margin * 2,
        content_height,
    );
    create_tabs_controls(
        hwnd,
        margin,
        content_top,
        dlg_width - margin * 2,
        content_height,
    );
    create_shortcuts_controls(
        hwnd,
        margin,
        content_top,
        dlg_width - margin * 2,
        content_height,
    );
    create_git_sync_controls(
        hwnd,
        margin,
        content_top,
        dlg_width - margin * 2,
        content_height,
    );

    // Show only General tab initially
    show_tab(TAB_GENERAL);

    // Create buttons at bottom
    let btn_y = dlg_height - button_height - margin;
    create_button(
        hwnd,
        IDCANCEL,
        "Cancel",
        dlg_width - margin - button_width * 3 - 20,
        btn_y,
        button_width,
        button_height,
    );
    create_button(
        hwnd,
        IDC_APPLY,
        "Apply",
        dlg_width - margin - button_width * 2 - 10,
        btn_y,
        button_width,
        button_height,
    );
    create_default_button(
        hwnd,
        IDOK,
        "OK",
        dlg_width - margin - button_width,
        btn_y,
        button_width,
        button_height,
    );

    // Populate controls with current config
    populate_controls();
}

/// Create controls for the General tab
unsafe fn create_general_controls(hwnd: HWND, x: i32, y: i32, _w: i32, _h: i32) {
    let mut controls = Vec::new();
    let row_height = 26;
    let label_width = 120;
    let control_width = 180;

    // Scrollback lines
    let mut cy = y;
    controls.push(create_label(
        hwnd,
        -1,
        "Scrollback lines:",
        x,
        cy + 3,
        label_width,
        18,
    ));
    let (edit, spin) = create_spinner(
        hwnd,
        IDC_SCROLLBACK_EDIT,
        IDC_SCROLLBACK_SPIN,
        x + label_width + 10,
        cy,
        control_width,
        22,
        0,
        100000,
        10000,
    );
    controls.push(edit);
    controls.push(spin);

    // Confirm close
    cy += row_height + 5;
    controls.push(create_checkbox(
        hwnd,
        IDC_CONFIRM_CLOSE,
        "Confirm close with running process",
        x,
        cy,
        300,
        20,
    ));

    // Copy on select
    cy += row_height;
    controls.push(create_checkbox(
        hwnd,
        IDC_COPY_ON_SELECT,
        "Copy text on selection",
        x,
        cy,
        300,
        20,
    ));

    DIALOG_STATE.with(|s| {
        if let Some(ref mut state) = *s.borrow_mut() {
            state.general_controls = controls;
        }
    });
}

/// Create controls for the Appearance tab
unsafe fn create_appearance_controls(hwnd: HWND, x: i32, y: i32, _w: i32, _h: i32) {
    let mut controls = Vec::new();
    let row_height = 26;
    let label_width = 100;
    let control_width = 180;

    // Theme
    let mut cy = y;
    controls.push(create_label(hwnd, -1, "Theme:", x, cy + 3, label_width, 18));
    let theme_combo = create_combobox(hwnd, IDC_THEME, x + label_width + 10, cy, control_width, 22);
    add_combobox_item(theme_combo, "Default Dark");
    add_combobox_item(theme_combo, "Default Light");
    add_combobox_item(theme_combo, "Tokyo Night");
    add_combobox_item(theme_combo, "Dracula");
    add_combobox_item(theme_combo, "Nord");
    controls.push(theme_combo);

    // Font family
    cy += row_height + 5;
    controls.push(create_label(
        hwnd,
        -1,
        "Font family:",
        x,
        cy + 3,
        label_width,
        18,
    ));
    let font_edit = create_edit(
        hwnd,
        IDC_FONT_FAMILY,
        x + label_width + 10,
        cy,
        control_width,
        22,
    );
    controls.push(font_edit);

    // Font size
    cy += row_height + 5;
    controls.push(create_label(
        hwnd,
        -1,
        "Font size:",
        x,
        cy + 3,
        label_width,
        18,
    ));
    let (edit, spin) = create_spinner(
        hwnd,
        IDC_FONT_SIZE_EDIT,
        IDC_FONT_SIZE_SPIN,
        x + label_width + 10,
        cy,
        80,
        22,
        6,
        72,
        14,
    );
    controls.push(edit);
    controls.push(spin);

    // Cursor style
    cy += row_height + 5;
    controls.push(create_label(
        hwnd,
        -1,
        "Cursor style:",
        x,
        cy + 3,
        label_width,
        18,
    ));
    let cursor_combo = create_combobox(
        hwnd,
        IDC_CURSOR_STYLE,
        x + label_width + 10,
        cy,
        control_width,
        22,
    );
    add_combobox_item(cursor_combo, "Block");
    add_combobox_item(cursor_combo, "Underline");
    add_combobox_item(cursor_combo, "Bar");
    controls.push(cursor_combo);

    // Cursor blink
    cy += row_height + 5;
    controls.push(create_checkbox(
        hwnd,
        IDC_CURSOR_BLINK,
        "Cursor blink",
        x,
        cy,
        200,
        20,
    ));

    // Opacity
    cy += row_height + 5;
    controls.push(create_label(
        hwnd,
        -1,
        "Opacity:",
        x,
        cy + 3,
        label_width,
        18,
    ));
    let trackbar = create_trackbar(
        hwnd,
        IDC_OPACITY_TRACK,
        x + label_width + 10,
        cy,
        150,
        22,
        20,
        100,
    );
    controls.push(trackbar);
    controls.push(create_label(
        hwnd,
        IDC_OPACITY_LABEL,
        "100%",
        x + label_width + 165,
        cy + 3,
        40,
        18,
    ));

    // Bold is bright
    cy += row_height + 5;
    controls.push(create_checkbox(
        hwnd,
        IDC_BOLD_BRIGHT,
        "Bold text is bright",
        x,
        cy,
        200,
        20,
    ));

    DIALOG_STATE.with(|s| {
        if let Some(ref mut state) = *s.borrow_mut() {
            state.appearance_controls = controls;
        }
    });
}

/// Create controls for the Tabs tab
unsafe fn create_tabs_controls(hwnd: HWND, x: i32, y: i32, _w: i32, _h: i32) {
    let mut controls = Vec::new();
    let row_height = 26;
    let label_width = 120;
    let control_width = 150;

    // Show tab bar
    let mut cy = y;
    controls.push(create_label(
        hwnd,
        -1,
        "Show tab bar:",
        x,
        cy + 3,
        label_width,
        18,
    ));
    let show_combo = create_combobox(
        hwnd,
        IDC_SHOW_TABBAR,
        x + label_width + 10,
        cy,
        control_width,
        22,
    );
    add_combobox_item(show_combo, "Always");
    add_combobox_item(show_combo, "When multiple tabs");
    add_combobox_item(show_combo, "Never");
    controls.push(show_combo);

    // Tab bar position
    cy += row_height + 5;
    controls.push(create_label(
        hwnd,
        -1,
        "Tab bar position:",
        x,
        cy + 3,
        label_width,
        18,
    ));
    let pos_combo = create_combobox(
        hwnd,
        IDC_TAB_POSITION,
        x + label_width + 10,
        cy,
        control_width,
        22,
    );
    add_combobox_item(pos_combo, "Top");
    add_combobox_item(pos_combo, "Bottom");
    controls.push(pos_combo);

    // New tab position
    cy += row_height + 5;
    controls.push(create_label(
        hwnd,
        -1,
        "New tab position:",
        x,
        cy + 3,
        label_width,
        18,
    ));
    let new_tab_combo = create_combobox(
        hwnd,
        IDC_NEW_TAB_POS,
        x + label_width + 10,
        cy,
        control_width,
        22,
    );
    add_combobox_item(new_tab_combo, "At end");
    add_combobox_item(new_tab_combo, "After current");
    controls.push(new_tab_combo);

    // Show close button
    cy += row_height + 5;
    controls.push(create_checkbox(
        hwnd,
        IDC_SHOW_CLOSE_BTN,
        "Show close button on tabs",
        x,
        cy,
        250,
        20,
    ));

    DIALOG_STATE.with(|s| {
        if let Some(ref mut state) = *s.borrow_mut() {
            state.tabs_controls = controls;
        }
    });
}

/// Create controls for the Shortcuts tab
unsafe fn create_shortcuts_controls(hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    let mut controls = Vec::new();

    // Create a listview with two columns: Action and Shortcut
    let listview = create_listview(hwnd, IDC_SHORTCUTS_LIST, x, y, w, h - 30);
    add_listview_column(listview, 0, "Action", (w * 40) / 100);
    add_listview_column(listview, 1, "Shortcut", (w * 55) / 100);
    controls.push(listview);

    // Note: Editing shortcuts would require a custom key capture dialog
    // For now, just display them
    controls.push(create_label(
        hwnd,
        -1,
        "Edit shortcuts in config.toml",
        x,
        y + h - 25,
        w,
        20,
    ));

    DIALOG_STATE.with(|s| {
        if let Some(ref mut state) = *s.borrow_mut() {
            state.shortcuts_controls = controls;
        }
    });
}

/// Create controls for the Git Sync tab
unsafe fn create_git_sync_controls(hwnd: HWND, x: i32, y: i32, _w: i32, _h: i32) {
    let mut controls = Vec::new();
    let row_height = 26;
    let label_width = 100;
    let control_width = 200;

    // Get sync status
    let status = config_dir()
        .map(|dir| git_sync::get_sync_status(&dir))
        .unwrap_or_default();

    // Section: Remote Repository
    let mut cy = y;
    controls.push(create_label(hwnd, -1, "Remote Repository", x, cy, 200, 18));

    // Git remote URL
    cy += row_height;
    controls.push(create_label(
        hwnd,
        -1,
        "Git Remote URL:",
        x,
        cy + 3,
        label_width,
        18,
    ));
    let git_edit = create_edit(
        hwnd,
        IDC_GIT_REMOTE,
        x + label_width + 10,
        cy,
        control_width,
        22,
    );
    if let Some(url) = &status.remote_url {
        set_edit_text(git_edit, url);
    }
    controls.push(git_edit);

    // Section: Sync Status
    cy += row_height + 15;
    controls.push(create_label(hwnd, -1, "Sync Status", x, cy, 200, 18));

    // Status
    cy += row_height;
    controls.push(create_label(
        hwnd,
        -1,
        "Status:",
        x,
        cy + 3,
        label_width,
        18,
    ));
    let status_text = if !status.is_repo {
        "Not initialized"
    } else if status.remote_url.is_none() {
        "No remote configured"
    } else {
        "Configured"
    };
    controls.push(create_label(
        hwnd,
        IDC_GIT_STATUS,
        status_text,
        x + label_width + 10,
        cy + 3,
        control_width,
        18,
    ));

    // Branch
    cy += row_height;
    controls.push(create_label(
        hwnd,
        -1,
        "Branch:",
        x,
        cy + 3,
        label_width,
        18,
    ));
    let branch_text = status.branch.clone().unwrap_or_else(|| "-".to_string());
    controls.push(create_label(
        hwnd,
        IDC_GIT_BRANCH,
        &branch_text,
        x + label_width + 10,
        cy + 3,
        control_width,
        18,
    ));

    // Last sync
    cy += row_height;
    controls.push(create_label(
        hwnd,
        -1,
        "Last sync:",
        x,
        cy + 3,
        label_width,
        18,
    ));
    let last_sync_text = if let Some(ts) = status.last_commit_time {
        format_timestamp(ts)
    } else {
        "-".to_string()
    };
    controls.push(create_label(
        hwnd,
        IDC_GIT_LAST_SYNC,
        &last_sync_text,
        x + label_width + 10,
        cy + 3,
        control_width,
        18,
    ));

    // Changes
    cy += row_height;
    controls.push(create_label(
        hwnd,
        -1,
        "Changes:",
        x,
        cy + 3,
        label_width,
        18,
    ));
    let changes_text = if status.has_local_changes {
        "Uncommitted changes"
    } else if status.commits_ahead > 0 && status.commits_behind > 0 {
        "Diverged from remote"
    } else if status.commits_ahead > 0 {
        "Ahead of remote"
    } else if status.commits_behind > 0 {
        "Behind remote"
    } else {
        "Up to date"
    };
    controls.push(create_label(
        hwnd,
        IDC_GIT_CHANGES,
        changes_text,
        x + label_width + 10,
        cy + 3,
        control_width,
        18,
    ));

    // Sync Now button
    cy += row_height + 10;
    controls.push(create_button(
        hwnd,
        IDC_GIT_SYNC_NOW,
        "Sync Now",
        x,
        cy,
        100,
        25,
    ));

    DIALOG_STATE.with(|s| {
        if let Some(ref mut state) = *s.borrow_mut() {
            state.git_sync_controls = controls;
        }
    });
}

/// Format a Unix timestamp as a human-readable relative time
fn format_timestamp(ts: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let diff = now - ts;

    if diff < 60 {
        "Just now".to_string()
    } else if diff < 3600 {
        let mins = diff / 60;
        format!("{} minute{} ago", mins, if mins == 1 { "" } else { "s" })
    } else if diff < 86400 {
        let hours = diff / 3600;
        format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" })
    } else if diff < 604800 {
        let days = diff / 86400;
        format!("{} day{} ago", days, if days == 1 { "" } else { "s" })
    } else {
        let weeks = diff / 604800;
        format!("{} week{} ago", weeks, if weeks == 1 { "" } else { "s" })
    }
}

/// Update Git Sync status labels
fn update_git_status_display(hwnd: HWND) {
    let status = config_dir()
        .map(|dir| git_sync::get_sync_status(&dir))
        .unwrap_or_default();

    let status_label = get_dialog_item(hwnd, IDC_GIT_STATUS);
    let status_text = if !status.is_repo {
        "Not initialized"
    } else if status.remote_url.is_none() {
        "No remote configured"
    } else {
        "Configured"
    };
    set_edit_text(status_label, status_text);

    let branch_label = get_dialog_item(hwnd, IDC_GIT_BRANCH);
    let branch_text = status.branch.clone().unwrap_or_else(|| "-".to_string());
    set_edit_text(branch_label, &branch_text);

    let last_sync_label = get_dialog_item(hwnd, IDC_GIT_LAST_SYNC);
    let last_sync_text = if let Some(ts) = status.last_commit_time {
        format_timestamp(ts)
    } else {
        "-".to_string()
    };
    set_edit_text(last_sync_label, &last_sync_text);

    let changes_label = get_dialog_item(hwnd, IDC_GIT_CHANGES);
    let changes_text = if status.has_local_changes {
        "Uncommitted changes"
    } else if status.commits_ahead > 0 && status.commits_behind > 0 {
        "Diverged from remote"
    } else if status.commits_ahead > 0 {
        "Ahead of remote"
    } else if status.commits_behind > 0 {
        "Behind remote"
    } else {
        "Up to date"
    };
    set_edit_text(changes_label, changes_text);
}

/// Perform git sync now
fn perform_sync_now(hwnd: HWND) {
    let Some(dir) = config_dir() else {
        log::error!("No config directory found");
        return;
    };

    // Get remote URL from the edit field
    let remote_edit = get_dialog_item(hwnd, IDC_GIT_REMOTE);
    let remote_url = get_edit_text(remote_edit);

    // First, check if we need to initialize with remote
    if !remote_url.is_empty() && git_sync::get_remote_url(&dir).is_none() {
        match git_sync::init_with_remote(&dir, &remote_url) {
            Ok(git_sync::InitResult::PulledRemote) => {
                log::info!("Pulled config from remote");
                update_git_status_display(hwnd);
                return;
            }
            Ok(_) => {
                log::info!("Git remote initialized");
            }
            Err(e) => {
                log::error!("Failed to initialize git remote: {}", e);
                crate::dialogs::show_error(
                    hwnd,
                    "Git Sync",
                    &format!("Failed to initialize: {}", e),
                );
                return;
            }
        }
    }

    // Perform sync: pull then push
    match git_sync::pull_with_conflict_resolution(&dir) {
        Ok(PullResult::Updated) => {
            log::info!("Pulled updates from remote");
        }
        Ok(PullResult::ConflictsResolved(files)) => {
            log::info!("Pulled with conflicts resolved: {:?}", files);
        }
        Ok(PullResult::UpToDate) => {
            log::info!("Already up to date");
        }
        Ok(PullResult::NoRemote) | Ok(PullResult::NotARepo) => {
            log::info!("No remote configured or not a repo");
        }
        Err(e) => {
            log::error!("Sync failed: {}", e);
            crate::dialogs::show_error(hwnd, "Git Sync", &format!("Sync failed: {}", e));
        }
    }

    // Push any local changes
    if git_sync::is_git_repo(&dir) {
        if let Err(e) = git_sync::commit_and_push(&dir, "Sync configuration") {
            log::error!("Failed to push: {}", e);
        }
    }

    update_git_status_display(hwnd);
}

/// Show controls for a specific tab, hide others
fn show_tab(tab_index: i32) {
    DIALOG_STATE.with(|s| {
        if let Some(ref state) = *s.borrow() {
            // Hide all
            for hwnd in &state.general_controls {
                show_control(*hwnd, false);
            }
            for hwnd in &state.appearance_controls {
                show_control(*hwnd, false);
            }
            for hwnd in &state.tabs_controls {
                show_control(*hwnd, false);
            }
            for hwnd in &state.shortcuts_controls {
                show_control(*hwnd, false);
            }
            for hwnd in &state.git_sync_controls {
                show_control(*hwnd, false);
            }

            // Show the selected tab's controls
            let controls = match tab_index {
                TAB_GENERAL => &state.general_controls,
                TAB_APPEARANCE => &state.appearance_controls,
                TAB_TABS => &state.tabs_controls,
                TAB_SHORTCUTS => &state.shortcuts_controls,
                TAB_GIT_SYNC => &state.git_sync_controls,
                _ => &state.general_controls,
            };
            for hwnd in controls {
                show_control(*hwnd, true);
            }
        }
    });
}

/// Populate controls with current config values
fn populate_controls() {
    DIALOG_STATE.with(|s| {
        if let Some(ref state) = *s.borrow() {
            let config = &state.config;

            // General tab
            if let Some(&edit) = state.general_controls.get(1) {
                set_edit_text(edit, &config.general.scrollback_lines.to_string());
            }
            if let Some(&checkbox) = state.general_controls.get(3) {
                set_checkbox_state(checkbox, config.general.confirm_close_with_running);
            }
            if let Some(&checkbox) = state.general_controls.get(4) {
                set_checkbox_state(checkbox, config.general.copy_on_select);
            }

            // Appearance tab
            if let Some(&combo) = state.appearance_controls.get(1) {
                let idx = match config.appearance.theme.as_str() {
                    "Default Dark" | "dark" => 0,
                    "Default Light" | "light" => 1,
                    "Tokyo Night" | "tokyo-night" => 2,
                    "Dracula" | "dracula" => 3,
                    "Nord" | "nord" => 4,
                    _ => 0,
                };
                set_combobox_selection(combo, idx);
            }
            if let Some(&edit) = state.appearance_controls.get(3) {
                set_edit_text(edit, &config.appearance.font.family);
            }
            if let Some(&edit) = state.appearance_controls.get(5) {
                set_edit_text(edit, &config.appearance.font.size.to_string());
            }
            if let Some(&combo) = state.appearance_controls.get(8) {
                let idx = match config.appearance.cursor_style {
                    CursorStyleConfig::Block => 0,
                    CursorStyleConfig::Underline => 1,
                    CursorStyleConfig::Bar => 2,
                };
                set_combobox_selection(combo, idx);
            }
            if let Some(&checkbox) = state.appearance_controls.get(9) {
                set_checkbox_state(checkbox, config.appearance.cursor_blink);
            }
            if let Some(&trackbar) = state.appearance_controls.get(11) {
                let opacity_pct = (config.appearance.opacity * 100.0) as i32;
                set_trackbar_value(trackbar, opacity_pct);
            }
            if let Some(&label) = state.appearance_controls.get(12) {
                let opacity_pct = (config.appearance.opacity * 100.0) as i32;
                set_edit_text(label, &format!("{}%", opacity_pct));
            }
            if let Some(&checkbox) = state.appearance_controls.get(13) {
                set_checkbox_state(checkbox, config.appearance.bold_is_bright);
            }

            // Tabs tab
            if let Some(&combo) = state.tabs_controls.get(1) {
                let idx = match config.tabs.show_tab_bar {
                    TabBarVisibility::Always => 0,
                    TabBarVisibility::Multiple => 1,
                    TabBarVisibility::Never => 2,
                };
                set_combobox_selection(combo, idx);
            }
            if let Some(&combo) = state.tabs_controls.get(3) {
                let idx = match config.tabs.tab_bar_position {
                    TabBarPosition::Top => 0,
                    TabBarPosition::Bottom => 1,
                };
                set_combobox_selection(combo, idx);
            }
            if let Some(&combo) = state.tabs_controls.get(5) {
                let idx = match config.tabs.new_tab_position {
                    NewTabPosition::End => 0,
                    NewTabPosition::AfterCurrent => 1,
                };
                set_combobox_selection(combo, idx);
            }
            if let Some(&checkbox) = state.tabs_controls.get(6) {
                set_checkbox_state(checkbox, config.tabs.show_close_button);
            }

            // Shortcuts tab - populate listview
            if let Some(&listview) = state.shortcuts_controls.first() {
                let shortcuts = &config.shortcuts;
                let items = [
                    ("New Tab", &shortcuts.new_tab),
                    ("Close Tab", &shortcuts.close_tab),
                    ("Next Tab", &shortcuts.next_tab),
                    ("Previous Tab", &shortcuts.prev_tab),
                    ("New Window", &shortcuts.new_window),
                    ("Close Window", &shortcuts.close_window),
                    ("Copy", &shortcuts.copy),
                    ("Paste", &shortcuts.paste),
                    ("Select All", &shortcuts.select_all),
                    ("Zoom In", &shortcuts.zoom_in),
                    ("Zoom Out", &shortcuts.zoom_out),
                    ("Zoom Reset", &shortcuts.zoom_reset),
                    ("Find", &shortcuts.find),
                    ("Reset Terminal", &shortcuts.reset),
                ];

                for (i, (action, shortcut)) in items.iter().enumerate() {
                    let idx = add_listview_item(listview, i as i32, action);
                    set_listview_subitem(listview, idx, 1, shortcut);
                }
            }
        }
    });
}

/// Collect values from controls into config
fn collect_config() -> Config {
    let mut config = Config::default();

    DIALOG_STATE.with(|s| {
        if let Some(ref state) = *s.borrow() {
            // Start with current config
            config = state.config.clone();

            // General tab
            if let Some(&edit) = state.general_controls.get(1) {
                if let Ok(lines) = get_edit_text(edit).parse() {
                    config.general.scrollback_lines = lines;
                }
            }
            if let Some(&checkbox) = state.general_controls.get(3) {
                config.general.confirm_close_with_running = get_checkbox_state(checkbox);
            }
            if let Some(&checkbox) = state.general_controls.get(4) {
                config.general.copy_on_select = get_checkbox_state(checkbox);
            }

            // Appearance tab
            if let Some(&combo) = state.appearance_controls.get(1) {
                config.appearance.theme = match get_combobox_selection(combo) {
                    Some(0) => "dark".to_string(),
                    Some(1) => "light".to_string(),
                    Some(2) => "tokyo_night".to_string(),
                    Some(3) => "dracula".to_string(),
                    Some(4) => "nord".to_string(),
                    _ => "dark".to_string(),
                };
            }
            if let Some(&edit) = state.appearance_controls.get(3) {
                config.appearance.font.family = get_edit_text(edit);
            }
            if let Some(&edit) = state.appearance_controls.get(5) {
                if let Ok(size) = get_edit_text(edit).parse() {
                    config.appearance.font.size = size;
                }
            }
            if let Some(&combo) = state.appearance_controls.get(8) {
                config.appearance.cursor_style = match get_combobox_selection(combo) {
                    Some(0) => CursorStyleConfig::Block,
                    Some(1) => CursorStyleConfig::Underline,
                    Some(2) => CursorStyleConfig::Bar,
                    _ => CursorStyleConfig::Block,
                };
            }
            if let Some(&checkbox) = state.appearance_controls.get(9) {
                config.appearance.cursor_blink = get_checkbox_state(checkbox);
            }
            if let Some(&trackbar) = state.appearance_controls.get(11) {
                let opacity_pct = get_trackbar_value(trackbar);
                config.appearance.opacity = (opacity_pct as f64) / 100.0;
            }
            if let Some(&checkbox) = state.appearance_controls.get(13) {
                config.appearance.bold_is_bright = get_checkbox_state(checkbox);
            }

            // Tabs tab
            if let Some(&combo) = state.tabs_controls.get(1) {
                config.tabs.show_tab_bar = match get_combobox_selection(combo) {
                    Some(0) => TabBarVisibility::Always,
                    Some(1) => TabBarVisibility::Multiple,
                    Some(2) => TabBarVisibility::Never,
                    _ => TabBarVisibility::Always,
                };
            }
            if let Some(&combo) = state.tabs_controls.get(3) {
                config.tabs.tab_bar_position = match get_combobox_selection(combo) {
                    Some(0) => TabBarPosition::Top,
                    Some(1) => TabBarPosition::Bottom,
                    _ => TabBarPosition::Top,
                };
            }
            if let Some(&combo) = state.tabs_controls.get(5) {
                config.tabs.new_tab_position = match get_combobox_selection(combo) {
                    Some(0) => NewTabPosition::End,
                    Some(1) => NewTabPosition::AfterCurrent,
                    _ => NewTabPosition::End,
                };
            }
            if let Some(&checkbox) = state.tabs_controls.get(6) {
                config.tabs.show_close_button = get_checkbox_state(checkbox);
            }
        }
    });

    config
}

/// Save the current config
fn save_config(_hwnd: HWND) -> Result<(), cterm_app::config::ConfigError> {
    let config = collect_config();
    cterm_app::save_config(&config)?;

    // If git sync is configured, commit and push
    if let Some(dir) = config_dir() {
        if git_sync::is_git_repo(&dir) && git_sync::get_remote_url(&dir).is_some() {
            if let Err(e) = git_sync::commit_and_push(&dir, "Update configuration") {
                log::error!("Failed to push config: {}", e);
            }
        }
    }

    Ok(())
}

/// Handle WM_COMMAND
fn handle_command(hwnd: HWND, id: i32) {
    match id {
        IDOK => {
            if save_config(hwnd).is_ok() {
                unsafe { EndDialog(hwnd, IDOK as isize) };
            } else {
                crate::dialogs::show_error(hwnd, "Error", "Failed to save configuration");
            }
        }
        IDC_APPLY => {
            if save_config(hwnd).is_err() {
                crate::dialogs::show_error(hwnd, "Error", "Failed to save configuration");
            }
        }
        IDC_GIT_SYNC_NOW => {
            perform_sync_now(hwnd);
        }
        IDCANCEL => {
            unsafe { EndDialog(hwnd, IDCANCEL as isize) };
        }
        _ => {}
    }
}

/// Handle WM_NOTIFY
fn handle_notify(hwnd: HWND, nmhdr: &NMHDR) {
    match nmhdr.code {
        TCN_SELCHANGE if nmhdr.idFrom == IDC_TABS as usize => {
            let tab_ctrl = get_dialog_item(hwnd, IDC_TABS);
            let new_tab = get_selected_tab(tab_ctrl);

            DIALOG_STATE.with(|s| {
                if let Some(ref mut state) = *s.borrow_mut() {
                    state.current_tab = new_tab;
                }
            });

            show_tab(new_tab);
        }
        _ => {}
    }
}

/// Handle trackbar (slider) changes
fn handle_trackbar_change(hwnd: HWND, trackbar: HWND) {
    // Check if this is the opacity trackbar
    let opacity_track = get_dialog_item(hwnd, IDC_OPACITY_TRACK);
    if trackbar == opacity_track {
        let value = get_trackbar_value(trackbar);
        let label = get_dialog_item(hwnd, IDC_OPACITY_LABEL);
        set_edit_text(label, &format!("{}%", value));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_tab_indices() {
        use super::*;
        assert_eq!(TAB_GENERAL, 0);
        assert_eq!(TAB_APPEARANCE, 1);
        assert_eq!(TAB_TABS, 2);
        assert_eq!(TAB_SHORTCUTS, 3);
    }
}
