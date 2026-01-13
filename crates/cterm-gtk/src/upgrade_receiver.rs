//! Upgrade receiver - handles receiving state from the old process during seamless upgrade
//!
//! This module is used when cterm is started with --upgrade-receiver flag.
//! It connects to the old process via Unix socket, receives the terminal state
//! and PTY file descriptors, then reconstructs the windows and tabs.

use cterm_app::config::{load_config, Config};
use cterm_app::upgrade::{TabUpgradeState, UpgradeState, WindowUpgradeState};
use cterm_core::fd_passing;
use cterm_core::pty::Pty;
use cterm_core::screen::{Screen, ScreenConfig};
use cterm_core::term::Terminal;
use cterm_ui::theme::Theme;
use gtk4::glib;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::io::Write;
use std::os::unix::io::RawFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::rc::Rc;

use crate::menu;
use crate::tab_bar::TabBar;
use crate::terminal_widget::TerminalWidget;

/// Maximum size of the state buffer (64MB)
const MAX_STATE_SIZE: usize = 64 * 1024 * 1024;

/// Maximum number of file descriptors to receive
const MAX_FDS: usize = 256;

/// Run the upgrade receiver
///
/// This function:
/// 1. Connects to the Unix socket at the given path
/// 2. Receives the upgrade state and PTY file descriptors
/// 3. Sends acknowledgment
/// 4. Reconstructs the GTK application with the received state
pub fn run_receiver(socket_path: &Path) -> glib::ExitCode {
    match receive_and_reconstruct(socket_path) {
        Ok(()) => glib::ExitCode::SUCCESS,
        Err(e) => {
            log::error!("Upgrade receiver failed: {}", e);
            glib::ExitCode::FAILURE
        }
    }
}

fn receive_and_reconstruct(socket_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("Connecting to upgrade socket: {:?}", socket_path);

    // Connect to the socket
    let mut stream = UnixStream::connect(socket_path)?;

    log::info!("Connected, receiving state and file descriptors...");

    // Receive state and FDs
    let mut buf = vec![0u8; MAX_STATE_SIZE];
    let (fds, data_len) = fd_passing::recv_fds(&stream, MAX_FDS, &mut buf)?;

    log::info!(
        "Received {} bytes of state data and {} file descriptors",
        data_len,
        fds.len()
    );

    // Deserialize the state
    let state: UpgradeState = bincode::deserialize(&buf[..data_len])?;

    log::info!(
        "Upgrade state: format_version={}, cterm_version={}, windows={}",
        state.format_version,
        state.cterm_version,
        state.windows.len()
    );

    // Send acknowledgment before we start GTK (so old process can exit)
    stream.write_all(&[1])?;
    stream.flush()?;

    log::info!("Sent acknowledgment, starting GTK with restored state...");

    // Now start GTK and reconstruct the windows
    run_gtk_with_state(state, fds)?;

    Ok(())
}

/// Start GTK application with the restored state
fn run_gtk_with_state(
    state: UpgradeState,
    fds: Vec<RawFd>,
) -> Result<(), Box<dyn std::error::Error>> {
    use gtk4::Application;

    // Store state and FDs for use during window construction
    // We use thread-local storage since GTK callbacks don't easily pass data
    UPGRADE_STATE.with(|s| {
        *s.borrow_mut() = Some((state, fds));
    });

    let app = Application::builder()
        .application_id("com.cterm.terminal")
        .build();

    app.connect_activate(|app| {
        // Retrieve the stored state
        UPGRADE_STATE.with(|s| {
            if let Some((state, fds)) = s.borrow_mut().take() {
                reconstruct_windows(app, state, fds);
            }
        });
    });

    app.run();
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
                terminal_widget.set_on_bell(move || {
                    if let Some(current_page) = notebook_bell.current_page() {
                        let tabs = tabs_bell.borrow();
                        if let Some((current_id, _, _)) = tabs.get(current_page as usize) {
                            if *current_id != tab_id {
                                tab_bar_bell.set_bell(tab_id, true);
                            }
                        }
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

    // Set active tab
    if window_state.active_tab < tabs.borrow().len() {
        notebook.set_current_page(Some(window_state.active_tab as u32));
        if let Some((id, _, _)) = tabs.borrow().get(window_state.active_tab) {
            tab_bar.set_active(*id);
        }
    }

    // Focus the current terminal
    if let Some(page) = notebook.current_page() {
        if let Some(widget) = notebook.nth_page(Some(page)) {
            widget.grab_focus();
        }
    }

    // Handle maximized/fullscreen state
    if window_state.maximized {
        window.maximize();
    }
    if window_state.fullscreen {
        window.fullscreen();
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
