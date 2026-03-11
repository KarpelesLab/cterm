//! Dialog implementations for macOS
//!
//! Native macOS dialogs using NSAlert and other AppKit dialogs.

use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSModalResponseOK, NSSavePanel, NSTextField,
    NSWindow,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSSize, NSString, NSURL};
use std::path::PathBuf;

/// Show an error dialog
pub fn show_error(mtm: MainThreadMarker, parent: Option<&NSWindow>, title: &str, message: &str) {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Critical);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(message));
    alert.addButtonWithTitle(&NSString::from_str("OK"));

    if let Some(window) = parent {
        // Sheet presentation
        alert.beginSheetModalForWindow_completionHandler(window, None);
    } else {
        // Modal presentation
        alert.runModal();
    }
}

/// Show a confirmation dialog
/// Returns true if user clicked OK/Yes
pub fn show_confirm(
    mtm: MainThreadMarker,
    _parent: Option<&NSWindow>,
    title: &str,
    message: &str,
) -> bool {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Warning);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(message));
    alert.addButtonWithTitle(&NSString::from_str("OK"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));

    let response = alert.runModal();

    // First button (OK) returns NSAlertFirstButtonReturn
    response == NSAlertFirstButtonReturn
}

/// Show an input dialog
/// Returns the entered text, or None if cancelled
pub fn show_input(
    mtm: MainThreadMarker,
    _parent: Option<&NSWindow>,
    title: &str,
    message: &str,
    default: &str,
) -> Option<String> {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Informational);
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(message));
    alert.addButtonWithTitle(&NSString::from_str("OK"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));

    // Create text field for input
    let text_field = unsafe {
        let field = NSTextField::new(mtm);
        field.setStringValue(&NSString::from_str(default));
        field.setFrameSize(NSSize::new(300.0, 24.0));
        field
    };

    alert.setAccessoryView(Some(&text_field));

    // Make text field first responder
    let window = unsafe { alert.window() };
    window.makeFirstResponder(Some(&text_field));

    let response = alert.runModal();

    // First button (OK) returns NSAlertFirstButtonReturn
    if response == NSAlertFirstButtonReturn {
        Some(text_field.stringValue().to_string())
    } else {
        None
    }
}

/// Show about dialog
pub fn show_about(mtm: MainThreadMarker) {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Informational);
    alert.setMessageText(&NSString::from_str("cterm"));

    let info = format!(
        "Version {}\n\nA high-performance terminal emulator.\n\nBuilt with Rust and Metal.",
        env!("CARGO_PKG_VERSION")
    );
    alert.setInformativeText(&NSString::from_str(&info));
    alert.addButtonWithTitle(&NSString::from_str("OK"));

    alert.runModal();
}

/// Show a save panel for saving a file
///
/// Returns the selected path, or None if cancelled.
pub fn show_save_panel(
    mtm: MainThreadMarker,
    _parent: Option<&NSWindow>,
    suggested_name: Option<&str>,
    suggested_dir: Option<&std::path::Path>,
) -> Option<PathBuf> {
    let panel = NSSavePanel::savePanel(mtm);

    // Set suggested filename
    if let Some(name) = suggested_name {
        panel.setNameFieldStringValue(&NSString::from_str(name));
    }

    // Set suggested directory
    if let Some(dir) = suggested_dir {
        if let Some(dir_str) = dir.to_str() {
            let url = NSURL::fileURLWithPath(&NSString::from_str(dir_str));
            panel.setDirectoryURL(Some(&url));
        }
    }

    // Allow creating directories
    panel.setCanCreateDirectories(true);

    // Run modal
    let response = panel.runModal();

    if response == NSModalResponseOK {
        panel
            .URL()
            .and_then(|url| url.path().map(|path| PathBuf::from(path.to_string())))
    } else {
        None
    }
}

/// Result of the file drop dialog
pub enum FileDropChoice {
    PastePath,
    PasteContents,
    CreateViaBase64(String),
    CreateViaPrintf(String),
    Cancel,
}

/// Show a file drop options dialog
///
/// Returns the user's chosen action for handling the dropped file.
pub fn show_file_drop_dialog(
    mtm: MainThreadMarker,
    info: &cterm_app::file_drop::FileDropInfo,
) -> FileDropChoice {
    use cterm_app::file_drop::{format_size, SIZE_WARNING_THRESHOLD};

    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Informational);
    alert.setMessageText(&NSString::from_str(&format!(
        "File dropped: {}",
        info.filename
    )));

    let mut informative = format_size(info.size);
    if info.size > SIZE_WARNING_THRESHOLD {
        informative.push_str(&format!(
            "\n\nWarning: This file is large ({}). Sending its contents may take a while.",
            format_size(info.size)
        ));
    }
    alert.setInformativeText(&NSString::from_str(&informative));

    // Button order: first added = NSAlertFirstButtonReturn (1000)
    // The order determines the return value offset.
    alert.addButtonWithTitle(&NSString::from_str("Paste Path"));
    // Track button indices: 0 = Paste Path
    let mut paste_contents_idx: Option<usize> = None;
    let mut next_idx: usize = 1;

    if info.is_text {
        alert.addButtonWithTitle(&NSString::from_str("Paste Contents"));
        paste_contents_idx = Some(next_idx);
        next_idx += 1;
    }

    alert.addButtonWithTitle(&NSString::from_str("Create via base64"));
    let base64_idx = next_idx;
    next_idx += 1;

    alert.addButtonWithTitle(&NSString::from_str("Create via printf"));
    let printf_idx = next_idx;
    next_idx += 1;

    alert.addButtonWithTitle(&NSString::from_str("Cancel"));
    let _cancel_idx = next_idx;

    let response = alert.runModal();
    let chosen = (response - NSAlertFirstButtonReturn) as usize;

    if chosen == 0 {
        return FileDropChoice::PastePath;
    }
    if paste_contents_idx == Some(chosen) {
        return FileDropChoice::PasteContents;
    }
    if chosen == base64_idx {
        // Ask for filename
        if let Some(filename) = show_input(
            mtm,
            None,
            "Filename",
            "Enter the target filename:",
            &info.filename,
        ) {
            return FileDropChoice::CreateViaBase64(filename);
        }
        return FileDropChoice::Cancel;
    }
    if chosen == printf_idx {
        // Ask for filename
        if let Some(filename) = show_input(
            mtm,
            None,
            "Filename",
            "Enter the target filename:",
            &info.filename,
        ) {
            return FileDropChoice::CreateViaPrintf(filename);
        }
        return FileDropChoice::Cancel;
    }

    FileDropChoice::Cancel
}

/// Dialogs wrapper implementing cterm-ui traits
pub struct Dialogs {
    mtm: MainThreadMarker,
}

impl Dialogs {
    pub fn new(mtm: MainThreadMarker) -> Self {
        Self { mtm }
    }
}

impl cterm_ui::traits::Dialogs for Dialogs {
    fn show_error(&self, title: &str, message: &str) {
        show_error(self.mtm, None, title, message);
    }

    fn show_confirm(&self, title: &str, message: &str) -> bool {
        show_confirm(self.mtm, None, title, message)
    }

    fn show_input(&self, title: &str, message: &str, default: &str) -> Option<String> {
        show_input(self.mtm, None, title, message, default)
    }
}

/// Result of the color picker dialog
pub enum ColorPickerResult {
    /// User selected a color (hex string like "#FF5500")
    Color(String),
    /// User chose to clear the tab color
    Clear,
    /// User cancelled
    Cancel,
}

/// Show a color picker dialog for tab color
///
/// Returns the selected color as a hex string, or None if cancelled.
pub fn show_color_picker_dialog(
    mtm: MainThreadMarker,
    current_color: Option<&str>,
) -> ColorPickerResult {
    use objc2_app_kit::{NSColor, NSColorWell};

    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Informational);
    alert.setMessageText(&NSString::from_str("Set Tab Color"));
    alert.setInformativeText(&NSString::from_str("Choose a color for this tab:"));
    alert.addButtonWithTitle(&NSString::from_str("OK"));
    alert.addButtonWithTitle(&NSString::from_str("Clear"));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));

    // Create NSColorWell as accessory view
    let color_well_frame = objc2_foundation::NSRect::new(NSPoint::ZERO, NSSize::new(100.0, 30.0));
    let color_well: objc2::rc::Retained<NSColorWell> =
        unsafe { NSColorWell::initWithFrame(mtm.alloc(), color_well_frame) };

    // Set initial color if provided
    if let Some(hex) = current_color {
        if let Some(color) = hex_to_nscolor(mtm, hex) {
            color_well.setColor(&color);
        }
    }

    alert.setAccessoryView(Some(&color_well));

    let response = alert.runModal();

    // NSAlertFirstButtonReturn = 1000, second = 1001, third = 1002
    use objc2_app_kit::{NSAlertSecondButtonReturn, NSAlertThirdButtonReturn};

    if response == NSAlertFirstButtonReturn {
        // OK - get color from well and convert to hex
        let color = color_well.color();
        ColorPickerResult::Color(nscolor_to_hex(&color))
    } else if response == NSAlertSecondButtonReturn {
        // Clear
        ColorPickerResult::Clear
    } else {
        // Cancel or window closed
        ColorPickerResult::Cancel
    }
}

/// Convert a hex color string to NSColor
fn hex_to_nscolor(
    mtm: MainThreadMarker,
    hex: &str,
) -> Option<objc2::rc::Retained<objc2_app_kit::NSColor>> {
    use objc2_app_kit::NSColor;

    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;

    Some(unsafe {
        NSColor::colorWithRed_green_blue_alpha(
            r as f64 / 255.0,
            g as f64 / 255.0,
            b as f64 / 255.0,
            1.0,
        )
    })
}

/// Convert NSColor to hex string
fn nscolor_to_hex(color: &objc2_app_kit::NSColor) -> String {
    use objc2_app_kit::NSColorSpace;

    // Convert to sRGB color space for consistent color representation
    let rgb_color = unsafe { color.colorUsingColorSpace(NSColorSpace::sRGBColorSpace().as_ref()) };

    if let Some(rgb) = rgb_color {
        let r = (rgb.redComponent() * 255.0).round() as u8;
        let g = (rgb.greenComponent() * 255.0).round() as u8;
        let b = (rgb.blueComponent() * 255.0).round() as u8;
        format!("#{:02X}{:02X}{:02X}", r, g, b)
    } else {
        // Fallback if color conversion fails
        "#808080".to_string()
    }
}
