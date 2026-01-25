//! Log viewer window for cterm
//!
//! Provides an in-app log viewer window with refresh capability.

use std::ptr;

use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::um::winbase::MulDiv;
use winapi::um::wingdi::{CreateFontW, GetDeviceCaps, FW_NORMAL, LOGPIXELSY};
use winapi::um::winuser::*;

use crate::dialog_utils::{create_button, set_edit_text, to_wide};

/// Window class name for log viewer
const LOG_VIEWER_CLASS: &str = "cterm_log_viewer";

// Control IDs
const IDC_LOG_EDIT: i32 = 1001;
const IDC_REFRESH_BTN: i32 = 1002;
const IDC_CLOSE_BTN: i32 = 1003;

/// Register the log viewer window class
pub fn register_window_class() -> bool {
    let class_name = to_wide(LOG_VIEWER_CLASS);

    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: ptr::null_mut(),
        hIcon: ptr::null_mut(),
        hCursor: unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) },
        hbrBackground: (COLOR_WINDOW + 1) as *mut _,
        lpszMenuName: ptr::null(),
        lpszClassName: class_name.as_ptr(),
        hIconSm: ptr::null_mut(),
    };

    unsafe { RegisterClassExW(&wc) != 0 }
}

/// Show the log viewer window
pub fn show_log_viewer(parent: HWND) {
    // Register window class (only once)
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let _ = REGISTERED.get_or_init(register_window_class);

    // Window dimensions
    let width = 600;
    let height = 450;

    // Center on parent
    let (x, y) = if !parent.is_null() {
        let mut rect = unsafe { std::mem::zeroed() };
        unsafe { GetWindowRect(parent, &mut rect) };
        let parent_width = rect.right - rect.left;
        let parent_height = rect.bottom - rect.top;
        (
            rect.left + (parent_width - width) / 2,
            rect.top + (parent_height - height) / 2,
        )
    } else {
        // Center on screen
        let screen_width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
        let screen_height = unsafe { GetSystemMetrics(SM_CYSCREEN) };
        ((screen_width - width) / 2, (screen_height - height) / 2)
    };

    let class_name = to_wide(LOG_VIEWER_CLASS);
    let title = to_wide("Debug Log");

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME,
            x,
            y,
            width,
            height,
            parent,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };

    if !hwnd.is_null() {
        unsafe {
            ShowWindow(hwnd, SW_SHOW);
            UpdateWindow(hwnd);
        }
    }
}

/// Window procedure for log viewer
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            init_window(hwnd);
            0
        }
        WM_SIZE => {
            handle_resize(hwnd);
            0
        }
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as i32;
            match id {
                IDC_REFRESH_BTN => {
                    refresh_logs(hwnd);
                }
                IDC_CLOSE_BTN => {
                    DestroyWindow(hwnd);
                }
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Initialize the window controls
unsafe fn init_window(hwnd: HWND) {
    // Get window client area
    let mut rect = std::mem::zeroed();
    GetClientRect(hwnd, &mut rect);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;

    let button_height = 25;
    let button_width = 80;
    let margin = 10;
    let edit_height = height - button_height - margin * 3;

    // Create multiline edit control for log display
    let edit_class = to_wide("EDIT");
    let edit = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        edit_class.as_ptr(),
        ptr::null(),
        WS_CHILD
            | WS_VISIBLE
            | WS_VSCROLL
            | WS_HSCROLL
            | ES_MULTILINE as u32
            | ES_AUTOVSCROLL as u32
            | ES_AUTOHSCROLL as u32
            | ES_READONLY as u32,
        margin,
        margin,
        width - margin * 2,
        edit_height,
        hwnd,
        IDC_LOG_EDIT as *mut _,
        ptr::null_mut(),
        ptr::null_mut(),
    );

    // Create monospace font
    let hdc = GetDC(hwnd);
    let log_pixels_y = GetDeviceCaps(hdc, LOGPIXELSY);
    ReleaseDC(hwnd, hdc);

    let font_name = to_wide("Consolas");
    let font_height = -MulDiv(10, log_pixels_y, 72); // 10pt font
    let font = CreateFontW(
        font_height,
        0,
        0,
        0,
        FW_NORMAL as i32,
        0,
        0,
        0,
        0, // DEFAULT_CHARSET
        0,
        0,
        0,
        1, // FIXED_PITCH
        font_name.as_ptr(),
    );

    if !font.is_null() {
        SendMessageW(edit, WM_SETFONT, font as WPARAM, 1);
    }

    // Create Refresh button
    let _refresh_btn = create_button(
        hwnd,
        IDC_REFRESH_BTN,
        "Refresh",
        width - margin - button_width * 2 - margin,
        height - margin - button_height,
        button_width,
        button_height,
    );

    // Create Close button
    let _close_btn = create_button(
        hwnd,
        IDC_CLOSE_BTN,
        "Close",
        width - margin - button_width,
        height - margin - button_height,
        button_width,
        button_height,
    );

    // Load initial logs
    refresh_logs(hwnd);
}

/// Handle window resize
unsafe fn handle_resize(hwnd: HWND) {
    let mut rect = std::mem::zeroed();
    GetClientRect(hwnd, &mut rect);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;

    let button_height = 25;
    let button_width = 80;
    let margin = 10;
    let edit_height = height - button_height - margin * 3;

    // Resize edit control
    let edit = GetDlgItem(hwnd, IDC_LOG_EDIT);
    if !edit.is_null() {
        SetWindowPos(
            edit,
            ptr::null_mut(),
            margin,
            margin,
            width - margin * 2,
            edit_height,
            SWP_NOZORDER,
        );
    }

    // Reposition buttons
    let refresh_btn = GetDlgItem(hwnd, IDC_REFRESH_BTN);
    if !refresh_btn.is_null() {
        SetWindowPos(
            refresh_btn,
            ptr::null_mut(),
            width - margin - button_width * 2 - margin,
            height - margin - button_height,
            button_width,
            button_height,
            SWP_NOZORDER,
        );
    }

    let close_btn = GetDlgItem(hwnd, IDC_CLOSE_BTN);
    if !close_btn.is_null() {
        SetWindowPos(
            close_btn,
            ptr::null_mut(),
            width - margin - button_width,
            height - margin - button_height,
            button_width,
            button_height,
            SWP_NOZORDER,
        );
    }
}

/// Refresh the log display
unsafe fn refresh_logs(hwnd: HWND) {
    let edit = GetDlgItem(hwnd, IDC_LOG_EDIT);
    if edit.is_null() {
        return;
    }

    // Get formatted logs
    let logs = cterm_app::log_capture::get_logs_formatted();

    // Convert newlines to CRLF for Windows edit control
    let logs = logs.replace('\n', "\r\n");

    // Set text
    set_edit_text(edit, &logs);

    // Scroll to bottom
    let line_count = SendMessageW(edit, EM_GETLINECOUNT as u32, 0, 0);
    SendMessageW(edit, EM_LINESCROLL as u32, 0, line_count as LPARAM);
}
