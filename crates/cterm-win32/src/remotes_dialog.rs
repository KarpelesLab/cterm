//! Manage Remotes dialog for Win32
//!
//! Dialog for adding/removing remote hosts that templates can target.

use std::cell::RefCell;
use std::ptr;

use winapi::shared::basetsd::INT_PTR;
use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::um::winuser::*;

use crate::dialog_utils::*;
use cterm_app::config::{load_config, save_config, ConnectionMethod, ConnectionType, RemoteConfig};

// Control IDs
const IDC_REMOTE_COMBO: i32 = 1001;
const IDC_REMOTE_ADD: i32 = 1002;
const IDC_REMOTE_REMOVE: i32 = 1003;
const IDC_REMOTE_NAME: i32 = 1004;
const IDC_REMOTE_HOST: i32 = 1005;
const IDC_REMOTE_METHOD: i32 = 1006;
const IDC_REMOTE_PROXY: i32 = 1007;
const IDC_REMOTE_CONN_TYPE: i32 = 1008;
const IDC_REMOTE_RELAY_USER: i32 = 1009;
const IDC_REMOTE_RELAY_DEVICE: i32 = 1010;
const IDC_REMOTE_SESSION_NAME: i32 = 1011;

struct RemotesDialogState {
    remotes: Vec<RemoteConfig>,
    current_index: Option<usize>,
    /// Suppress combo change handler during programmatic updates
    updating: bool,
}

thread_local! {
    static REMOTES_STATE: RefCell<Option<RemotesDialogState>> = const { RefCell::new(None) };
    static REMOTES_SAVED: RefCell<bool> = const { RefCell::new(false) };
}

/// Show the Manage Remotes dialog. Returns true if changes were saved.
pub fn show_remotes_dialog(parent: HWND) -> bool {
    let config = load_config().unwrap_or_default();
    let state = RemotesDialogState {
        remotes: config.remotes.clone(),
        current_index: if config.remotes.is_empty() {
            None
        } else {
            Some(0)
        },
        updating: false,
    };

    REMOTES_STATE.with(|s| {
        *s.borrow_mut() = Some(state);
    });
    REMOTES_SAVED.with(|s| {
        *s.borrow_mut() = false;
    });

    let template = build_remotes_dialog_template();
    let _ret = unsafe {
        DialogBoxIndirectParamW(
            ptr::null_mut(),
            template.as_ptr() as *const DLGTEMPLATE,
            parent,
            Some(remotes_dialog_proc),
            0,
        )
    };

    REMOTES_STATE.with(|s| {
        *s.borrow_mut() = None;
    });

    REMOTES_SAVED.with(|s| *s.borrow())
}

fn build_remotes_dialog_template() -> Vec<u8> {
    let mut template = Vec::new();
    let width: i16 = 260;
    let height: i16 = 320;
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
    let title = to_wide("Manage Remotes");
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

unsafe extern "system" fn remotes_dialog_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    _lparam: LPARAM,
) -> INT_PTR {
    match msg {
        WM_INITDIALOG => {
            init_remotes_dialog(hwnd);
            1
        }
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as i32;
            let code = ((wparam >> 16) & 0xFFFF) as u16;
            handle_remotes_command(hwnd, id, code);
            1
        }
        WM_CLOSE => {
            EndDialog(hwnd, IDCANCEL as isize);
            1
        }
        _ => 0,
    }
}

unsafe fn init_remotes_dialog(hwnd: HWND) {
    let mut rect = std::mem::zeroed();
    GetClientRect(hwnd, &mut rect);
    let dlg_width = rect.right - rect.left;
    let dlg_height = rect.bottom - rect.top;
    let margin = 10;
    let button_height = 25;
    let button_width = 80;
    let small_btn = 30;

    // Remote combo + add/remove buttons
    let combo_width = dlg_width - margin * 2 - small_btn * 2 - 10;
    create_combobox(hwnd, IDC_REMOTE_COMBO, margin, margin, combo_width, 22);
    create_button(
        hwnd,
        IDC_REMOTE_ADD,
        "+",
        margin + combo_width + 5,
        margin,
        small_btn,
        button_height,
    );
    create_button(
        hwnd,
        IDC_REMOTE_REMOVE,
        "\u{2212}",
        margin + combo_width + 5 + small_btn + 5,
        margin,
        small_btn,
        button_height,
    );

    // Name field
    let field_y = margin + 30;
    let label_width = 45;
    let edit_x = margin + label_width + 5;
    let edit_width = dlg_width - edit_x - margin;

    create_label(hwnd, -1, "Name:", margin, field_y + 3, label_width, 20);
    create_edit(hwnd, IDC_REMOTE_NAME, edit_x, field_y, edit_width, 22);

    // Host field
    let field_y2 = field_y + 28;
    create_label(hwnd, -1, "Host:", margin, field_y2 + 3, label_width, 20);
    create_edit(hwnd, IDC_REMOTE_HOST, edit_x, field_y2, edit_width, 22);

    // Method combobox
    let field_y3 = field_y2 + 28;
    create_label(hwnd, -1, "Method:", margin, field_y3 + 3, label_width, 20);
    create_combobox(hwnd, IDC_REMOTE_METHOD, edit_x, field_y3, edit_width, 22);
    let method_combo = get_dialog_item(hwnd, IDC_REMOTE_METHOD);
    add_combobox_item(method_combo, "ctermd");
    add_combobox_item(method_combo, "Mosh");
    set_combobox_selection(method_combo, 0);

    // Connection type combobox
    let field_y4 = field_y3 + 28;
    create_label(hwnd, -1, "Type:", margin, field_y4 + 3, label_width, 20);
    create_combobox(hwnd, IDC_REMOTE_CONN_TYPE, edit_x, field_y4, edit_width, 22);
    let conn_type_combo = get_dialog_item(hwnd, IDC_REMOTE_CONN_TYPE);
    add_combobox_item(conn_type_combo, "Direct");
    add_combobox_item(conn_type_combo, "Relay");
    set_combobox_selection(conn_type_combo, 0);

    // Proxy/Relay host field
    let field_y5 = field_y4 + 28;
    create_label(hwnd, -1, "Proxy:", margin, field_y5 + 3, label_width, 20);
    create_edit(hwnd, IDC_REMOTE_PROXY, edit_x, field_y5, edit_width, 22);

    // Relay Username field
    let field_y6 = field_y5 + 28;
    create_label(
        hwnd,
        -1,
        "Relay User:",
        margin,
        field_y6 + 3,
        label_width,
        20,
    );
    create_edit(
        hwnd,
        IDC_REMOTE_RELAY_USER,
        edit_x,
        field_y6,
        edit_width,
        22,
    );

    // Relay Device field
    let field_y7 = field_y6 + 28;
    create_label(hwnd, -1, "Device:", margin, field_y7 + 3, label_width, 20);
    create_edit(
        hwnd,
        IDC_REMOTE_RELAY_DEVICE,
        edit_x,
        field_y7,
        edit_width,
        22,
    );

    // Session Name field
    let field_y8 = field_y7 + 28;
    create_label(hwnd, -1, "Session:", margin, field_y8 + 3, label_width, 20);
    create_edit(
        hwnd,
        IDC_REMOTE_SESSION_NAME,
        edit_x,
        field_y8,
        edit_width,
        22,
    );

    // Cancel / Save buttons at bottom
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
        "Save",
        dlg_width - margin - button_width,
        btn_y,
        button_width,
        button_height,
    );

    // Populate combo and fields
    refresh_combo(hwnd);
    load_selected(hwnd);
}

fn refresh_combo(hwnd: HWND) {
    let combo = get_dialog_item(hwnd, IDC_REMOTE_COMBO);
    clear_combobox(combo);

    REMOTES_STATE.with(|s| {
        if let Some(ref mut state) = *s.borrow_mut() {
            state.updating = true;
            if state.remotes.is_empty() {
                add_combobox_item(combo, "(no remotes)");
                set_combobox_selection(combo, 0);
            } else {
                for remote in &state.remotes {
                    add_combobox_item(combo, &format!("{} ({})", remote.name, remote.host));
                }
                if let Some(idx) = state.current_index {
                    set_combobox_selection(combo, idx as i32);
                }
            }
            state.updating = false;
        }
    });
}

fn load_selected(hwnd: HWND) {
    let name_edit = get_dialog_item(hwnd, IDC_REMOTE_NAME);
    let host_edit = get_dialog_item(hwnd, IDC_REMOTE_HOST);
    let method_combo = get_dialog_item(hwnd, IDC_REMOTE_METHOD);
    let conn_type_combo = get_dialog_item(hwnd, IDC_REMOTE_CONN_TYPE);
    let proxy_edit = get_dialog_item(hwnd, IDC_REMOTE_PROXY);
    let relay_user_edit = get_dialog_item(hwnd, IDC_REMOTE_RELAY_USER);
    let relay_device_edit = get_dialog_item(hwnd, IDC_REMOTE_RELAY_DEVICE);
    let session_name_edit = get_dialog_item(hwnd, IDC_REMOTE_SESSION_NAME);

    REMOTES_STATE.with(|s| {
        if let Some(ref state) = *s.borrow() {
            if let Some(idx) = state.current_index {
                if let Some(remote) = state.remotes.get(idx) {
                    set_edit_text(name_edit, &remote.name);
                    set_edit_text(host_edit, &remote.host);
                    set_combobox_selection(
                        method_combo,
                        match remote.method {
                            ConnectionMethod::Daemon => 0,
                            ConnectionMethod::Mosh => 1,
                        },
                    );
                    set_combobox_selection(
                        conn_type_combo,
                        match remote.connection_type {
                            ConnectionType::Direct => 0,
                            ConnectionType::Relay => 1,
                        },
                    );
                    set_edit_text(proxy_edit, remote.proxy_jump.as_deref().unwrap_or(""));
                    set_edit_text(
                        relay_user_edit,
                        remote.relay_username.as_deref().unwrap_or(""),
                    );
                    set_edit_text(
                        relay_device_edit,
                        remote.relay_device.as_deref().unwrap_or(""),
                    );
                    set_edit_text(
                        session_name_edit,
                        remote.session_name.as_deref().unwrap_or(""),
                    );
                    return;
                }
            }
            set_edit_text(name_edit, "");
            set_edit_text(host_edit, "");
            set_combobox_selection(method_combo, 0);
            set_combobox_selection(conn_type_combo, 0);
            set_edit_text(proxy_edit, "");
            set_edit_text(relay_user_edit, "");
            set_edit_text(relay_device_edit, "");
            set_edit_text(session_name_edit, "");
        }
    });
}

fn save_current_fields(hwnd: HWND) {
    let name = get_edit_text(get_dialog_item(hwnd, IDC_REMOTE_NAME));
    let host = get_edit_text(get_dialog_item(hwnd, IDC_REMOTE_HOST));
    let method = match get_combobox_selection(get_dialog_item(hwnd, IDC_REMOTE_METHOD)) {
        Some(1) => ConnectionMethod::Mosh,
        _ => ConnectionMethod::Daemon,
    };
    let connection_type = match get_combobox_selection(get_dialog_item(hwnd, IDC_REMOTE_CONN_TYPE))
    {
        Some(1) => ConnectionType::Relay,
        _ => ConnectionType::Direct,
    };
    let proxy_jump = opt_text(get_dialog_item(hwnd, IDC_REMOTE_PROXY));
    let relay_username = opt_text(get_dialog_item(hwnd, IDC_REMOTE_RELAY_USER));
    let relay_device = opt_text(get_dialog_item(hwnd, IDC_REMOTE_RELAY_DEVICE));
    let session_name = opt_text(get_dialog_item(hwnd, IDC_REMOTE_SESSION_NAME));

    REMOTES_STATE.with(|s| {
        if let Some(ref mut state) = *s.borrow_mut() {
            if let Some(idx) = state.current_index {
                if let Some(remote) = state.remotes.get_mut(idx) {
                    remote.name = name;
                    remote.host = host;
                    remote.method = method;
                    remote.connection_type = connection_type;
                    remote.proxy_jump = proxy_jump;
                    remote.relay_username = relay_username;
                    remote.relay_device = relay_device;
                    remote.session_name = session_name;
                }
            }
        }
    });
}

fn opt_text(hwnd: HWND) -> Option<String> {
    let val = get_edit_text(hwnd);
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

unsafe fn handle_remotes_command(hwnd: HWND, id: i32, code: u16) {
    match id {
        IDOK => {
            // Save fields for current selection, then persist
            save_current_fields(hwnd);
            REMOTES_STATE.with(|s| {
                if let Some(ref state) = *s.borrow() {
                    let mut config = load_config().unwrap_or_default();
                    config.remotes = state.remotes.clone();
                    if let Err(e) = save_config(&config) {
                        log::error!("Failed to save config: {}", e);
                    }
                }
            });
            REMOTES_SAVED.with(|s| {
                *s.borrow_mut() = true;
            });
            EndDialog(hwnd, IDOK as isize);
        }
        IDCANCEL => {
            EndDialog(hwnd, IDCANCEL as isize);
        }
        IDC_REMOTE_ADD => {
            save_current_fields(hwnd);
            REMOTES_STATE.with(|s| {
                if let Some(ref mut state) = *s.borrow_mut() {
                    let name = format!("remote-{}", state.remotes.len() + 1);
                    state.remotes.push(RemoteConfig {
                        name,
                        host: String::new(),
                        method: Default::default(),
                        connection_type: Default::default(),
                        proxy_jump: None,
                        relay_username: None,
                        relay_device: None,
                        session_name: None,
                        ssh_compression: true,
                    });
                    state.current_index = Some(state.remotes.len() - 1);
                }
            });
            refresh_combo(hwnd);
            load_selected(hwnd);
        }
        IDC_REMOTE_REMOVE => {
            REMOTES_STATE.with(|s| {
                if let Some(ref mut state) = *s.borrow_mut() {
                    if let Some(idx) = state.current_index {
                        if idx < state.remotes.len() {
                            state.remotes.remove(idx);
                        }
                        if state.remotes.is_empty() {
                            state.current_index = None;
                        } else {
                            state.current_index = Some(0);
                        }
                    }
                }
            });
            refresh_combo(hwnd);
            load_selected(hwnd);
        }
        IDC_REMOTE_COMBO if code == CBN_SELCHANGE => {
            let is_updating =
                REMOTES_STATE.with(|s| s.borrow().as_ref().is_some_and(|st| st.updating));
            if !is_updating {
                // Save current fields before switching
                save_current_fields(hwnd);
                let combo = get_dialog_item(hwnd, IDC_REMOTE_COMBO);
                if let Some(idx) = get_combobox_selection(combo) {
                    REMOTES_STATE.with(|s| {
                        if let Some(ref mut state) = *s.borrow_mut() {
                            if (idx as usize) < state.remotes.len() {
                                state.current_index = Some(idx as usize);
                            }
                        }
                    });
                }
                load_selected(hwnd);
            }
        }
        IDC_REMOTE_NAME
        | IDC_REMOTE_HOST
        | IDC_REMOTE_PROXY
        | IDC_REMOTE_RELAY_USER
        | IDC_REMOTE_RELAY_DEVICE
        | IDC_REMOTE_SESSION_NAME
            if code == EN_CHANGE =>
        {
            let is_updating =
                REMOTES_STATE.with(|s| s.borrow().as_ref().is_some_and(|st| st.updating));
            if !is_updating {
                save_current_fields(hwnd);
                refresh_combo(hwnd);
            }
        }
        IDC_REMOTE_METHOD | IDC_REMOTE_CONN_TYPE if code == CBN_SELCHANGE => {
            let is_updating =
                REMOTES_STATE.with(|s| s.borrow().as_ref().is_some_and(|st| st.updating));
            if !is_updating {
                save_current_fields(hwnd);
            }
        }
        _ => {}
    }
}

fn align_to_word(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(2) {
        v.push(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remotes_state() {
        let state = RemotesDialogState {
            remotes: vec![RemoteConfig {
                name: "test".to_string(),
                host: "user@host".to_string(),
                method: Default::default(),
                connection_type: Default::default(),
                proxy_jump: None,
                relay_username: None,
                relay_device: None,
                session_name: None,
                ssh_compression: true,
            }],
            current_index: Some(0),
            updating: false,
        };
        assert_eq!(state.remotes.len(), 1);
        assert_eq!(state.current_index, Some(0));
    }
}
