//! Upgrade receiver - handles receiving state from the old process during seamless upgrade
//!
//! This module is used when cterm is started with --upgrade-receiver flag.
//! It receives state from the parent process via an inherited FD, receives the terminal state
//! and PTY file descriptors, then reconstructs the windows and tabs.

use cterm_app::config::{load_config, Config};
use cterm_app::upgrade::{receive_upgrade, TabUpgradeState, UpgradeState, WindowUpgradeState};
use cterm_core::pty::Pty;
use cterm_core::screen::{Screen, ScreenConfig};
use cterm_core::term::Terminal;
use cterm_ui::theme::Theme;
use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::os::unix::io::RawFd;
use std::rc::Rc;

use crate::dialogs;
use crate::menu;
use crate::tab_bar::TabBar;
use crate::terminal_widget::TerminalWidget;

/// Run the upgrade receiver
///
/// This function:
/// 1. Reads from the inherited FD passed by the parent
/// 2. Receives the upgrade state and PTY file descriptors
/// 3. Sends acknowledgment
/// 4. Reconstructs the GTK application with the received state
pub fn run_receiver(fd: i32) -> glib::ExitCode {
    match receive_and_reconstruct(fd) {
        Ok(()) => glib::ExitCode::SUCCESS,
        Err(e) => {
            log::error!("Upgrade receiver failed: {}", e);
            glib::ExitCode::FAILURE
        }
    }
}

fn receive_and_reconstruct(fd: i32) -> Result<(), Box<dyn std::error::Error>> {
    // Use the upgrade module to receive the state
    let (state, fds) = receive_upgrade(fd as RawFd)?;

    log::info!(
        "Upgrade state: format_version={}, cterm_version={}, windows={}",
        state.format_version,
        state.cterm_version,
        state.windows.len()
    );

    log::info!("Starting GTK with restored state...");

    // Now start GTK and reconstruct the windows
    run_gtk_with_state(state, fds)?;

    Ok(())
}

/// Start GTK application with the restored state
fn run_gtk_with_state(
    state: UpgradeState,
    fds: Vec<RawFd>,
) -> Result<(), Box<dyn std::error::Error>> {
    use gtk4::gio;
    use gtk4::Application;

    // Store state and FDs for use during window construction
    // We use thread-local storage since GTK callbacks don't easily pass data
    UPGRADE_STATE.with(|s| {
        *s.borrow_mut() = Some((state, fds));
    });

    // Use NON_UNIQUE flag to prevent DBus conflicts with the old instance
    // that may still be shutting down
    let app = Application::builder()
        .application_id("com.cterm.terminal")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_activate(|app| {
        // Retrieve the stored state
        UPGRADE_STATE.with(|s| {
            if let Some((state, fds)) = s.borrow_mut().take() {
                reconstruct_windows(app, state, fds);
            }
        });
    });

    // Use run_with_args with empty args to prevent GTK from parsing
    // the command line (which contains --upgrade-receiver that GTK doesn't know)
    app.run_with_args(&[] as &[&str]);
    Ok(())
}

// Thread-local storage for upgrade state (used to pass data to GTK callback)
thread_local! {
    static UPGRADE_STATE: std::cell::RefCell<Option<(UpgradeState, Vec<RawFd>)>> =
        const { std::cell::RefCell::new(None) };
}

/// Reconstruct windows from the upgrade state
fn reconstruct_windows(app: &gtk4::Application, state: UpgradeState, fds: Vec<RawFd>) {
    log::info!("Reconstructing {} windows", state.windows.len());

    // Load config and theme
    let config = load_config().unwrap_or_default();
    let theme = Theme::default();

    for (window_idx, window_state) in state.windows.into_iter().enumerate() {
        log::info!(
            "Window {}: {}x{} at ({}, {}), {} tabs, active={}",
            window_idx,
            window_state.width,
            window_state.height,
            window_state.x,
            window_state.y,
            window_state.tabs.len(),
            window_state.active_tab
        );

        match create_restored_window(app, &config, &theme, window_state, &fds) {
            Ok(window) => {
                window.present();
                log::info!("Window {} restored successfully", window_idx);
            }
            Err(e) => {
                log::error!("Failed to restore window {}: {}", window_idx, e);
            }
        }
    }

    // Close received FDs that weren't used (shouldn't happen normally)
    // Note: FDs that were used are now owned by the NativePty instances
    // and will be closed when those are dropped
}

/// Create a restored window with its tabs
fn create_restored_window(
    app: &gtk4::Application,
    config: &Config,
    theme: &Theme,
    window_state: WindowUpgradeState,
    fds: &[RawFd],
) -> Result<gtk4::ApplicationWindow, Box<dyn std::error::Error>> {
    use gtk4::{ApplicationWindow, Box as GtkBox, Notebook, Orientation, PopoverMenuBar};

    // Create the main window
    let window = ApplicationWindow::builder()
        .application(app)
        .title("cterm")
        .default_width(window_state.width)
        .default_height(window_state.height)
        .build();

    // Set position if available (may not work on all window managers)
    // Note: GTK4 doesn't have a direct way to set window position

    // Create the main container
    let main_box = GtkBox::new(Orientation::Vertical, 0);

    // Create menu bar
    let menu_model = menu::create_menu_model();
    let menu_bar = PopoverMenuBar::from_model(Some(&menu_model));
    main_box.append(&menu_bar);

    // Create tab bar
    let tab_bar = TabBar::new();
    main_box.append(tab_bar.widget());

    // Create notebook for terminal tabs
    let notebook = Notebook::builder()
        .show_tabs(false)
        .show_border(false)
        .vexpand(true)
        .hexpand(true)
        .build();

    main_box.append(&notebook);
    window.set_child(Some(&main_box));

    // Track tabs for callbacks
    let tabs: Rc<RefCell<Vec<(u64, String, TerminalWidget)>>> = Rc::new(RefCell::new(Vec::new()));

    // Track bell state for window title
    let has_bell: Rc<RefCell<bool>> = Rc::new(RefCell::new(false));

    // Set up window actions (required for menu items to work)
    setup_window_actions(
        &window, &notebook, &tabs, &tab_bar, &has_bell, config, theme,
    );

    // Reconstruct each tab
    for (tab_idx, tab_state) in window_state.tabs.into_iter().enumerate() {
        log::info!(
            "  Restoring tab {}: id={}, title='{}', fd_index={}, child_pid={}",
            tab_idx,
            tab_state.id,
            tab_state.title,
            tab_state.pty_fd_index,
            tab_state.child_pid
        );

        match create_restored_tab(config, theme, tab_state, fds) {
            Ok((tab_id, title, terminal_widget)) => {
                // Add to notebook
                notebook.append_page(terminal_widget.widget(), None::<&gtk4::Widget>);

                // Add to tab bar
                tab_bar.add_tab(tab_id, &title);

                // Set up close callback
                let notebook_close = notebook.clone();
                let tabs_close = Rc::clone(&tabs);
                let tab_bar_close = tab_bar.clone();
                let window_close = window.clone();
                tab_bar.set_on_close(tab_id, move || {
                    close_tab_by_id(
                        &notebook_close,
                        &tabs_close,
                        &tab_bar_close,
                        &window_close,
                        tab_id,
                    );
                });

                // Set up click callback
                let notebook_click = notebook.clone();
                let tabs_click = Rc::clone(&tabs);
                let tab_bar_click = tab_bar.clone();
                tab_bar.set_on_click(tab_id, move || {
                    let tabs = tabs_click.borrow();
                    if let Some(idx) = tabs.iter().position(|(id, _, _)| *id == tab_id) {
                        notebook_click.set_current_page(Some(idx as u32));
                        tab_bar_click.set_active(tab_id);
                        tab_bar_click.clear_bell(tab_id);
                        // Focus the terminal widget
                        if let Some(widget) = notebook_click.nth_page(Some(idx as u32)) {
                            widget.grab_focus();
                        }
                    }
                });

                // Set up exit callback
                let notebook_exit = notebook.clone();
                let tabs_exit = Rc::clone(&tabs);
                let tab_bar_exit = tab_bar.clone();
                let window_exit = window.clone();
                terminal_widget.set_on_exit(move || {
                    close_tab_by_id(
                        &notebook_exit,
                        &tabs_exit,
                        &tab_bar_exit,
                        &window_exit,
                        tab_id,
                    );
                });

                // Set up bell callback
                let tab_bar_bell = tab_bar.clone();
                let notebook_bell = notebook.clone();
                let tabs_bell = Rc::clone(&tabs);
                let window_bell = window.clone();
                let has_bell_bell = Rc::clone(&has_bell);
                terminal_widget.set_on_bell(move || {
                    let is_window_active = window_bell.is_active();
                    let is_current_tab = if let Some(current_page) = notebook_bell.current_page() {
                        let tabs = tabs_bell.borrow();
                        tabs.get(current_page as usize)
                            .map(|(id, _, _)| *id == tab_id)
                            .unwrap_or(false)
                    } else {
                        false
                    };

                    // Show bell indicator on tab if not current or window not active
                    if !is_current_tab || !is_window_active {
                        tab_bar_bell.set_bell(tab_id, true);
                    }

                    // Update window title if window is not active
                    if !is_window_active {
                        *has_bell_bell.borrow_mut() = true;
                        window_bell.set_title(Some("🔔 cterm"));
                    }
                });

                // Store the tab
                tabs.borrow_mut().push((tab_id, title, terminal_widget));
            }
            Err(e) => {
                log::error!("Failed to restore tab {}: {}", tab_idx, e);
            }
        }
    }

    // Update tab bar visibility (hide if only one tab)
    tab_bar.update_visibility();

    // Set up window focus handler to clear bell when window becomes active
    {
        let has_bell_focus = Rc::clone(&has_bell);
        let window_focus = window.clone();
        let tab_bar_focus = tab_bar.clone();
        let tabs_focus = Rc::clone(&tabs);
        let notebook_focus = notebook.clone();
        window.connect_is_active_notify(move |win| {
            if win.is_active() {
                let mut bell = has_bell_focus.borrow_mut();
                if *bell {
                    *bell = false;
                    window_focus.set_title(Some("cterm"));

                    // Clear bell on the currently active tab
                    if let Some(page_idx) = notebook_focus.current_page() {
                        let tabs = tabs_focus.borrow();
                        if let Some((tab_id, _, _)) = tabs.get(page_idx as usize) {
                            tab_bar_focus.clear_bell(*tab_id);
                        }
                    }
                }
            }
        });
    }

    // Set active tab
    if window_state.active_tab < tabs.borrow().len() {
        notebook.set_current_page(Some(window_state.active_tab as u32));
        if let Some((id, _, _)) = tabs.borrow().get(window_state.active_tab) {
            tab_bar.set_active(*id);
        }
    }

    // Focus the current terminal (deferred until after window is realized)
    {
        let notebook_focus = notebook.clone();
        glib::idle_add_local_once(move || {
            if let Some(page) = notebook_focus.current_page() {
                if let Some(widget) = notebook_focus.nth_page(Some(page)) {
                    widget.grab_focus();
                }
            }
        });
    }

    // Handle maximized/fullscreen state
    if window_state.maximized {
        window.maximize();
    }
    if window_state.fullscreen {
        window.fullscreen();
    }

    // Set up terminal focus restoration after menu interactions
    {
        let focus_controller = gtk4::EventControllerKey::new();
        focus_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

        let notebook_focus = notebook.clone();
        let tabs_focus = Rc::clone(&tabs);

        focus_controller.connect_key_pressed(move |controller, keyval, _keycode, state| {
            // Skip modifier keys and menu activation keys
            let is_modifier = matches!(
                keyval,
                gdk::Key::Shift_L
                    | gdk::Key::Shift_R
                    | gdk::Key::Control_L
                    | gdk::Key::Control_R
                    | gdk::Key::Alt_L
                    | gdk::Key::Alt_R
                    | gdk::Key::Super_L
                    | gdk::Key::Super_R
                    | gdk::Key::Meta_L
                    | gdk::Key::Meta_R
                    | gdk::Key::F10
            );

            if is_modifier {
                return glib::Propagation::Proceed;
            }

            // Check if focus is on the terminal
            if let Some(widget) = controller.widget() {
                if let Some(focus_widget) = widget.focus_child() {
                    if !focus_widget.has_css_class("terminal") {
                        // Focus is not on terminal - restore it and forward the key
                        if let Some(page_idx) = notebook_focus.current_page() {
                            let tabs_ref = tabs_focus.borrow();
                            if let Some((_, _, terminal)) = tabs_ref.get(page_idx as usize) {
                                // Grab focus
                                terminal.widget().grab_focus();

                                // Forward the key to the terminal
                                let has_ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
                                let has_alt = state.contains(gdk::ModifierType::ALT_MASK);

                                if let Some(c) = keyval.to_unicode() {
                                    if has_ctrl && !has_alt {
                                        // Ctrl+key - convert to control character
                                        let ctrl_char = match c.to_ascii_lowercase() {
                                            'a'..='z' => Some((c.to_ascii_lowercase() as u8 - b'a' + 1) as char),
                                            '[' | '3' => Some('\x1b'), // Escape
                                            '\\' | '4' => Some('\x1c'),
                                            ']' | '5' => Some('\x1d'),
                                            '^' | '6' => Some('\x1e'),
                                            '_' | '7' => Some('\x1f'),
                                            '@' | '2' => Some('\x00'),
                                            _ => None,
                                        };
                                        if let Some(ctrl) = ctrl_char {
                                            terminal.write_str(&ctrl.to_string());
                                            terminal.widget().queue_draw();
                                            return glib::Propagation::Stop;
                                        }
                                    } else if !has_ctrl && !has_alt {
                                        // Simple character - write directly
                                        let mut s = [0u8; 4];
                                        let s = c.encode_utf8(&mut s);
                                        terminal.write_str(s);
                                        terminal.widget().queue_draw();
                                        return glib::Propagation::Stop;
                                    }
                                }

                                // For special keys or Alt combinations, let the terminal's
                                // key handler process it. Focus is now on the terminal.
                            }
                        }
                    }
                }
            }

            glib::Propagation::Proceed
        });

        window.add_controller(focus_controller);
    }

    Ok(window)
}

/// Create a restored terminal tab
fn create_restored_tab(
    config: &Config,
    theme: &Theme,
    tab_state: TabUpgradeState,
    fds: &[RawFd],
) -> Result<(u64, String, TerminalWidget), Box<dyn std::error::Error>> {
    // Get the PTY FD for this tab
    if tab_state.pty_fd_index >= fds.len() {
        return Err(format!(
            "PTY FD index {} out of range (max {})",
            tab_state.pty_fd_index,
            fds.len()
        )
        .into());
    }

    let pty_fd = fds[tab_state.pty_fd_index];

    // Reconstruct Pty from the FD and child PID
    let pty = unsafe { Pty::from_raw_fd(pty_fd, tab_state.child_pid) };

    // Reconstruct Screen from the terminal state
    let term_state = &tab_state.terminal;
    let screen_config = ScreenConfig {
        scrollback_lines: config.general.scrollback_lines,
    };

    let screen = Screen::from_upgrade_state(
        term_state.grid.clone(),
        term_state.scrollback.clone(),
        term_state.alternate_grid.clone(),
        term_state.cursor.clone(),
        term_state.saved_cursor.clone(),
        term_state.alt_saved_cursor.clone(),
        term_state.scroll_region,
        term_state.style.clone(),
        term_state.modes.clone(),
        term_state.title.clone(),
        term_state.scroll_offset,
        term_state.tab_stops.clone(),
        screen_config,
    );

    // Create Terminal with the restored screen and PTY
    let terminal = Terminal::from_restored(screen, pty);

    // Create TerminalWidget with the restored terminal
    let terminal_widget = TerminalWidget::from_restored(terminal, config, theme);

    Ok((tab_state.id, tab_state.title, terminal_widget))
}

/// Close a tab by its ID
#[allow(clippy::type_complexity)]
fn close_tab_by_id(
    notebook: &gtk4::Notebook,
    tabs: &Rc<RefCell<Vec<(u64, String, TerminalWidget)>>>,
    tab_bar: &TabBar,
    window: &gtk4::ApplicationWindow,
    id: u64,
) {
    // Find index of this tab
    let index = {
        let tabs = tabs.borrow();
        tabs.iter().position(|(tab_id, _, _)| *tab_id == id)
    };

    let Some(index) = index else { return };

    // Remove from notebook
    notebook.remove_page(Some(index as u32));

    // Remove from tabs list
    tabs.borrow_mut().remove(index);

    // Remove from tab bar
    tab_bar.remove_tab(id);

    // Update tab bar visibility (hide if only one tab)
    tab_bar.update_visibility();

    // Close window if no tabs left
    if tabs.borrow().is_empty() {
        window.close();
        return;
    }

    // Update active tab in tab bar
    if let Some(page_idx) = notebook.current_page() {
        let tabs = tabs.borrow();
        if let Some((active_id, _, _)) = tabs.get(page_idx as usize) {
            tab_bar.set_active(*active_id);
            tab_bar.clear_bell(*active_id);
        }
    }

    // Focus the current terminal
    if let Some(page) = notebook.current_page() {
        if let Some(widget) = notebook.nth_page(Some(page)) {
            widget.grab_focus();
        }
    }
}

/// Set up window actions for menu items
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn setup_window_actions(
    window: &gtk4::ApplicationWindow,
    notebook: &gtk4::Notebook,
    tabs: &Rc<RefCell<Vec<(u64, String, TerminalWidget)>>>,
    tab_bar: &TabBar,
    _has_bell: &Rc<RefCell<bool>>,
    _config: &Config,
    _theme: &Theme,
) {
    // New Tab action (stub - full implementation requires more complex setup)
    {
        let action = gio::SimpleAction::new("new-tab", None);
        action.connect_activate(|_, _| {
            log::info!("New tab requested from restored window (not fully implemented)");
        });
        window.add_action(&action);
    }

    // Close Tab action
    {
        let action = gio::SimpleAction::new("close-tab", None);
        let notebook_clone = notebook.clone();
        let tabs_clone = Rc::clone(tabs);
        let tab_bar_clone = tab_bar.clone();
        let window_clone = window.clone();
        action.connect_activate(move |_, _| {
            if let Some(page_idx) = notebook_clone.current_page() {
                let tab_id = {
                    let tabs = tabs_clone.borrow();
                    tabs.get(page_idx as usize).map(|(id, _, _)| *id)
                };
                if let Some(id) = tab_id {
                    close_tab_by_id(
                        &notebook_clone,
                        &tabs_clone,
                        &tab_bar_clone,
                        &window_clone,
                        id,
                    );
                }
            }
        });
        window.add_action(&action);
    }

    // Quit action
    {
        let action = gio::SimpleAction::new("quit", None);
        let window_clone = window.clone();
        action.connect_activate(move |_, _| {
            window_clone.close();
        });
        window.add_action(&action);
    }

    // Previous Tab action
    {
        let action = gio::SimpleAction::new("prev-tab", None);
        let notebook = notebook.clone();
        let tabs = Rc::clone(tabs);
        let tab_bar = tab_bar.clone();
        action.connect_activate(move |_, _| {
            if let Some(current) = notebook.current_page() {
                let n_pages = notebook.n_pages();
                if n_pages > 1 {
                    let new_page = if current == 0 {
                        n_pages - 1
                    } else {
                        current - 1
                    };
                    notebook.set_current_page(Some(new_page));
                    let tabs = tabs.borrow();
                    if let Some((id, _, _)) = tabs.get(new_page as usize) {
                        tab_bar.set_active(*id);
                        tab_bar.clear_bell(*id);
                    }
                }
            }
        });
        window.add_action(&action);
    }

    // Next Tab action
    {
        let action = gio::SimpleAction::new("next-tab", None);
        let notebook = notebook.clone();
        let tabs = Rc::clone(tabs);
        let tab_bar = tab_bar.clone();
        action.connect_activate(move |_, _| {
            if let Some(current) = notebook.current_page() {
                let n_pages = notebook.n_pages();
                if n_pages > 1 {
                    let new_page = (current + 1) % n_pages;
                    notebook.set_current_page(Some(new_page));
                    let tabs = tabs.borrow();
                    if let Some((id, _, _)) = tabs.get(new_page as usize) {
                        tab_bar.set_active(*id);
                        tab_bar.clear_bell(*id);
                    }
                }
            }
        });
        window.add_action(&action);
    }

    // About action
    {
        let action = gio::SimpleAction::new("about", None);
        let window_clone = window.clone();
        action.connect_activate(move |_, _| {
            dialogs::show_about_dialog(&window_clone);
        });
        window.add_action(&action);
    }

    // Stub actions for menu items that need more complex setup
    // These are no-ops but prevent "unknown action" warnings
    for name in &[
        "new-window",
        "close-other-tabs",
        "docker-picker",
        "copy",
        "paste",
        "select-all",
        "copy-html",
        "set-title",
        "set-color",
        "find",
        "set-encoding",
        "send-signal",
        "reset",
        "clear-reset",
        "switch-tab",
        "preferences",
        "check-updates",
        "debug-relaunch",
        "debug-dump-state",
    ] {
        let action = gio::SimpleAction::new(name, None);
        window.add_action(&action);
    }
}
