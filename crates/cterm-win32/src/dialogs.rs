//! Modal dialogs for cterm
//!
//! Provides dialog boxes for preferences, about, find, set title/color, etc.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr;

use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HWND, RECT};
use winapi::um::winuser::*;

/// Convert a Rust string to a null-terminated wide string
fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
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
pub fn show_warning(parent: HWND, title: &str, message: &str) {
    show_message(parent, title, message, MB_OK | MB_ICONWARNING);
}

/// Show an info message
pub fn show_info(parent: HWND, title: &str, message: &str) {
    show_message(parent, title, message, MB_OK | MB_ICONINFORMATION);
}

/// Show a confirmation dialog
pub fn show_confirm(parent: HWND, title: &str, message: &str) -> bool {
    show_message(parent, title, message, MB_YESNO | MB_ICONQUESTION) == IDYES
}

/// Result from an input dialog
pub enum InputDialogResult {
    Ok(String),
    Cancel,
}

/// Show an input dialog using a custom dialog template
pub fn show_input_dialog(
    parent: HWND,
    title: &str,
    prompt: &str,
    initial_value: &str,
) -> InputDialogResult {
    // For simplicity, we'll use a basic approach with InputBox-style behavior
    // In a full implementation, this would use a proper dialog template

    // Create dialog data
    let mut data = InputDialogData {
        prompt: prompt.to_string(),
        value: initial_value.to_string(),
        result: None,
    };

    // For now, return the initial value or empty
    // TODO: Implement proper dialog with edit control
    log::warn!("Input dialog not fully implemented, returning initial value");
    InputDialogResult::Ok(data.value)
}

/// Internal data for input dialog
struct InputDialogData {
    prompt: String,
    value: String,
    result: Option<String>,
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

/// Show a set color dialog with predefined colors
pub fn show_set_color_dialog(parent: HWND) -> Option<Option<String>> {
    // For simplicity, we'll show a simple list dialog
    // In a full implementation, this would be a proper dialog with color swatches

    let title = to_wide("Set Tab Color");
    let prompt =
        to_wide("Choose a color for this tab:\n\n(Feature will be enhanced in future versions)");

    let result = unsafe {
        MessageBoxW(
            parent,
            prompt.as_ptr(),
            title.as_ptr(),
            MB_OKCANCEL | MB_ICONQUESTION,
        )
    };

    if result == IDOK {
        // Show color picker
        show_color_picker(parent).map(|c| Some(format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)))
    } else {
        None
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
    // Simplified version - in full implementation, use a proper dialog
    match show_input_dialog(parent, "Find", "Search for:", "") {
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
}
