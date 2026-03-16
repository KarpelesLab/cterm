//! Update dialog for checking and installing updates on macOS
//!
//! This module provides a native macOS dialog for checking for updates,
//! displaying release notes, and directing users to download updates.

use objc2::rc::Retained;
use objc2::MainThreadOnly;
use objc2_app_kit::{
    NSAlert, NSAlertStyle, NSFont, NSProgressIndicator, NSProgressIndicatorStyle, NSScrollView,
    NSTextView,
};
use objc2_foundation::{MainThreadMarker, NSRect, NSSize, NSString};

use cterm_app::upgrade::{UpdateError, UpdateInfo, Updater};

/// Current application version
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// GitHub repository for updates
const GITHUB_REPO: &str = "unixshells/cterm";

/// Check for updates synchronously and show result
///
/// This function spawns a background thread to check for updates,
/// then shows the result in a dialog on the main thread.
pub fn check_for_updates_sync(mtm: MainThreadMarker) {
    // Show a "checking" dialog with spinner
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Informational);
    alert.setMessageText(&NSString::from_str("Checking for Updates"));
    alert.setInformativeText(&NSString::from_str("Connecting to GitHub..."));

    // Add a spinning progress indicator
    let progress = unsafe {
        let p = NSProgressIndicator::new(mtm);
        p.setStyle(NSProgressIndicatorStyle::Spinning);
        p.setControlSize(objc2_app_kit::NSControlSize::Regular);
        p.setFrameSize(NSSize::new(32.0, 32.0));
        p.startAnimation(None);
        p
    };
    alert.setAccessoryView(Some(&progress));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));

    // Run the check in a blocking way on a background thread
    // We'll use channels to communicate the result
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");

        let result = runtime.block_on(async {
            let updater = Updater::new(GITHUB_REPO, CURRENT_VERSION)?;
            updater.check_for_update().await
        });

        let _ = tx.send(result);
    });

    // Wait for result with a timeout (poll while showing the dialog briefly)
    // Use a short modal run then check for result
    let window = unsafe { alert.window() };
    window.makeKeyAndOrderFront(None);

    // Poll for result
    let mut result = None;
    for _ in 0..100 {
        // 10 seconds max (100 * 100ms)
        std::thread::sleep(std::time::Duration::from_millis(100));

        if let Ok(r) = rx.try_recv() {
            result = Some(r);
            break;
        }

        // Process events to keep UI responsive
        unsafe {
            use objc2_app_kit::NSApplication;
            let app = NSApplication::sharedApplication(mtm);
            // Process pending events without blocking
            while let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
                objc2_app_kit::NSEventMask::Any,
                None,
                objc2_foundation::NSDefaultRunLoopMode,
                true,
            ) {
                app.sendEvent(&event);
            }
        }
    }

    // Close the checking dialog
    window.close();

    // Show result dialog
    match result {
        Some(Ok(Some(info))) => show_update_available(mtm, info),
        Some(Ok(None)) => show_no_update(mtm),
        Some(Err(e)) => show_error(mtm, e),
        None => show_timeout(mtm),
    }
}

/// Show dialog when an update is available
fn show_update_available(mtm: MainThreadMarker, info: UpdateInfo) {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Informational);
    alert.setMessageText(&NSString::from_str("Update Available"));

    let message = format!(
        "A new version of cterm is available!\n\n\
        Current version: {}\n\
        New version: {}\n\n\
        Would you like to download and install the update?\n\
        The application will restart automatically.",
        CURRENT_VERSION, info.version
    );
    alert.setInformativeText(&NSString::from_str(&message));

    // Add release notes if available
    if !info.release_notes.is_empty() {
        let scroll_view = create_release_notes_view(mtm, &info.release_notes);
        alert.setAccessoryView(Some(&scroll_view));
    }

    alert.addButtonWithTitle(&NSString::from_str("Install Update"));
    alert.addButtonWithTitle(&NSString::from_str("Open Releases"));
    alert.addButtonWithTitle(&NSString::from_str("Later"));

    let response = alert.runModal();
    if response == objc2_app_kit::NSAlertFirstButtonReturn {
        // Install update automatically
        download_and_install_update(mtm, info);
    } else if response == objc2_app_kit::NSAlertSecondButtonReturn {
        // Open releases page
        open_releases_page();
    }
}

/// Download, install, and restart the application
fn download_and_install_update(mtm: MainThreadMarker, info: UpdateInfo) {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;

    // Show download progress dialog
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Informational);
    alert.setMessageText(&NSString::from_str("Downloading Update"));
    alert.setInformativeText(&NSString::from_str("Starting download..."));

    // Add progress indicator
    let progress = unsafe {
        let p = NSProgressIndicator::new(mtm);
        p.setStyle(NSProgressIndicatorStyle::Bar);
        p.setControlSize(objc2_app_kit::NSControlSize::Regular);
        p.setFrameSize(NSSize::new(300.0, 20.0));
        p.setIndeterminate(false);
        p.setMinValue(0.0);
        p.setMaxValue(100.0);
        p
    };
    alert.setAccessoryView(Some(&progress));
    alert.addButtonWithTitle(&NSString::from_str("Cancel"));

    let window = unsafe { alert.window() };
    window.makeKeyAndOrderFront(None);

    // Shared state for progress
    let downloaded = Arc::new(AtomicU64::new(0));
    let total = Arc::new(AtomicU64::new(info.size));
    let cancelled = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let error_msg = Arc::new(parking_lot::Mutex::new(None::<String>));
    let download_path = Arc::new(parking_lot::Mutex::new(None::<std::path::PathBuf>));

    let downloaded_clone = Arc::clone(&downloaded);
    let total_clone = Arc::clone(&total);
    let cancelled_clone = Arc::clone(&cancelled);
    let finished_clone = Arc::clone(&finished);
    let error_clone = Arc::clone(&error_msg);
    let path_clone = Arc::clone(&download_path);

    // Start download in background thread
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");

        let result = runtime.block_on(async {
            let updater = Updater::new(GITHUB_REPO, CURRENT_VERSION)?;

            // Download with progress callback
            let downloaded_cb = Arc::clone(&downloaded_clone);
            let total_cb = Arc::clone(&total_clone);
            let cancelled_cb = Arc::clone(&cancelled_clone);

            let path = updater
                .download(&info, |dl, tot| {
                    downloaded_cb.store(dl, Ordering::Relaxed);
                    total_cb.store(tot, Ordering::Relaxed);

                    // Check for cancellation
                    if cancelled_cb.load(Ordering::Relaxed) {
                        // Can't really cancel mid-download, but we'll handle it
                    }
                })
                .await?;

            // Verify checksum
            updater.verify(&path, &info).await?;

            Ok::<_, UpdateError>(path)
        });

        match result {
            Ok(path) => {
                *path_clone.lock() = Some(path);
            }
            Err(e) => {
                *error_clone.lock() = Some(e.to_string());
            }
        }
        finished_clone.store(true, Ordering::Relaxed);
    });

    // Poll for progress updates
    loop {
        std::thread::sleep(std::time::Duration::from_millis(50));

        let dl = downloaded.load(Ordering::Relaxed);
        let tot = total.load(Ordering::Relaxed);

        if tot > 0 {
            let percent = (dl as f64 / tot as f64) * 100.0;
            progress.setDoubleValue(percent);

            let mb_dl = dl as f64 / 1_048_576.0;
            let mb_tot = tot as f64 / 1_048_576.0;
            let text = format!("Downloading... {:.1} / {:.1} MB", mb_dl, mb_tot);
            alert.setInformativeText(&NSString::from_str(&text));
        }

        // Process events
        unsafe {
            use objc2_app_kit::NSApplication;
            let app = NSApplication::sharedApplication(mtm);
            while let Some(event) = app.nextEventMatchingMask_untilDate_inMode_dequeue(
                objc2_app_kit::NSEventMask::Any,
                None,
                objc2_foundation::NSDefaultRunLoopMode,
                true,
            ) {
                app.sendEvent(&event);
            }
        }

        if finished.load(Ordering::Relaxed) {
            break;
        }
    }

    window.close();

    // Check for errors
    if let Some(err) = error_msg.lock().take() {
        show_install_error(mtm, &err);
        return;
    }

    // Get download path
    let archive_path = match download_path.lock().take() {
        Some(p) => p,
        None => {
            show_install_error(mtm, "Download failed: no file path");
            return;
        }
    };

    // Extract and install
    match install_update(mtm, &archive_path) {
        Ok(new_binary) => {
            // Show success and restart
            let alert = NSAlert::new(mtm);
            alert.setAlertStyle(NSAlertStyle::Informational);
            alert.setMessageText(&NSString::from_str("Update Installed"));
            alert.setInformativeText(&NSString::from_str(
                "The update has been installed successfully.\n\n\
                 Click Restart to launch the new version.",
            ));
            alert.addButtonWithTitle(&NSString::from_str("Restart Now"));
            alert.runModal();

            // Perform relaunch with the new binary
            perform_relaunch_with_binary(mtm, &new_binary);
        }
        Err(e) => {
            show_install_error(mtm, &e);
        }
    }
}

/// Extract archive and install the update
fn install_update(
    _mtm: MainThreadMarker,
    archive_path: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    // Extract the archive
    let extracted_dir =
        Updater::extract_archive(archive_path).map_err(|e| format!("Extract failed: {}", e))?;

    // Install the app bundle
    let new_binary = Updater::install_macos_update(&extracted_dir)
        .map_err(|e| format!("Install failed: {}", e))?;

    // Clean up extracted directory (optional, will be cleaned up on reboot anyway)
    let _ = std::fs::remove_dir_all(&extracted_dir);
    let _ = std::fs::remove_file(archive_path);

    Ok(new_binary)
}

/// Show installation error dialog
fn show_install_error(mtm: MainThreadMarker, error: &str) {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Critical);
    alert.setMessageText(&NSString::from_str("Update Failed"));
    alert.setInformativeText(&NSString::from_str(&format!(
        "Could not install the update:\n\n{}\n\n\
         You can try downloading the update manually from the releases page.",
        error
    )));
    alert.addButtonWithTitle(&NSString::from_str("Open Releases"));
    alert.addButtonWithTitle(&NSString::from_str("OK"));

    let response = alert.runModal();
    if response == objc2_app_kit::NSAlertFirstButtonReturn {
        open_releases_page();
    }
}

/// Perform relaunch with a specific binary path
fn perform_relaunch_with_binary(mtm: MainThreadMarker, binary_path: &std::path::Path) {
    use objc2_app_kit::NSApplication;

    log::info!("Relaunching with new binary: {}", binary_path.display());

    // Get the app delegate and call perform_relaunch
    // This will use the new binary since we've replaced the app bundle
    let app = NSApplication::sharedApplication(mtm);
    if let Some(delegate) = app.delegate() {
        // Call our perform_relaunch method
        let _: () = unsafe {
            objc2::msg_send![&*delegate, debugRelaunch: std::ptr::null::<objc2::runtime::AnyObject>()]
        };
    }
}

/// Show dialog when no update is available
fn show_no_update(mtm: MainThreadMarker) {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Informational);
    alert.setMessageText(&NSString::from_str("No Updates Available"));
    alert.setInformativeText(&NSString::from_str(&format!(
        "You're running the latest version of cterm ({}).",
        CURRENT_VERSION
    )));
    alert.addButtonWithTitle(&NSString::from_str("OK"));
    alert.runModal();
}

/// Show dialog when update check fails
fn show_error(mtm: MainThreadMarker, error: UpdateError) {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Warning);
    alert.setMessageText(&NSString::from_str("Update Check Failed"));
    alert.setInformativeText(&NSString::from_str(&format!(
        "Could not check for updates:\n\n{}",
        error
    )));
    alert.addButtonWithTitle(&NSString::from_str("OK"));
    alert.runModal();
}

/// Show dialog when update check times out
fn show_timeout(mtm: MainThreadMarker) {
    let alert = NSAlert::new(mtm);
    alert.setAlertStyle(NSAlertStyle::Warning);
    alert.setMessageText(&NSString::from_str("Update Check Timed Out"));
    alert.setInformativeText(&NSString::from_str(
        "Could not connect to GitHub to check for updates.\n\n\
        Please check your internet connection and try again.",
    ));
    alert.addButtonWithTitle(&NSString::from_str("OK"));
    alert.runModal();
}

/// Create a scroll view with release notes
fn create_release_notes_view(mtm: MainThreadMarker, notes: &str) -> Retained<NSScrollView> {
    let frame = NSRect::new(
        objc2_foundation::NSPoint::new(0.0, 0.0),
        NSSize::new(400.0, 150.0),
    );

    let scroll_view = unsafe {
        let sv = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), frame);
        sv.setHasVerticalScroller(true);
        sv.setHasHorizontalScroller(false);
        sv.setBorderType(objc2_app_kit::NSBorderType::BezelBorder);
        sv
    };

    let text_view = unsafe {
        let content_size = scroll_view.contentSize();
        let text_frame = NSRect::new(objc2_foundation::NSPoint::new(0.0, 0.0), content_size);
        let tv = NSTextView::initWithFrame(NSTextView::alloc(mtm), text_frame);
        tv.setEditable(false);
        tv.setString(&NSString::from_str(notes));
        if let Some(font) = NSFont::userFixedPitchFontOfSize(11.0) {
            tv.setFont(Some(&font));
        }
        tv
    };

    scroll_view.setDocumentView(Some(&text_view));
    scroll_view
}

/// Open the GitHub releases page in the default browser
fn open_releases_page() {
    let url = format!("https://github.com/{}/releases", GITHUB_REPO);
    let _ = std::process::Command::new("open").arg(&url).spawn();
}
