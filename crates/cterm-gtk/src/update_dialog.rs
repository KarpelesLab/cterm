//! Update dialog for checking and installing updates
//!
//! This dialog shows:
//! - Checking for updates... (with spinner)
//! - Update available (with version info and release notes)
//! - Downloading progress
//! - Ready to upgrade button
//! - Error messages

use cterm_app::upgrade::{UpdateError, UpdateInfo, Updater};
use gtk4::prelude::*;
use gtk4::{glib, Align, Box as GtkBox, Button, Label, Orientation, ProgressBar, Spinner, Window};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// Current application version
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// GitHub repository for updates
const GITHUB_REPO: &str = "unixshells/cterm";

/// State of the update process
#[derive(Debug, Clone)]
#[allow(dead_code)]
enum UpdateState {
    Checking,
    NoUpdate,
    UpdateAvailable(UpdateInfo),
    Downloading { progress: f64 },
    Downloaded { path: PathBuf, info: UpdateInfo },
    Error(String),
}

/// Create and show the update dialog
pub fn show_update_dialog(parent: &impl IsA<Window>) {
    let dialog = gtk4::Window::builder()
        .title("Check for Updates")
        .transient_for(parent)
        .modal(true)
        .default_width(500)
        .default_height(300)
        .resizable(false)
        .build();

    let content = GtkBox::new(Orientation::Vertical, 12);
    content.set_margin_top(20);
    content.set_margin_bottom(20);
    content.set_margin_start(20);
    content.set_margin_end(20);

    // Title
    let title = Label::new(Some("Software Updates"));
    title.add_css_class("title-2");
    content.append(&title);

    // Status area (will be updated during check)
    let status_box = GtkBox::new(Orientation::Vertical, 8);
    status_box.set_halign(Align::Center);
    status_box.set_valign(Align::Center);
    status_box.set_vexpand(true);

    // Initial spinner
    let spinner = Spinner::new();
    spinner.start();
    status_box.append(&spinner);

    let status_label = Label::new(Some("Checking for updates..."));
    status_box.append(&status_label);

    content.append(&status_box);

    // Progress bar (hidden initially)
    let progress_bar = ProgressBar::new();
    progress_bar.set_visible(false);
    progress_bar.set_show_text(true);
    content.append(&progress_bar);

    // Release notes (hidden initially)
    let notes_scroll = gtk4::ScrolledWindow::new();
    notes_scroll.set_visible(false);
    notes_scroll.set_vexpand(true);
    notes_scroll.set_min_content_height(100);

    let notes_label = Label::new(None);
    notes_label.set_wrap(true);
    notes_label.set_xalign(0.0);
    notes_scroll.set_child(Some(&notes_label));
    content.append(&notes_scroll);

    // Buttons
    let button_box = GtkBox::new(Orientation::Horizontal, 8);
    button_box.set_halign(Align::End);

    let close_button = Button::with_label("Close");
    button_box.append(&close_button);

    let action_button = Button::with_label("Download Update");
    action_button.add_css_class("suggested-action");
    action_button.set_visible(false);
    button_box.append(&action_button);

    content.append(&button_box);

    dialog.set_child(Some(&content));

    // State management
    let state: Rc<RefCell<UpdateState>> = Rc::new(RefCell::new(UpdateState::Checking));
    let downloaded_path: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));

    // Close button handler
    let dialog_close = dialog.clone();
    close_button.connect_clicked(move |_| {
        dialog_close.close();
    });

    // Action button handler (Download / Install)
    let state_clone = state.clone();
    let progress_bar_clone = progress_bar.clone();
    let status_label_clone = status_label.clone();
    let action_button_clone = action_button.clone();
    let _spinner_clone = spinner.clone();
    let downloaded_path_clone = downloaded_path.clone();
    let dialog_clone = dialog.clone();

    action_button.connect_clicked(move |btn| {
        let current_state = state_clone.borrow().clone();

        match current_state {
            UpdateState::UpdateAvailable(info) => {
                // Start download
                btn.set_sensitive(false);
                btn.set_label("Downloading...");
                progress_bar_clone.set_visible(true);
                progress_bar_clone.set_fraction(0.0);

                // Use Arc<Mutex> for thread-safe progress updates
                let progress_state =
                    std::sync::Arc::new(std::sync::Mutex::new((0u64, 0u64, false)));
                let progress_bar_update = progress_bar_clone.clone();
                let progress_state_timer = progress_state.clone();

                // Set up timer to poll progress state
                glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    let (downloaded, total, done) = *progress_state_timer.lock().unwrap();
                    if total > 0 {
                        let fraction = downloaded as f64 / total as f64;
                        progress_bar_update.set_fraction(fraction);
                        progress_bar_update.set_text(Some(&format!(
                            "{:.1} MB / {:.1} MB",
                            downloaded as f64 / 1_048_576.0,
                            total as f64 / 1_048_576.0
                        )));
                    }
                    if done {
                        glib::ControlFlow::Break
                    } else {
                        glib::ControlFlow::Continue
                    }
                });

                // Spawn download task
                let status_label = status_label_clone.clone();
                let action_btn = action_button_clone.clone();
                let state = state_clone.clone();
                let downloaded_path = downloaded_path_clone.clone();
                let progress_state_download = progress_state.clone();

                glib::spawn_future_local(async move {
                    match download_update(&info, move |downloaded, total| {
                        // Update shared progress state (thread-safe)
                        if let Ok(mut state) = progress_state_download.lock() {
                            *state = (downloaded, total, false);
                        }
                    })
                    .await
                    {
                        Ok(path) => {
                            // Mark progress timer as done
                            if let Ok(mut state) = progress_state.lock() {
                                state.2 = true;
                            }
                            glib::idle_add_local_once({
                                let action_btn = action_btn.clone();
                                let status_label = status_label.clone();
                                let state = state.clone();
                                let downloaded_path = downloaded_path.clone();
                                let info = info.clone();
                                let path = path.clone();
                                move || {
                                    *downloaded_path.borrow_mut() = Some(path.clone());
                                    *state.borrow_mut() = UpdateState::Downloaded { path, info };
                                    action_btn.set_label("Install and Restart");
                                    action_btn.set_sensitive(true);
                                    status_label.set_text("Download complete. Ready to install.");
                                }
                            });
                        }
                        Err(e) => {
                            // Mark progress timer as done
                            if let Ok(mut state) = progress_state.lock() {
                                state.2 = true;
                            }
                            glib::idle_add_local_once({
                                let state = state.clone();
                                let status_label = status_label.clone();
                                let action_btn = action_btn.clone();
                                move || {
                                    *state.borrow_mut() =
                                        UpdateState::Error(format!("Download failed: {}", e));
                                    status_label.set_text(&format!("Download failed: {}", e));
                                    action_btn.set_visible(false);
                                }
                            });
                        }
                    }
                });
            }
            UpdateState::Downloaded { path, .. } => {
                // Trigger upgrade
                log::info!("User requested upgrade with binary at {:?}", path);
                status_label_clone.set_text("Starting upgrade...");
                btn.set_sensitive(false);

                // Close dialog
                dialog_clone.close();

                // Signal the main window to execute the upgrade via action
                if let Some(toplevel) = dialog_clone.transient_for() {
                    let path_str = path.to_string_lossy().to_string();
                    if let Err(e) = toplevel
                        .activate_action("win.execute-upgrade", Some(&path_str.to_variant()))
                    {
                        log::error!("Failed to activate upgrade action: {}", e);
                    }
                }
            }
            _ => {}
        }
    });

    // Start checking for updates
    let state_check = state.clone();
    let spinner_check = spinner.clone();
    let status_label_check = status_label.clone();
    let action_button_check = action_button.clone();
    let notes_scroll_check = notes_scroll.clone();
    let notes_label_check = notes_label.clone();

    glib::spawn_future_local(async move {
        let result = check_for_updates().await;

        glib::idle_add_local_once(move || {
            spinner_check.stop();
            spinner_check.set_visible(false);

            match result {
                Ok(Some(info)) => {
                    *state_check.borrow_mut() = UpdateState::UpdateAvailable(info.clone());
                    status_label_check.set_text(&format!(
                        "Version {} is available (current: {})",
                        info.version, CURRENT_VERSION
                    ));
                    action_button_check.set_visible(true);

                    // Show release notes if available
                    if !info.release_notes.is_empty() {
                        notes_label_check.set_text(&info.release_notes);
                        notes_scroll_check.set_visible(true);
                    }
                }
                Ok(None) => {
                    *state_check.borrow_mut() = UpdateState::NoUpdate;
                    status_label_check.set_text(&format!(
                        "You're running the latest version ({})",
                        CURRENT_VERSION
                    ));
                }
                Err(e) => {
                    *state_check.borrow_mut() = UpdateState::Error(e.to_string());
                    status_label_check.set_text(&format!("Error checking for updates: {}", e));
                }
            }
        });
    });

    dialog.present();
}

/// Check for updates asynchronously
///
/// Runs the reqwest-based updater inside a Tokio runtime since reqwest
/// requires Tokio's reactor.
async fn check_for_updates() -> Result<Option<UpdateInfo>, UpdateError> {
    // Run in a blocking thread with its own Tokio runtime
    let (tx, rx) = futures::channel::oneshot::channel::<Result<Option<UpdateInfo>, UpdateError>>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
        let result = rt.block_on(async {
            let updater = Updater::new(GITHUB_REPO, CURRENT_VERSION)?;
            updater.check_for_update().await
        });
        let _ = tx.send(result);
    });

    rx.await.unwrap_or_else(|_| Err(UpdateError::NotFound))
}

/// Download update with progress callback
///
/// Runs the reqwest-based updater inside a Tokio runtime since reqwest
/// requires Tokio's reactor.
async fn download_update<F>(info: &UpdateInfo, on_progress: F) -> Result<PathBuf, UpdateError>
where
    F: FnMut(u64, u64) + Send + 'static,
{
    let info = info.clone();
    let (tx, rx) = futures::channel::oneshot::channel::<Result<PathBuf, UpdateError>>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
        let result = rt.block_on(async {
            let updater = Updater::new(GITHUB_REPO, CURRENT_VERSION)?;
            updater.download(&info, on_progress).await
        });
        let _ = tx.send(result);
    });

    rx.await.unwrap_or_else(|_| Err(UpdateError::NotFound))
}
