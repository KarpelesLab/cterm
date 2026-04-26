//! Session picker and SSH connect dialogs for Win32
//!
//! Provides dialogs for attaching to existing daemon sessions and connecting
//! to remote hosts via SSH.

use std::cell::RefCell;
use std::ptr;

use winapi::shared::basetsd::INT_PTR;
use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::um::commctrl::*;
use winapi::um::winuser::*;

use crate::dialog_utils::*;

// ============================================================================
// Session Picker Dialog
// ============================================================================

/// Information about a daemon session for display
#[derive(Clone)]
struct SessionEntry {
    session_id: String,
    title: String,
    cols: u32,
    rows: u32,
    running: bool,
}

struct SessionPickerState {
    sessions: Vec<SessionEntry>,
    error_message: Option<String>,
}

thread_local! {
    static SESSION_STATE: RefCell<Option<SessionPickerState>> = const { RefCell::new(None) };
    static SESSION_RESULT: RefCell<Option<String>> = const { RefCell::new(None) };
}

// Control IDs for session picker
const IDC_SESSION_LIST: i32 = 1001;
const IDC_SESSION_STATUS: i32 = 1002;
const IDC_SESSION_REFRESH: i32 = 1003;

/// Show the session picker dialog.
///
/// Connects to the local daemon, lists available sessions, and returns
/// the selected session ID, or None if cancelled.
pub fn show_session_picker(parent: HWND) -> Option<String> {
    // Query daemon for sessions (synchronous via tokio runtime)
    let mut state = SessionPickerState {
        sessions: Vec::new(),
        error_message: None,
    };

    match query_sessions() {
        Ok(sessions) => state.sessions = sessions,
        Err(e) => state.error_message = Some(e),
    }

    SESSION_STATE.with(|s| {
        *s.borrow_mut() = Some(state);
    });
    SESSION_RESULT.with(|r| {
        *r.borrow_mut() = None;
    });

    let template = build_session_dialog_template();
    let ret = unsafe {
        DialogBoxIndirectParamW(
            ptr::null_mut(),
            template.as_ptr() as *const DLGTEMPLATE,
            parent,
            Some(session_dialog_proc),
            0,
        )
    };

    SESSION_STATE.with(|s| {
        *s.borrow_mut() = None;
    });

    if ret == IDOK as isize {
        SESSION_RESULT.with(|r| r.borrow().clone())
    } else {
        None
    }
}

/// Query daemon sessions via tokio runtime
fn query_sessions() -> Result<Vec<SessionEntry>, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("Failed to create runtime: {}", e))?;

    rt.block_on(async {
        let conn = cterm_client::DaemonConnection::connect_local()
            .await
            .map_err(|e| format!("Failed to connect to daemon: {}", e))?;
        let sessions = conn
            .list_sessions()
            .await
            .map_err(|e| format!("Failed to list sessions: {}", e))?;
        Ok(sessions
            .into_iter()
            .map(|s| SessionEntry {
                session_id: s.session_id,
                title: s.title,
                cols: s.cols,
                rows: s.rows,
                running: s.running,
            })
            .collect())
    })
}

fn build_session_dialog_template() -> Vec<u8> {
    let mut template = Vec::new();
    let width: i16 = 300;
    let height: i16 = 220;
    let style = DS_MODALFRAME | DS_CENTER | WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_SETFONT;
    let ex_style = 0u32;
    let c_dit = 0u16;

    template.extend_from_slice(&style.to_le_bytes());
    template.extend_from_slice(&ex_style.to_le_bytes());
    template.extend_from_slice(&c_dit.to_le_bytes());
    template.extend_from_slice(&0i16.to_le_bytes());
    template.extend_from_slice(&0i16.to_le_bytes());
    template.extend_from_slice(&width.to_le_bytes());
    template.extend_from_slice(&height.to_le_bytes());

    // Menu (none)
    template.extend_from_slice(&[0u8, 0]);
    // Class (default)
    template.extend_from_slice(&[0u8, 0]);
    // Title
    let title = to_wide("Attach to Session");
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

unsafe extern "system" fn session_dialog_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    _lparam: LPARAM,
) -> INT_PTR {
    match msg {
        WM_INITDIALOG => {
            init_session_dialog(hwnd);
            1
        }
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as i32;
            handle_session_command(hwnd, id);
            1
        }
        WM_NOTIFY => {
            let nmhdr = _lparam as *const NMHDR;
            if !nmhdr.is_null()
                && (*nmhdr).code == NM_DBLCLK
                && (*nmhdr).idFrom == IDC_SESSION_LIST as usize
                && try_select_session(hwnd)
            {
                EndDialog(hwnd, IDOK as isize);
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

unsafe fn init_session_dialog(hwnd: HWND) {
    let mut rect = std::mem::zeroed();
    GetClientRect(hwnd, &mut rect);
    let dlg_width = rect.right - rect.left;
    let dlg_height = rect.bottom - rect.top;
    let margin = 10;
    let button_height = 25;
    let button_width = 80;

    // Info label
    create_label(
        hwnd,
        -1,
        "Select a daemon session to attach to:",
        margin,
        margin,
        dlg_width - margin * 2,
        20,
    );

    // List view
    let list_top = margin + 25;
    let list_height = dlg_height - list_top - button_height - margin * 2 - 25;
    let listview = create_listview(
        hwnd,
        IDC_SESSION_LIST,
        margin,
        list_top,
        dlg_width - margin * 2,
        list_height,
    );

    let list_width = dlg_width - margin * 2 - 4;
    add_listview_column(listview, 0, "Status", (list_width * 15) / 100);
    add_listview_column(listview, 1, "Title", (list_width * 50) / 100);
    add_listview_column(listview, 2, "Size", (list_width * 20) / 100);
    add_listview_column(listview, 3, "ID", (list_width * 15) / 100);

    // Status label
    let status_y = list_top + list_height + 5;
    create_label(
        hwnd,
        IDC_SESSION_STATUS,
        "",
        margin,
        status_y,
        dlg_width - margin * 2,
        20,
    );

    // Buttons
    let btn_y = dlg_height - button_height - margin;
    create_button(
        hwnd,
        IDC_SESSION_REFRESH,
        "Refresh",
        margin,
        btn_y,
        button_width,
        button_height,
    );
    create_button(
        hwnd,
        IDCANCEL,
        "Cancel",
        dlg_width - margin - button_width * 2 - 10,
        btn_y,
        button_width,
        button_height,
    );
    create_default_button(
        hwnd,
        IDOK,
        "Attach",
        dlg_width - margin - button_width,
        btn_y,
        button_width,
        button_height,
    );

    // Populate
    SESSION_STATE.with(|state| {
        if let Some(ref state) = *state.borrow() {
            if let Some(ref err) = state.error_message {
                let status = get_dialog_item(hwnd, IDC_SESSION_STATUS);
                set_edit_text(status, err);
            } else if state.sessions.is_empty() {
                let status = get_dialog_item(hwnd, IDC_SESSION_STATUS);
                set_edit_text(status, "No sessions available.");
            } else {
                let status = get_dialog_item(hwnd, IDC_SESSION_STATUS);
                set_edit_text(
                    status,
                    &format!("{} session(s) found", state.sessions.len()),
                );
            }
            populate_session_list(listview, &state.sessions);
        }
    });

    // Disable Attach if no items
    let attach_btn = get_dialog_item(hwnd, IDOK);
    let has_selection = get_listview_selection(listview).is_some();
    enable_control(attach_btn, has_selection);
}

fn populate_session_list(listview: HWND, sessions: &[SessionEntry]) {
    clear_listview(listview);
    for (i, session) in sessions.iter().enumerate() {
        let status = if session.running { "Running" } else { "Exited" };
        let idx = add_listview_item(listview, i as i32, status);
        let title = if session.title.is_empty() {
            "Untitled"
        } else {
            &session.title
        };
        set_listview_subitem(listview, idx, 1, title);
        set_listview_subitem(
            listview,
            idx,
            2,
            &format!("{}x{}", session.cols, session.rows),
        );
        let id_short = &session.session_id[..8.min(session.session_id.len())];
        set_listview_subitem(listview, idx, 3, id_short);
    }
    if !sessions.is_empty() {
        select_listview_item(listview, 0);
    }
}

unsafe fn handle_session_command(hwnd: HWND, id: i32) {
    match id {
        IDOK => {
            if try_select_session(hwnd) {
                EndDialog(hwnd, IDOK as isize);
            }
        }
        IDCANCEL => {
            EndDialog(hwnd, IDCANCEL as isize);
        }
        IDC_SESSION_REFRESH => {
            // Re-query sessions
            let result = query_sessions();
            SESSION_STATE.with(|s| {
                if let Some(ref mut state) = *s.borrow_mut() {
                    match result {
                        Ok(sessions) => {
                            state.sessions = sessions;
                            state.error_message = None;
                        }
                        Err(e) => {
                            state.sessions.clear();
                            state.error_message = Some(e);
                        }
                    }
                }
            });
            // Refresh UI
            let listview = get_dialog_item(hwnd, IDC_SESSION_LIST);
            let status = get_dialog_item(hwnd, IDC_SESSION_STATUS);
            SESSION_STATE.with(|s| {
                if let Some(ref state) = *s.borrow() {
                    if let Some(ref err) = state.error_message {
                        set_edit_text(status, err);
                    } else if state.sessions.is_empty() {
                        set_edit_text(status, "No sessions available.");
                    } else {
                        set_edit_text(
                            status,
                            &format!("{} session(s) found", state.sessions.len()),
                        );
                    }
                    populate_session_list(listview, &state.sessions);
                }
            });
            let attach_btn = get_dialog_item(hwnd, IDOK);
            let has_selection = get_listview_selection(listview).is_some();
            enable_control(attach_btn, has_selection);
        }
        _ => {}
    }
}

fn try_select_session(hwnd: HWND) -> bool {
    let listview = get_dialog_item(hwnd, IDC_SESSION_LIST);
    if let Some(idx) = get_listview_selection(listview) {
        SESSION_STATE.with(|state| {
            if let Some(ref state) = *state.borrow() {
                if let Some(session) = state.sessions.get(idx as usize) {
                    SESSION_RESULT.with(|r| {
                        *r.borrow_mut() = Some(session.session_id.clone());
                    });
                }
            }
        });
        SESSION_RESULT.with(|r| r.borrow().is_some())
    } else {
        false
    }
}

// ============================================================================
// SSH Connect Dialog
// ============================================================================

thread_local! {
    static SSH_RESULT: RefCell<Option<String>> = const { RefCell::new(None) };
}

const IDC_SSH_HOST: i32 = 2001;

/// Show the SSH connection dialog.
///
/// Prompts the user for a hostname (user@host) and returns the entered host,
/// or None if cancelled.
pub fn show_ssh_dialog(parent: HWND) -> Option<String> {
    SSH_RESULT.with(|r| {
        *r.borrow_mut() = None;
    });

    let template = build_ssh_dialog_template();
    let ret = unsafe {
        DialogBoxIndirectParamW(
            ptr::null_mut(),
            template.as_ptr() as *const DLGTEMPLATE,
            parent,
            Some(ssh_dialog_proc),
            0,
        )
    };

    if ret == IDOK as isize {
        SSH_RESULT.with(|r| r.borrow().clone())
    } else {
        None
    }
}

fn build_ssh_dialog_template() -> Vec<u8> {
    let mut template = Vec::new();
    let width: i16 = 250;
    let height: i16 = 90;
    let style = DS_MODALFRAME | DS_CENTER | WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_SETFONT;
    let ex_style = 0u32;
    let c_dit = 0u16;

    template.extend_from_slice(&style.to_le_bytes());
    template.extend_from_slice(&ex_style.to_le_bytes());
    template.extend_from_slice(&c_dit.to_le_bytes());
    template.extend_from_slice(&0i16.to_le_bytes());
    template.extend_from_slice(&0i16.to_le_bytes());
    template.extend_from_slice(&width.to_le_bytes());
    template.extend_from_slice(&height.to_le_bytes());

    template.extend_from_slice(&[0u8, 0]); // menu
    template.extend_from_slice(&[0u8, 0]); // class
    let title = to_wide("SSH Remote Terminal");
    for c in &title {
        template.extend_from_slice(&c.to_le_bytes());
    }

    align_to_word(&mut template);
    template.extend_from_slice(&9u16.to_le_bytes());
    let font = to_wide("Segoe UI");
    for c in &font {
        template.extend_from_slice(&c.to_le_bytes());
    }

    template
}

unsafe extern "system" fn ssh_dialog_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    _lparam: LPARAM,
) -> INT_PTR {
    match msg {
        WM_INITDIALOG => {
            init_ssh_dialog(hwnd);
            1
        }
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as i32;
            handle_ssh_command(hwnd, id);
            1
        }
        WM_CLOSE => {
            EndDialog(hwnd, IDCANCEL as isize);
            1
        }
        _ => 0,
    }
}

unsafe fn init_ssh_dialog(hwnd: HWND) {
    let mut rect = std::mem::zeroed();
    GetClientRect(hwnd, &mut rect);
    let dlg_width = rect.right - rect.left;
    let dlg_height = rect.bottom - rect.top;
    let margin = 10;
    let button_height = 25;
    let button_width = 80;

    create_label(
        hwnd,
        -1,
        "Host (e.g. user@hostname or user@hostname:port):",
        margin,
        margin,
        dlg_width - margin * 2,
        20,
    );

    create_edit(
        hwnd,
        IDC_SSH_HOST,
        margin,
        margin + 22,
        dlg_width - margin * 2,
        22,
    );

    let btn_y = dlg_height - button_height - margin;
    create_button(
        hwnd,
        IDCANCEL,
        "Cancel",
        dlg_width - margin - button_width * 2 - 10,
        btn_y,
        button_width,
        button_height,
    );
    create_default_button(
        hwnd,
        IDOK,
        "Connect",
        dlg_width - margin - button_width,
        btn_y,
        button_width,
        button_height,
    );
}

unsafe fn handle_ssh_command(hwnd: HWND, id: i32) {
    match id {
        IDOK => {
            let host_edit = get_dialog_item(hwnd, IDC_SSH_HOST);
            let host = get_edit_text(host_edit);
            if !host.is_empty() {
                SSH_RESULT.with(|r| {
                    *r.borrow_mut() = Some(host);
                });
                EndDialog(hwnd, IDOK as isize);
            }
        }
        IDCANCEL => {
            EndDialog(hwnd, IDCANCEL as isize);
        }
        _ => {}
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn align_to_word(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(2) {
        v.push(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_sessions_entry() {
        let entry = SessionEntry {
            session_id: "abcdef12".to_string(),
            title: "test".to_string(),
            cols: 80,
            rows: 24,
            running: true,
        };
        assert_eq!(entry.session_id, "abcdef12");
        assert!(entry.running);
    }
}
