//! Dialog implementations for macOS
//!
//! Native macOS dialogs using NSAlert and other AppKit dialogs.

use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSModalResponseOK, NSSavePanel, NSTextField,
    NSWindow,
};
use objc2_foundation::{MainThreadMarker, NSSize, NSString, NSURL};
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

/// Show crash recovery dialog
/// Returns true if user wants to report the crash
#[cfg(unix)]
pub fn show_crash_recovery(
    mtm: MainThreadMarker,
    signal: i32,
    previous_pid: i32,
    recovered_count: usize,
) -> bool {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Warning);
    alert.setMessageText(&NSString::from_str("cterm recovered from a crash"));

    let signal_name = match signal {
        11 => "SIGSEGV (segmentation fault)",
        10 => "SIGBUS (bus error)",
        6 => "SIGABRT (abort)",
        4 => "SIGILL (illegal instruction)",
        8 => "SIGFPE (floating point exception)",
        _ => "unknown signal",
    };

    let info = format!(
        "The previous cterm process (PID {}) crashed with {}.\n\n\
        {} terminal session{} {} been recovered and should continue working normally.\n\n\
        Would you like to report this crash to help improve cterm?",
        previous_pid,
        signal_name,
        recovered_count,
        if recovered_count == 1 { "" } else { "s" },
        if recovered_count == 1 { "has" } else { "have" }
    );
    alert.setInformativeText(&NSString::from_str(&info));

    alert.addButtonWithTitle(&NSString::from_str("Report Crash"));
    alert.addButtonWithTitle(&NSString::from_str("Don't Report"));

    let response = alert.runModal();
    response == NSAlertFirstButtonReturn
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
