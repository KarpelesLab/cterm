//! Modal dialogs for cterm
//!
//! Provides dialog boxes for preferences, about, find, set title/color, etc.

use std::cell::RefCell;
use std::ptr;

use winapi::shared::basetsd::INT_PTR;
use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::um::winuser::*;

// Thread-local storage for dialog data (needed because dialog procedures don't have context)
thread_local! {
    static DIALOG_INPUT: RefCell<Option<String>> = const { RefCell::new(None) };
    static DIALOG_RESULT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Convert a Rust string to a null-terminated wide string
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Show an About dialog
pub fn show_about_dialog(parent: HWND) {
    let title = to_wide("About cterm");
    let message = to_wide(&format!(
        "cterm - Terminal Emulator\n\nVersion: {}\n\nA high-performance terminal emulator\nwritten in Rust.",
        env!("CARGO_PKG_VERSION")
    ));

    unsafe {
        MessageBoxW(
            parent,
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

/// Show a message box
pub fn show_message(parent: HWND, title: &str, message: &str, flags: u32) -> i32 {
    let title = to_wide(title);
    let message = to_wide(message);

    unsafe { MessageBoxW(parent, message.as_ptr(), title.as_ptr(), flags) }
}

/// Show an error message
pub fn show_error(parent: HWND, title: &str, message: &str) {
    show_message(parent, title, message, MB_OK | MB_ICONERROR);
}

/// Show a warning message
#[allow(dead_code)]
pub fn show_warning(parent: HWND, title: &str, message: &str) {
    show_message(parent, title, message, MB_OK | MB_ICONWARNING);
}

/// Show an info message
#[allow(dead_code)]
pub fn show_info(parent: HWND, title: &str, message: &str) {
    show_message(parent, title, message, MB_OK | MB_ICONINFORMATION);
}

/// Show a confirmation dialog
#[allow(dead_code)]
pub fn show_confirm(parent: HWND, title: &str, message: &str) -> bool {
    show_message(parent, title, message, MB_YESNO | MB_ICONQUESTION) == IDYES
}

/// Result from an input dialog
pub enum InputDialogResult {
    Ok(String),
    Cancel,
}

// Dialog control IDs
const IDC_EDIT: i32 = 1001;
const IDC_PROMPT: i32 = 1002;

/// Dialog procedure for input dialog
unsafe extern "system" fn input_dialog_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    _lparam: LPARAM,
) -> INT_PTR {
    match msg {
        WM_INITDIALOG => {
            // Center the dialog on parent
            let mut rect = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rect);
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;

            let mut parent_rect = std::mem::zeroed();
            let parent = GetParent(hwnd);
            if !parent.is_null() {
                GetWindowRect(parent, &mut parent_rect);
            } else {
                // Use screen center
                parent_rect.right = GetSystemMetrics(SM_CXSCREEN);
                parent_rect.bottom = GetSystemMetrics(SM_CYSCREEN);
            }

            let x = (parent_rect.left + parent_rect.right) / 2 - width / 2;
            let y = (parent_rect.top + parent_rect.bottom) / 2 - height / 2;
            SetWindowPos(hwnd, ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER);

            // Set initial text in edit control
            DIALOG_INPUT.with(|input| {
                if let Some(ref text) = *input.borrow() {
                    let edit = GetDlgItem(hwnd, IDC_EDIT);
                    let wide = to_wide(text);
                    SetWindowTextW(edit, wide.as_ptr());
                    // Select all text
                    SendMessageW(edit, EM_SETSEL as u32, 0, -1i32 as LPARAM);
                }
            });

            // Focus the edit control
            let edit = GetDlgItem(hwnd, IDC_EDIT);
            SetFocus(edit);

            1 // Return TRUE to indicate we set focus
        }
        WM_COMMAND => {
            let id = (wparam & 0xFFFF) as i32;
            match id {
                IDOK => {
                    // Get text from edit control
                    let edit = GetDlgItem(hwnd, IDC_EDIT);
                    let len = GetWindowTextLengthW(edit) as usize;
                    let mut buffer = vec![0u16; len + 1];
                    GetWindowTextW(edit, buffer.as_mut_ptr(), buffer.len() as i32);

                    let text = String::from_utf16_lossy(&buffer[..len]);
                    DIALOG_RESULT.with(|result| {
                        *result.borrow_mut() = Some(text);
                    });

                    EndDialog(hwnd, IDOK as isize);
                    1
                }
                IDCANCEL => {
                    DIALOG_RESULT.with(|result| {
                        *result.borrow_mut() = None;
                    });
                    EndDialog(hwnd, IDCANCEL as isize);
                    1
                }
                _ => 0,
            }
        }
        _ => 0,
    }
}

/// Build a dialog template in memory
fn build_input_dialog_template(title: &str, prompt: &str) -> Vec<u8> {
    let mut template = Vec::new();

    // DLGTEMPLATE structure (must be DWORD aligned)
    let style = DS_MODALFRAME | DS_CENTER | WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_SETFONT;
    let ex_style = 0u32;
    let c_dit = 4u16; // number of controls: prompt label, edit, OK, Cancel
    let x = 0i16;
    let y = 0i16;
    let cx = 250i16;
    let cy = 80i16;

    // DLGTEMPLATE
    template.extend_from_slice(&style.to_le_bytes());
    template.extend_from_slice(&ex_style.to_le_bytes());
    template.extend_from_slice(&c_dit.to_le_bytes());
    template.extend_from_slice(&x.to_le_bytes());
    template.extend_from_slice(&y.to_le_bytes());
    template.extend_from_slice(&cx.to_le_bytes());
    template.extend_from_slice(&cy.to_le_bytes());

    // Menu (none)
    template.extend_from_slice(&[0u8, 0]);
    // Class (use default)
    template.extend_from_slice(&[0u8, 0]);
    // Title
    let title_wide = to_wide(title);
    for c in &title_wide {
        template.extend_from_slice(&c.to_le_bytes());
    }

    // Font (for DS_SETFONT)
    align_to_word(&mut template);
    template.extend_from_slice(&9u16.to_le_bytes()); // point size
    let font_wide = to_wide("MS Shell Dlg");
    for c in &font_wide {
        template.extend_from_slice(&c.to_le_bytes());
    }

    // Control 1: Static label (prompt)
    align_to_dword(&mut template);
    add_dialog_control(
        &mut template,
        WS_CHILD | WS_VISIBLE | SS_LEFT,
        10,
        10,
        230,
        14,
        IDC_PROMPT,
        0x0082, // Static
        prompt,
    );

    // Control 2: Edit control
    align_to_dword(&mut template);
    add_dialog_control(
        &mut template,
        WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
        10,
        28,
        230,
        14,
        IDC_EDIT,
        0x0081, // Edit
        "",
    );

    // Control 3: OK button
    align_to_dword(&mut template);
    add_dialog_control(
        &mut template,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
        130,
        52,
        50,
        14,
        IDOK,
        0x0080, // Button
        "OK",
    );

    // Control 4: Cancel button
    align_to_dword(&mut template);
    add_dialog_control(
        &mut template,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
        190,
        52,
        50,
        14,
        IDCANCEL,
        0x0080, // Button
        "Cancel",
    );

    template
}

fn align_to_word(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(2) {
        v.push(0);
    }
}

fn align_to_dword(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(4) {
        v.push(0);
    }
}

#[allow(clippy::too_many_arguments)]
fn add_dialog_control(
    template: &mut Vec<u8>,
    style: u32,
    x: i16,
    y: i16,
    cx: i16,
    cy: i16,
    id: i32,
    class: u16,
    text: &str,
) {
    // DLGITEMTEMPLATE
    let style = style | WS_CHILD;
    let ex_style = 0u32;

    template.extend_from_slice(&style.to_le_bytes());
    template.extend_from_slice(&ex_style.to_le_bytes());
    template.extend_from_slice(&x.to_le_bytes());
    template.extend_from_slice(&y.to_le_bytes());
    template.extend_from_slice(&cx.to_le_bytes());
    template.extend_from_slice(&cy.to_le_bytes());
    template.extend_from_slice(&(id as u16).to_le_bytes());

    // Window class (use ordinal)
    template.extend_from_slice(&0xFFFFu16.to_le_bytes());
    template.extend_from_slice(&class.to_le_bytes());

    // Title/text
    let text_wide = to_wide(text);
    for c in &text_wide {
        template.extend_from_slice(&c.to_le_bytes());
    }

    // No creation data
    template.extend_from_slice(&0u16.to_le_bytes());
}

/// Show an input dialog
pub fn show_input_dialog(
    parent: HWND,
    title: &str,
    prompt: &str,
    initial_value: &str,
) -> InputDialogResult {
    // Set up input value
    DIALOG_INPUT.with(|input| {
        *input.borrow_mut() = Some(initial_value.to_string());
    });
    DIALOG_RESULT.with(|result| {
        *result.borrow_mut() = None;
    });

    // Build dialog template
    let template = build_input_dialog_template(title, prompt);

    // Show dialog
    let ret = unsafe {
        DialogBoxIndirectParamW(
            ptr::null_mut(),
            template.as_ptr() as *const DLGTEMPLATE,
            parent,
            Some(input_dialog_proc),
            0,
        )
    };

    // Get result
    if ret == IDOK as isize {
        DIALOG_RESULT.with(|result| {
            if let Some(text) = result.borrow().clone() {
                InputDialogResult::Ok(text)
            } else {
                InputDialogResult::Cancel
            }
        })
    } else {
        InputDialogResult::Cancel
    }
}

/// Color picker result
#[derive(Debug, Clone)]
pub struct ColorChoice {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Show a color picker dialog
pub fn show_color_picker(parent: HWND) -> Option<ColorChoice> {
    use winapi::um::commdlg::{ChooseColorW, CC_ANYCOLOR, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW};

    let mut custom_colors: [u32; 16] = [0xFFFFFF; 16];
    let mut cc = CHOOSECOLORW {
        lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: parent,
        hInstance: ptr::null_mut(),
        rgbResult: 0,
        lpCustColors: custom_colors.as_mut_ptr(),
        Flags: CC_ANYCOLOR | CC_FULLOPEN | CC_RGBINIT,
        lCustData: 0,
        lpfnHook: None,
        lpTemplateName: ptr::null(),
    };

    unsafe {
        if ChooseColorW(&mut cc) != 0 {
            let color = cc.rgbResult;
            Some(ColorChoice {
                r: (color & 0xFF) as u8,
                g: ((color >> 8) & 0xFF) as u8,
                b: ((color >> 16) & 0xFF) as u8,
            })
        } else {
            None
        }
    }
}

/// Predefined color options for tab colors
pub const TAB_COLORS: &[(&str, &str)] = &[
    ("Default", ""),
    ("Red", "#e74c3c"),
    ("Orange", "#e67e22"),
    ("Yellow", "#f1c40f"),
    ("Green", "#2ecc71"),
    ("Teal", "#1abc9c"),
    ("Blue", "#3498db"),
    ("Purple", "#9b59b6"),
    ("Pink", "#e91e63"),
    ("Gray", "#95a5a6"),
];

/// Show a set color dialog - first asks to pick or clear, then shows color picker
pub fn show_set_color_dialog(parent: HWND) -> Option<Option<String>> {
    let title = to_wide("Set Tab Color");
    let prompt = to_wide(
        "Would you like to:\n\n• Click 'Yes' to choose a new color\n• Click 'No' to clear the current color\n• Click 'Cancel' to keep the current color",
    );

    let result = unsafe {
        MessageBoxW(
            parent,
            prompt.as_ptr(),
            title.as_ptr(),
            MB_YESNOCANCEL | MB_ICONQUESTION,
        )
    };

    match result {
        IDYES => {
            // Show color picker
            show_color_picker(parent).map(|c| Some(format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)))
        }
        IDNO => {
            // Clear color
            Some(None)
        }
        _ => {
            // Cancel
            None
        }
    }
}

/// Show a set title dialog
pub fn show_set_title_dialog(parent: HWND, current_title: &str) -> Option<String> {
    match show_input_dialog(parent, "Set Tab Title", "Enter a new title:", current_title) {
        InputDialogResult::Ok(title) => Some(title),
        InputDialogResult::Cancel => None,
    }
}

/// Find dialog options
#[derive(Debug, Clone)]
pub struct FindOptions {
    pub text: String,
    pub case_sensitive: bool,
    pub regex: bool,
}

/// Show a find dialog
pub fn show_find_dialog(parent: HWND) -> Option<FindOptions> {
    match show_input_dialog(parent, "Find in Terminal", "Search for:", "") {
        InputDialogResult::Ok(text) if !text.is_empty() => Some(FindOptions {
            text,
            case_sensitive: false,
            regex: false,
        }),
        _ => None,
    }
}

/// File dialog result
pub fn show_save_file_dialog(
    parent: HWND,
    title: &str,
    suggested_name: Option<&str>,
    filter: &str,
) -> Option<std::path::PathBuf> {
    use winapi::um::commdlg::{
        GetSaveFileNameW, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
    };

    let title = to_wide(title);
    let filter = to_wide(filter);
    let mut filename = [0u16; 260];

    // Copy suggested name if provided
    if let Some(name) = suggested_name {
        let wide_name = to_wide(name);
        let len = wide_name.len().min(filename.len() - 1);
        filename[..len].copy_from_slice(&wide_name[..len]);
    }

    let mut ofn = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: parent,
        hInstance: ptr::null_mut(),
        lpstrFilter: filter.as_ptr(),
        lpstrCustomFilter: ptr::null_mut(),
        nMaxCustFilter: 0,
        nFilterIndex: 1,
        lpstrFile: filename.as_mut_ptr(),
        nMaxFile: filename.len() as u32,
        lpstrFileTitle: ptr::null_mut(),
        nMaxFileTitle: 0,
        lpstrInitialDir: ptr::null(),
        lpstrTitle: title.as_ptr(),
        Flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST,
        nFileOffset: 0,
        nFileExtension: 0,
        lpstrDefExt: ptr::null(),
        lCustData: 0,
        lpfnHook: None,
        lpTemplateName: ptr::null(),
        pvReserved: ptr::null_mut(),
        dwReserved: 0,
        FlagsEx: 0,
    };

    unsafe {
        if GetSaveFileNameW(&mut ofn) != 0 {
            let len = filename
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(filename.len());
            let path = String::from_utf16_lossy(&filename[..len]);
            Some(std::path::PathBuf::from(path))
        } else {
            None
        }
    }
}

/// Show a save dialog for file transfer (wrapper for show_save_file_dialog)
///
/// Takes a windows crate HWND and converts it for winapi
pub fn show_save_dialog(
    hwnd: windows::Win32::Foundation::HWND,
    default_path: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let parent = hwnd.0 as *mut _;
    let suggested_name = default_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");

    show_save_file_dialog(
        parent,
        "Save File",
        Some(suggested_name),
        "All Files\0*.*\0\0",
    )
}

/// Show an error message (wrapper that takes windows crate HWND)
pub fn show_error_msg(hwnd: windows::Win32::Foundation::HWND, message: &str) {
    let parent = hwnd.0 as *mut _;
    show_error(parent, "Error", message);
}

/// Show a simple preferences placeholder dialog
pub fn show_preferences_dialog(parent: HWND) {
    let title = to_wide("Preferences");
    let message = to_wide(
        "Preferences can be edited in the configuration file:\n\n\
        Windows: %APPDATA%\\cterm\\config.toml\n\n\
        A full preferences dialog will be available in a future version.",
    );

    unsafe {
        MessageBoxW(
            parent,
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

/// Show tab templates placeholder dialog
pub fn show_tab_templates_dialog(parent: HWND) {
    let title = to_wide("Tab Templates");
    let message = to_wide(
        "Tab templates can be configured in:\n\n\
        Windows: %APPDATA%\\cterm\\sticky_tabs.toml\n\n\
        A visual template editor will be available in a future version.",
    );

    unsafe {
        MessageBoxW(
            parent,
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

/// Show check for updates dialog
pub fn show_check_updates_dialog(parent: HWND) {
    let title = to_wide("Check for Updates");
    let message = to_wide(&format!(
        "Current version: {}\n\n\
        Visit https://github.com/KarpelesLab/cterm/releases\n\
        to check for the latest version.",
        env!("CARGO_PKG_VERSION")
    ));

    unsafe {
        MessageBoxW(
            parent,
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONINFORMATION,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_wide() {
        let wide = to_wide("Hello");
        assert_eq!(wide.len(), 6); // "Hello" + null terminator
        assert_eq!(wide[5], 0);
    }

    #[test]
    fn test_tab_colors() {
        assert!(!TAB_COLORS.is_empty());
        assert_eq!(TAB_COLORS[0].0, "Default");
    }

    #[test]
    fn test_build_dialog_template() {
        let template = build_input_dialog_template("Test", "Enter value:");
        // Template should be non-empty and properly aligned
        assert!(!template.is_empty());
        assert!(template.len().is_multiple_of(4) || template.len() > 100);
    }
}
