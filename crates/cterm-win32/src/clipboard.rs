//! Clipboard operations for Windows
//!
//! Handles copy and paste using the Windows clipboard API.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr;

use winapi::um::winbase::{GlobalAlloc, GlobalFree, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use winapi::um::winuser::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
    CF_UNICODETEXT,
};

/// Copy text to the clipboard
pub fn copy_to_clipboard(text: &str) -> Result<(), ClipboardError> {
    // Convert to wide string (UTF-16)
    let wide: Vec<u16> = OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0)) // null terminator
        .collect();

    let size = wide.len() * std::mem::size_of::<u16>();

    unsafe {
        // Open clipboard
        if OpenClipboard(ptr::null_mut()) == 0 {
            return Err(ClipboardError::OpenFailed);
        }

        // Empty clipboard
        if EmptyClipboard() == 0 {
            CloseClipboard();
            return Err(ClipboardError::EmptyFailed);
        }

        // Allocate global memory
        let hglobal = GlobalAlloc(GMEM_MOVEABLE, size);
        if hglobal.is_null() {
            CloseClipboard();
            return Err(ClipboardError::AllocFailed);
        }

        // Lock memory and copy data
        let ptr = GlobalLock(hglobal) as *mut u16;
        if ptr.is_null() {
            GlobalFree(hglobal);
            CloseClipboard();
            return Err(ClipboardError::LockFailed);
        }

        ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        GlobalUnlock(hglobal);

        // Set clipboard data
        if SetClipboardData(CF_UNICODETEXT, hglobal).is_null() {
            GlobalFree(hglobal);
            CloseClipboard();
            return Err(ClipboardError::SetDataFailed);
        }

        // Close clipboard (data is now owned by clipboard)
        CloseClipboard();
    }

    Ok(())
}

/// Paste text from the clipboard
pub fn paste_from_clipboard() -> Result<String, ClipboardError> {
    unsafe {
        // Open clipboard
        if OpenClipboard(ptr::null_mut()) == 0 {
            return Err(ClipboardError::OpenFailed);
        }

        // Get clipboard data
        let hglobal = GetClipboardData(CF_UNICODETEXT);
        if hglobal.is_null() {
            CloseClipboard();
            return Err(ClipboardError::NoData);
        }

        // Lock memory
        let ptr = GlobalLock(hglobal) as *const u16;
        if ptr.is_null() {
            CloseClipboard();
            return Err(ClipboardError::LockFailed);
        }

        // Find the null terminator and get the string length
        let mut len = 0;
        while *ptr.add(len) != 0 {
            len += 1;
        }

        // Convert from UTF-16 to String
        let slice = std::slice::from_raw_parts(ptr, len);
        let result = String::from_utf16_lossy(slice);

        GlobalUnlock(hglobal);
        CloseClipboard();

        Ok(result)
    }
}

/// Check if the clipboard contains text
pub fn has_text() -> bool {
    unsafe {
        if OpenClipboard(ptr::null_mut()) == 0 {
            return false;
        }

        let has = !GetClipboardData(CF_UNICODETEXT).is_null();
        CloseClipboard();

        has
    }
}

/// Clipboard errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardError {
    /// Failed to open clipboard
    OpenFailed,
    /// Failed to empty clipboard
    EmptyFailed,
    /// Failed to allocate memory
    AllocFailed,
    /// Failed to lock memory
    LockFailed,
    /// Failed to set clipboard data
    SetDataFailed,
    /// No text data in clipboard
    NoData,
}

impl std::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenFailed => write!(f, "Failed to open clipboard"),
            Self::EmptyFailed => write!(f, "Failed to empty clipboard"),
            Self::AllocFailed => write!(f, "Failed to allocate clipboard memory"),
            Self::LockFailed => write!(f, "Failed to lock clipboard memory"),
            Self::SetDataFailed => write!(f, "Failed to set clipboard data"),
            Self::NoData => write!(f, "No text data in clipboard"),
        }
    }
}

impl std::error::Error for ClipboardError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clipboard_roundtrip() {
        let test_text = "Hello, clipboard!";

        // Copy to clipboard
        copy_to_clipboard(test_text).expect("Failed to copy");

        // Check clipboard has text
        assert!(has_text());

        // Paste from clipboard
        let pasted = paste_from_clipboard().expect("Failed to paste");
        assert_eq!(pasted, test_text);
    }

    #[test]
    fn test_clipboard_unicode() {
        let test_text = "Hello \u{1F600} World \u{4E2D}\u{6587}"; // emoji and Chinese

        copy_to_clipboard(test_text).expect("Failed to copy");
        let pasted = paste_from_clipboard().expect("Failed to paste");
        assert_eq!(pasted, test_text);
    }
}
