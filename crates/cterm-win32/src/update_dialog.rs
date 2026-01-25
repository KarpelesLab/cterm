//! Update dialog for cterm
//!
//! Provides a dialog window that checks for updates from GitHub releases
//! and displays release notes.

use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::um::winbase::MulDiv;
use winapi::um::wingdi::{CreateFontW, GetDeviceCaps, FW_NORMAL, LOGPIXELSY};
use winapi::um::winuser::*;

use crate::dialog_utils::{create_button, create_label, set_edit_text, to_wide};

/// Window class name for update dialog
const UPDATE_DIALOG_CLASS: &str = "cterm_update_dialog";

/// Custom message for update check result
const WM_UPDATE_RESULT: UINT = WM_USER + 100;

// Control IDs
const IDC_CURRENT_VERSION_LABEL: i32 = 1001;
const IDC_NEW_VERSION_LABEL: i32 = 1002;
const IDC_RELEASE_NOTES_EDIT: i32 = 1003;
const IDC_OPEN_RELEASES_BTN: i32 = 1004;
const IDC_CLOSE_BTN: i32 = 1005;
const IDC_STATUS_LABEL: i32 = 1006;

/// GitHub repository for update checks
const GITHUB_REPO: &str = "KarpelesLab/cterm";

/// Update check result passed via window message
enum UpdateResult {
    Checking,
    NoUpdate,
    UpdateAvailable(Box<cterm_app::upgrade::UpdateInfo>),
    Error(String),
}

// Thread-local storage for update result
thread_local! {
    static PENDING_UPDATE: std::cell::RefCell<Option<UpdateResult>> = const { std::cell::RefCell::new(None) };
}

/// Cancellation flag for update check
static UPDATE_CHECK_CANCELLED: AtomicBool = AtomicBool::new(false);

/// Register the update dialog window class
pub fn register_window_class() -> bool {
    let class_name = to_wide(UPDATE_DIALOG_CLASS);

    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: ptr::null_mut(),
        hIcon: ptr::null_mut(),
        hCursor: unsafe { LoadCursorW(ptr::null_mut(), IDC_ARROW) },
        hbrBackground: (COLOR_BTNFACE + 1) as *mut _,
        lpszMenuName: ptr::null(),
        lpszClassName: class_name.as_ptr(),
        hIconSm: ptr::null_mut(),
    };

    unsafe { RegisterClassExW(&wc) != 0 }
}

/// Show the update dialog
pub fn show_update_dialog(parent: HWND) {
    // Register window class (only once)
    static REGISTERED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let _ = REGISTERED.get_or_init(register_window_class);

    // Reset cancellation flag
    UPDATE_CHECK_CANCELLED.store(false, Ordering::SeqCst);

    // Window dimensions
    let width = 500;
    let height = 400;

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

    let class_name = to_wide(UPDATE_DIALOG_CLASS);
    let title = to_wide("Check for Updates");

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
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

/// Window procedure for update dialog
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            init_window(hwnd);
            // Start checking for updates
            start_update_check(hwnd);
            0
        }
        WM_UPDATE_RESULT => {
            handle_update_result(hwnd);
            0
        }
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as i32;
            match id {
                IDC_OPEN_RELEASES_BTN => {
                    // Open GitHub releases page
                    let url = to_wide(&format!("https://github.com/{}/releases", GITHUB_REPO));
                    winapi::um::shellapi::ShellExecuteW(
                        ptr::null_mut(),
                        to_wide("open").as_ptr(),
                        url.as_ptr(),
                        ptr::null(),
                        ptr::null(),
                        SW_SHOWNORMAL,
                    );
                }
                IDC_CLOSE_BTN => {
                    UPDATE_CHECK_CANCELLED.store(true, Ordering::SeqCst);
                    DestroyWindow(hwnd);
                }
                _ => {}
            }
            0
        }
        WM_CLOSE => {
            UPDATE_CHECK_CANCELLED.store(true, Ordering::SeqCst);
            DestroyWindow(hwnd);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Initialize the window controls
unsafe fn init_window(hwnd: HWND) {
    let margin = 15;
    let label_height = 20;
    let button_height = 25;
    let button_width = 120;

    // Get window client area
    let mut rect = std::mem::zeroed();
    GetClientRect(hwnd, &mut rect);
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;

    let mut y = margin;

    // Current version label
    let current_version = env!("CARGO_PKG_VERSION");
    create_label(
        hwnd,
        IDC_CURRENT_VERSION_LABEL,
        &format!("Current version: {}", current_version),
        margin,
        y,
        width - margin * 2,
        label_height,
    );
    y += label_height + 5;

    // New version / status label
    let _status_label = create_label(
        hwnd,
        IDC_STATUS_LABEL,
        "Checking for updates...",
        margin,
        y,
        width - margin * 2,
        label_height,
    );
    y += label_height + 10;

    // Release notes label
    create_label(
        hwnd,
        0,
        "Release Notes:",
        margin,
        y,
        width - margin * 2,
        label_height,
    );
    y += label_height + 5;

    // Release notes text area
    let notes_height = height - y - button_height - margin * 2;
    let edit_class = to_wide("EDIT");
    let edit = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        edit_class.as_ptr(),
        ptr::null(),
        WS_CHILD
            | WS_VISIBLE
            | WS_VSCROLL
            | ES_MULTILINE as u32
            | ES_AUTOVSCROLL as u32
            | ES_READONLY as u32,
        margin,
        y,
        width - margin * 2,
        notes_height,
        hwnd,
        IDC_RELEASE_NOTES_EDIT as *mut _,
        ptr::null_mut(),
        ptr::null_mut(),
    );

    // Set monospace font for release notes
    let hdc = GetDC(hwnd);
    let log_pixels_y = GetDeviceCaps(hdc, LOGPIXELSY);
    ReleaseDC(hwnd, hdc);

    let font_name = to_wide("Consolas");
    let font_height = -MulDiv(9, log_pixels_y, 72);
    let font = CreateFontW(
        font_height,
        0,
        0,
        0,
        FW_NORMAL as i32,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        1, // FIXED_PITCH
        font_name.as_ptr(),
    );

    if !font.is_null() {
        SendMessageW(edit, WM_SETFONT, font as WPARAM, 1);
    }

    y += notes_height + margin;

    // Open Releases button
    create_button(
        hwnd,
        IDC_OPEN_RELEASES_BTN,
        "Open Releases",
        margin,
        y,
        button_width,
        button_height,
    );

    // Close button
    create_button(
        hwnd,
        IDC_CLOSE_BTN,
        "Close",
        width - margin - button_width,
        y,
        button_width,
        button_height,
    );
}

/// Start the async update check
unsafe fn start_update_check(hwnd: HWND) {
    let hwnd_ptr = hwnd as usize;

    std::thread::spawn(move || {
        // Create a runtime for async operation
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                PENDING_UPDATE.with(|p| {
                    *p.borrow_mut() = Some(UpdateResult::Error(format!(
                        "Failed to create runtime: {}",
                        e
                    )));
                });
                post_update_result(hwnd_ptr);
                return;
            }
        };

        rt.block_on(async {
            // Check for cancellation
            if UPDATE_CHECK_CANCELLED.load(Ordering::SeqCst) {
                return;
            }

            let current_version = env!("CARGO_PKG_VERSION");
            let updater = match cterm_app::upgrade::Updater::new(GITHUB_REPO, current_version) {
                Ok(u) => u,
                Err(e) => {
                    PENDING_UPDATE.with(|p| {
                        *p.borrow_mut() = Some(UpdateResult::Error(e.to_string()));
                    });
                    post_update_result(hwnd_ptr);
                    return;
                }
            };

            // Check for cancellation
            if UPDATE_CHECK_CANCELLED.load(Ordering::SeqCst) {
                return;
            }

            let result = match updater.check_for_update().await {
                Ok(Some(info)) => UpdateResult::UpdateAvailable(Box::new(info)),
                Ok(None) => UpdateResult::NoUpdate,
                Err(e) => UpdateResult::Error(e.to_string()),
            };

            // Check for cancellation before posting
            if UPDATE_CHECK_CANCELLED.load(Ordering::SeqCst) {
                return;
            }

            PENDING_UPDATE.with(|p| {
                *p.borrow_mut() = Some(result);
            });

            post_update_result(hwnd_ptr);
        });
    });
}

/// Post the update result message to the window
fn post_update_result(hwnd_ptr: usize) {
    unsafe {
        PostMessageW(hwnd_ptr as HWND, WM_UPDATE_RESULT, 0, 0);
    }
}

/// Handle the update result message
unsafe fn handle_update_result(hwnd: HWND) {
    let result = PENDING_UPDATE.with(|p| p.borrow_mut().take());

    let Some(result) = result else {
        return;
    };

    let status_label = GetDlgItem(hwnd, IDC_STATUS_LABEL);
    let notes_edit = GetDlgItem(hwnd, IDC_RELEASE_NOTES_EDIT);

    match result {
        UpdateResult::Checking => {
            // Should not happen, but handle gracefully
            set_label_text(status_label, "Checking for updates...");
        }
        UpdateResult::NoUpdate => {
            set_label_text(status_label, "You are running the latest version!");
            set_edit_text(notes_edit, "No updates available.");
        }
        UpdateResult::UpdateAvailable(info) => {
            set_label_text(
                status_label,
                &format!("New version available: {} ({})", info.version, info.name),
            );
            // Convert newlines to CRLF for Windows edit control
            let notes = info.release_notes.replace('\n', "\r\n");
            set_edit_text(notes_edit, &notes);
        }
        UpdateResult::Error(msg) => {
            set_label_text(status_label, &format!("Error checking for updates"));
            set_edit_text(
                notes_edit,
                &format!("Failed to check for updates:\r\n\r\n{}", msg),
            );
        }
    }
}

/// Set text on a static label control
unsafe fn set_label_text(hwnd: HWND, text: &str) {
    let wide = to_wide(text);
    SetWindowTextW(hwnd, wide.as_ptr());
}
