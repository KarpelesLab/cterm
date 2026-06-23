//! Native Win32 dialogs for interactive SSH prompts (host key, password,
//! passphrase) — used both by the in-process tunnel
//! (`DaemonConnection::connect_ssh_with_prompts`) and by daemon-side SSH tabs
//! (which deliver prompts over gRPC as `SessionPromptEvent`).
//!
//! Win32 dialogs run their own modal message loop, so these are safe to call
//! from the off-main connection thread without marshaling.

use std::cell::RefCell;
use std::ptr;

use winapi::shared::basetsd::INT_PTR;
use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::um::winuser::*;

use crate::dialog_utils::*;
use cterm_proto::proto::{PromptKind, SessionPromptEvent};

thread_local! {
    static SECRET_RESULT: RefCell<Option<String>> = const { RefCell::new(None) };
    static SECRET_PROMPT: RefCell<String> = const { RefCell::new(String::new()) };
}

const IDC_SECRET_EDIT: i32 = 3001;

// Note: the remote-ctermd SSH tunnel (`connect_ssh*`) is Unix-only, so Windows
// has no in-process tunnel prompts. This module is reached only via
// `show_ssh_prompt`, which answers daemon-side SSH-tab prompts delivered over
// gRPC as `SessionPromptEvent`.

/// Show the appropriate dialog for a daemon-delivered prompt and return
/// `(accept, secret)`.
pub fn show_ssh_prompt(prompt: &SessionPromptEvent) -> (bool, Option<String>) {
    match PromptKind::try_from(prompt.kind).unwrap_or(PromptKind::Unspecified) {
        PromptKind::HostkeyUnknown | PromptKind::HostkeyChanged => {
            let kind = PromptKind::try_from(prompt.kind).unwrap_or(PromptKind::HostkeyUnknown);
            let accept = host_key_prompt(
                kind,
                &prompt.host,
                prompt.port as u16,
                &prompt.key_type,
                &prompt.fingerprint,
            );
            (accept, None)
        }
        PromptKind::Password => (false, secret_prompt("Password Required", &prompt.text)),
        PromptKind::Passphrase => (
            false,
            secret_prompt("Key Passphrase Required", &prompt.text),
        ),
        PromptKind::Unspecified => (false, None),
    }
}

fn host_key_prompt(
    kind: PromptKind,
    host: &str,
    port: u16,
    key_type: &str,
    fingerprint: &str,
) -> bool {
    let changed = kind == PromptKind::HostkeyChanged;
    let warning = if changed {
        "WARNING: the host key for this server has CHANGED since you last \
         connected. This could indicate a man-in-the-middle attack.\r\n\r\n"
    } else {
        "The authenticity of this host can't be established.\r\n\r\n"
    };
    let body = format!(
        "{warning}Host: {host}:{port}\r\nKey type: {key_type}\r\nFingerprint: {fingerprint}\r\n\r\n\
         Do you want to continue connecting and trust this key?"
    );
    let caption = if changed {
        "Host Key Has CHANGED"
    } else {
        "Unknown Host Key"
    };

    let body_w = to_wide(&body);
    let caption_w = to_wide(caption);
    let flags = MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_TOPMOST | MB_SETFOREGROUND;
    let ret = unsafe { MessageBoxW(ptr::null_mut(), body_w.as_ptr(), caption_w.as_ptr(), flags) };
    ret == IDYES
}

fn secret_prompt(title: &str, text: &str) -> Option<String> {
    SECRET_RESULT.with(|r| *r.borrow_mut() = None);
    SECRET_PROMPT.with(|r| {
        *r.borrow_mut() = if text.is_empty() {
            "Enter password:".to_string()
        } else {
            text.to_string()
        };
    });

    let template = build_secret_dialog_template(title);
    let ret = unsafe {
        DialogBoxIndirectParamW(
            ptr::null_mut(),
            template.as_ptr() as *const DLGTEMPLATE,
            ptr::null_mut(),
            Some(secret_dialog_proc),
            0,
        )
    };

    if ret == IDOK as isize {
        SECRET_RESULT.with(|r| r.borrow().clone())
    } else {
        None
    }
}

fn build_secret_dialog_template(title: &str) -> Vec<u8> {
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
    let title_w = to_wide(title);
    for c in &title_w {
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

unsafe extern "system" fn secret_dialog_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    _lparam: LPARAM,
) -> INT_PTR {
    match msg {
        WM_INITDIALOG => {
            init_secret_dialog(hwnd);
            1
        }
        WM_COMMAND => {
            handle_secret_command(hwnd, (wparam & 0xFFFF) as i32);
            1
        }
        WM_CLOSE => {
            EndDialog(hwnd, IDCANCEL as isize);
            1
        }
        _ => 0,
    }
}

unsafe fn init_secret_dialog(hwnd: HWND) {
    let mut rect = std::mem::zeroed();
    GetClientRect(hwnd, &mut rect);
    let dlg_width = rect.right - rect.left;
    let dlg_height = rect.bottom - rect.top;
    let margin = 10;
    let button_height = 25;
    let button_width = 80;

    let label = SECRET_PROMPT.with(|r| r.borrow().clone());
    create_label(hwnd, -1, &label, margin, margin, dlg_width - margin * 2, 20);

    let edit = create_edit(
        hwnd,
        IDC_SECRET_EDIT,
        margin,
        margin + 22,
        dlg_width - margin * 2,
        22,
    );
    // Mask the input like a password field.
    SendMessageW(edit, EM_SETPASSWORDCHAR, '*' as usize, 0);

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
        "OK",
        dlg_width - margin - button_width,
        btn_y,
        button_width,
        button_height,
    );
}

unsafe fn handle_secret_command(hwnd: HWND, id: i32) {
    match id {
        IDOK => {
            let edit = get_dialog_item(hwnd, IDC_SECRET_EDIT);
            let secret = get_edit_text(edit);
            SECRET_RESULT.with(|r| {
                *r.borrow_mut() = Some(secret);
            });
            EndDialog(hwnd, IDOK as isize);
        }
        IDCANCEL => {
            EndDialog(hwnd, IDCANCEL as isize);
        }
        _ => {}
    }
}

fn align_to_word(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(2) {
        v.push(0);
    }
}
