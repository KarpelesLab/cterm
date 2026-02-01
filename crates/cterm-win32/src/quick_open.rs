//! Quick Open dialog for rapidly searching and opening tab templates
//!
//! Shows a VS Code-style dialog for filtering and selecting templates.

use std::cell::RefCell;
use std::ptr;

use cterm_app::config::StickyTabConfig;
use cterm_app::{template_type_indicator, QuickOpenMatcher, TemplateMatch};
use winapi::shared::basetsd::INT_PTR;
use winapi::shared::minwindef::{LPARAM, UINT, WPARAM};
use winapi::shared::windef::HWND;
use winapi::um::winuser::*;

// Thread-local storage for dialog data
thread_local! {
    static QUICK_OPEN_TEMPLATES: RefCell<Vec<StickyTabConfig>> = const { RefCell::new(Vec::new()) };
    static QUICK_OPEN_FILTERED: RefCell<Vec<TemplateMatch>> = const { RefCell::new(Vec::new()) };
    static QUICK_OPEN_RESULT: RefCell<Option<StickyTabConfig>> = const { RefCell::new(None) };
}

/// Maximum number of results to display
const MAX_RESULTS: usize = 10;

/// Convert a Rust string to a null-terminated wide string
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// Dialog control IDs
const IDC_SEARCH_EDIT: i32 = 2001;
const IDC_RESULTS_LIST: i32 = 2002;

/// Dialog procedure for quick open dialog
unsafe extern "system" fn quick_open_dialog_proc(
    hwnd: HWND,
    msg: UINT,
    wparam: WPARAM,
    lparam: LPARAM,
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
                parent_rect.right = GetSystemMetrics(SM_CXSCREEN);
                parent_rect.bottom = GetSystemMetrics(SM_CYSCREEN);
            }

            let x = (parent_rect.left + parent_rect.right) / 2 - width / 2;
            let y = (parent_rect.top + parent_rect.bottom) / 2 - height / 2;
            SetWindowPos(hwnd, ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER);

            // Initialize with all templates
            update_filter(hwnd, "");

            // Focus the search edit
            let edit = GetDlgItem(hwnd, IDC_SEARCH_EDIT);
            SetFocus(edit);

            1 // Return TRUE to indicate we set focus
        }

        WM_COMMAND => {
            let notification = ((wparam >> 16) & 0xFFFF) as u16;
            let id = (wparam & 0xFFFF) as i32;

            match (id, notification) {
                (IDC_SEARCH_EDIT, n) if n == EN_CHANGE => {
                    // Search text changed - update filter
                    let edit = GetDlgItem(hwnd, IDC_SEARCH_EDIT);
                    let len = GetWindowTextLengthW(edit) as usize;
                    let mut buffer = vec![0u16; len + 1];
                    GetWindowTextW(edit, buffer.as_mut_ptr(), buffer.len() as i32);
                    let query = String::from_utf16_lossy(&buffer[..len]);
                    update_filter(hwnd, &query);
                    0
                }

                (IDC_RESULTS_LIST, n) if n == LBN_DBLCLK => {
                    // Double-click on list item - select it
                    let listbox = GetDlgItem(hwnd, IDC_RESULTS_LIST);
                    let sel = SendMessageW(listbox, LB_GETCURSEL, 0, 0) as i32;
                    if sel >= 0 {
                        select_item(sel as usize);
                        EndDialog(hwnd, IDOK as isize);
                    }
                    1
                }

                (IDOK, _) => {
                    // OK button or Enter pressed - select current item
                    let listbox = GetDlgItem(hwnd, IDC_RESULTS_LIST);
                    let sel = SendMessageW(listbox, LB_GETCURSEL, 0, 0) as i32;
                    if sel >= 0 {
                        select_item(sel as usize);
                        EndDialog(hwnd, IDOK as isize);
                    } else if QUICK_OPEN_FILTERED.with(|f| !f.borrow().is_empty()) {
                        // If no selection but there are items, select the first one
                        select_item(0);
                        EndDialog(hwnd, IDOK as isize);
                    }
                    1
                }

                (IDCANCEL, _) => {
                    QUICK_OPEN_RESULT.with(|r| {
                        *r.borrow_mut() = None;
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

/// Update the filter and refresh the listbox
unsafe fn update_filter(hwnd: HWND, query: &str) {
    let templates = QUICK_OPEN_TEMPLATES.with(|t| t.borrow().clone());
    let matcher = QuickOpenMatcher::new(templates);
    let filtered: Vec<TemplateMatch> = matcher
        .filter(query)
        .into_iter()
        .take(MAX_RESULTS)
        .collect();

    // Update the listbox
    let listbox = GetDlgItem(hwnd, IDC_RESULTS_LIST);

    // Clear existing items
    SendMessageW(listbox, LB_RESETCONTENT, 0, 0);

    // Add filtered items
    for match_result in &filtered {
        let indicator = template_type_indicator(&match_result.template);
        let display = if indicator.is_empty() {
            match_result.template.name.clone()
        } else {
            format!("{} {}", match_result.template.name, indicator)
        };
        let wide = to_wide(&display);
        SendMessageW(listbox, LB_ADDSTRING, 0, wide.as_ptr() as LPARAM);
    }

    // Select first item if available
    if !filtered.is_empty() {
        SendMessageW(listbox, LB_SETCURSEL, 0, 0);
    }

    // Store filtered results
    QUICK_OPEN_FILTERED.with(|f| {
        *f.borrow_mut() = filtered;
    });
}

/// Select an item from the filtered list
fn select_item(index: usize) {
    QUICK_OPEN_FILTERED.with(|f| {
        let filtered = f.borrow();
        if let Some(match_result) = filtered.get(index) {
            QUICK_OPEN_RESULT.with(|r| {
                *r.borrow_mut() = Some(match_result.template.clone());
            });
        }
    });
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

/// Build a dialog template in memory
fn build_quick_open_dialog_template() -> Vec<u8> {
    let mut template = Vec::new();

    // DLGTEMPLATE structure (must be DWORD aligned)
    let style = DS_MODALFRAME | DS_CENTER | WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_SETFONT;
    let ex_style = 0u32;
    let c_dit = 4u16; // number of controls: search edit, results list, OK, Cancel
    let x = 0i16;
    let y = 0i16;
    let cx = 300i16;
    let cy = 200i16;

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
    let title_wide = to_wide("Quick Open");
    for c in &title_wide {
        template.extend_from_slice(&c.to_le_bytes());
    }

    // Font (for DS_SETFONT)
    align_to_word(&mut template);
    template.extend_from_slice(&9u16.to_le_bytes()); // point size
    let font_wide = to_wide("Segoe UI");
    for c in &font_wide {
        template.extend_from_slice(&c.to_le_bytes());
    }

    // Control 1: Edit control for search
    align_to_dword(&mut template);
    add_dialog_control(
        &mut template,
        WS_CHILD | WS_VISIBLE | WS_BORDER | WS_TABSTOP | ES_AUTOHSCROLL,
        10,
        10,
        280,
        14,
        IDC_SEARCH_EDIT,
        0x0081, // Edit
        "",
    );

    // Control 2: Listbox for results
    align_to_dword(&mut template);
    add_dialog_control(
        &mut template,
        WS_CHILD
            | WS_VISIBLE
            | WS_BORDER
            | WS_TABSTOP
            | WS_VSCROLL
            | LBS_NOTIFY
            | LBS_NOINTEGRALHEIGHT,
        10,
        30,
        280,
        130,
        IDC_RESULTS_LIST,
        0x0083, // Listbox
        "",
    );

    // Control 3: OK button
    align_to_dword(&mut template);
    add_dialog_control(
        &mut template,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
        170,
        175,
        55,
        14,
        IDOK,
        0x0080, // Button
        "Open",
    );

    // Control 4: Cancel button
    align_to_dword(&mut template);
    add_dialog_control(
        &mut template,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_PUSHBUTTON,
        235,
        175,
        55,
        14,
        IDCANCEL,
        0x0080, // Button
        "Cancel",
    );

    template
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

/// Show the Quick Open dialog
///
/// Returns the selected template, or None if cancelled.
pub fn show_quick_open_dialog(
    parent: HWND,
    templates: Vec<StickyTabConfig>,
) -> Option<StickyTabConfig> {
    // Set up templates
    QUICK_OPEN_TEMPLATES.with(|t| {
        *t.borrow_mut() = templates;
    });
    QUICK_OPEN_FILTERED.with(|f| {
        *f.borrow_mut() = Vec::new();
    });
    QUICK_OPEN_RESULT.with(|r| {
        *r.borrow_mut() = None;
    });

    // Build dialog template
    let template = build_quick_open_dialog_template();

    // Show dialog
    let ret = unsafe {
        DialogBoxIndirectParamW(
            ptr::null_mut(),
            template.as_ptr() as *const DLGTEMPLATE,
            parent,
            Some(quick_open_dialog_proc),
            0,
        )
    };

    // Get result
    if ret == IDOK as isize {
        QUICK_OPEN_RESULT.with(|r| r.borrow().clone())
    } else {
        None
    }
}

/// Show the Quick Open dialog using windows crate HWND
pub fn show_quick_open(
    hwnd: windows::Win32::Foundation::HWND,
    templates: Vec<StickyTabConfig>,
) -> Option<StickyTabConfig> {
    let parent = hwnd.0 as *mut _;
    show_quick_open_dialog(parent, templates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_dialog_template() {
        let template = build_quick_open_dialog_template();
        // Template should be non-empty
        assert!(!template.is_empty());
    }
}
