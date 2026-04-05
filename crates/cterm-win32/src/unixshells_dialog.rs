//! Unix Shells sign-in dialog for Windows.

use std::cell::RefCell;
use std::ptr;
use std::sync::Arc;

use winapi::shared::basetsd::INT_PTR;
use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::um::winuser::*;

use crate::dialog_utils::*;
use cterm_app::unixshells::DeviceService;

const IDC_USERNAME: i32 = 2001;
const IDC_STATUS: i32 = 2002;
const IDC_SIGNIN: i32 = 2003;

thread_local! {
    static DIALOG_DS: RefCell<Option<Arc<DeviceService>>> = const { RefCell::new(None) };
}

/// Show the Unix Shells sign-in dialog.
pub fn show_unixshells_dialog(parent: HWND, device_service: Arc<DeviceService>) {
    DIALOG_DS.with(|ds| {
        *ds.borrow_mut() = Some(device_service);
    });

    let template = build_dialog_template();
    unsafe {
        DialogBoxIndirectParamW(
            ptr::null_mut(),
            template.as_ptr() as *const DLGTEMPLATE,
            parent,
            Some(dialog_proc),
            0,
        );
    }

    DIALOG_DS.with(|ds| {
        *ds.borrow_mut() = None;
    });
}

fn build_dialog_template() -> Vec<u8> {
    let mut template = Vec::new();
    let width: i16 = 250;
    let height: i16 = 120;
    let style = DS_MODALFRAME | DS_CENTER | WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_SETFONT;
    let ex_style = 0u32;
    let c_dit = 0u16;

    template.extend_from_slice(&style.to_le_bytes());
    template.extend_from_slice(&ex_style.to_le_bytes());
    template.extend_from_slice(&c_dit.to_le_bytes());
    template.extend_from_slice(&0i16.to_le_bytes()); // x
    template.extend_from_slice(&0i16.to_le_bytes()); // y
    template.extend_from_slice(&width.to_le_bytes());
    template.extend_from_slice(&height.to_le_bytes());
    template.extend_from_slice(&[0u8, 0]); // menu
    template.extend_from_slice(&[0u8, 0]); // class

    let title = to_wide("Unix Shells - Sign In");
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

fn align_to_word(v: &mut Vec<u8>) {
    while v.len() % 2 != 0 {
        v.push(0);
    }
}

unsafe extern "system" fn dialog_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    _lparam: LPARAM,
) -> INT_PTR {
    match msg {
        WM_INITDIALOG => {
            init_dialog(hwnd);
            1
        }
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as i32;
            match id {
                IDC_SIGNIN => {
                    do_sign_in(hwnd);
                }
                IDCANCEL => {
                    EndDialog(hwnd, IDCANCEL as isize);
                }
                _ => {}
            }
            1
        }
        WM_TIMER => {
            poll_login_state(hwnd);
            1
        }
        WM_CLOSE => {
            EndDialog(hwnd, IDCANCEL as isize);
            1
        }
        _ => 0,
    }
}

unsafe fn init_dialog(hwnd: HWND) {
    let mut rect = std::mem::zeroed();
    GetClientRect(hwnd, &mut rect);
    let w = rect.right - rect.left;
    let margin = 15;
    let label_y = margin;
    let field_y = label_y + 20;
    let status_y = field_y + 30;
    let btn_y = status_y + 25;

    create_label(hwnd, -1, "Username:", margin, label_y, 80, 18);
    create_edit(hwnd, IDC_USERNAME, margin, field_y, w - margin * 2, 22);
    create_label(hwnd, IDC_STATUS, "", margin, status_y, w - margin * 2, 18);
    create_button(hwnd, IDC_SIGNIN, "Sign In", w - margin - 80, btn_y, 80, 25);
    create_button(hwnd, IDCANCEL, "Cancel", w - margin - 170, btn_y, 80, 25);
}

unsafe fn do_sign_in(hwnd: HWND) {
    let username_hwnd = GetDlgItem(hwnd, IDC_USERNAME);
    let username = get_edit_text(username_hwnd).trim().to_lowercase();
    if username.is_empty() {
        return;
    }

    // Disable input
    EnableWindow(username_hwnd, 0);
    EnableWindow(GetDlgItem(hwnd, IDC_SIGNIN), 0);

    let status_hwnd = GetDlgItem(hwnd, IDC_STATUS);
    set_edit_text(status_hwnd, "Check your email to approve...");

    // Start login in background
    DIALOG_DS.with(|ds| {
        if let Some(ref service) = *ds.borrow() {
            let ds = service.clone();
            let username = username.clone();
            std::thread::spawn(move || {
                if let Err(e) = ds.start_login(&username) {
                    log::error!("Unix Shells login failed: {}", e);
                }
            });
        }
    });

    // Start polling timer (500ms)
    SetTimer(hwnd, 1, 500, None);
}

unsafe fn poll_login_state(hwnd: HWND) {
    DIALOG_DS.with(|ds| {
        if let Some(ref service) = *ds.borrow() {
            match service.login_state() {
                cterm_app::unixshells::LoginState::LoggedIn { ref username } => {
                    KillTimer(hwnd, 1);
                    let status_hwnd = GetDlgItem(hwnd, IDC_STATUS);
                    set_edit_text(status_hwnd, &format!("Signed in as {}", username));
                    // Close after brief delay so user sees the message
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    EndDialog(hwnd, IDOK as isize);
                }
                cterm_app::unixshells::LoginState::LoggedOut => {
                    if let Some(err) = service.last_error() {
                        KillTimer(hwnd, 1);
                        let status_hwnd = GetDlgItem(hwnd, IDC_STATUS);
                        set_edit_text(status_hwnd, &err);
                        EnableWindow(GetDlgItem(hwnd, IDC_USERNAME), 1);
                        EnableWindow(GetDlgItem(hwnd, IDC_SIGNIN), 1);
                    }
                }
                _ => {} // Still pending
            }
        }
    });
}
