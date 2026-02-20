//! Upgrade receiver - handles receiving state from the old process during seamless upgrade
//!
//! This module is used when cterm is started with --upgrade-receiver flag.
//! It receives state from the parent process via an inherited pipe handle, receives the
//! terminal state and PTY handles, then reconstructs the windows and tabs.

use std::os::windows::io::RawHandle;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread;

use cterm_app::config::{load_config, Config};
use cterm_app::upgrade::{receive_upgrade, TabUpgradeState, UpgradeState, WindowUpgradeState};
use cterm_core::pty::Pty;
use cterm_core::screen::{Screen, ScreenConfig};
use cterm_core::term::Terminal;
use cterm_ui::theme::Theme;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, TranslateMessage, MSG,
};

use crate::window::{TabEntry, WindowState};

/// Run the upgrade receiver
///
/// This function:
/// 1. Reads from the inherited pipe handle passed by the parent
/// 2. Receives the upgrade state and PTY handles
/// 3. Sends acknowledgment
/// 4. Reconstructs the Windows application with the received state
pub fn run_receiver(handle: usize) -> i32 {
    match receive_and_reconstruct(handle) {
        Ok(()) => 0,
        Err(e) => {
            log::error!("Upgrade receiver failed: {}", e);
            1
        }
    }
}

fn receive_and_reconstruct(handle: usize) -> Result<(), Box<dyn std::error::Error>> {
    // Use the upgrade module to receive the state
    let (state, handles) = receive_upgrade(handle)?;

    log::info!(
        "Upgrade state: format_version={}, cterm_version={}, windows={}",
        state.format_version,
        state.cterm_version,
        state.windows.len()
    );

    log::info!("Starting Windows app with restored state...");

    // Reconstruct the windows
    run_with_restored_state(state, handles)?;

    Ok(())
}

/// Run the Windows application with restored state
fn run_with_restored_state(
    state: UpgradeState,
    handles: Vec<(RawHandle, RawHandle, RawHandle, RawHandle, u32)>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Load config and theme
    let config = load_config().unwrap_or_default();
    let theme = load_theme(&config);

    // Set up DPI awareness
    crate::dpi::setup_dpi_awareness();

    // Initialize common controls for dialogs
    crate::dialog_utils::init_common_controls();

    // Register window class
    crate::window::register_window_class()?;

    // Reconstruct each window
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

        match create_restored_window(&config, &theme, window_state, &handles) {
            Ok(_hwnd) => {
                log::info!("Window {} restored successfully", window_idx);
            }
            Err(e) => {
                log::error!("Failed to restore window {}: {}", window_idx, e);
            }
        }
    }

    // Message loop
    let mut msg = MSG::default();
    loop {
        let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };

        if ret.0 == 0 {
            // WM_QUIT
            break;
        }

        if ret.0 == -1 {
            // Error
            return Err("GetMessageW error".into());
        }

        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    log::info!("cterm (restored) exiting");
    Ok(())
}

/// Load the theme based on config
fn load_theme(config: &Config) -> Theme {
    cterm_app::resolve_theme(config)
}

/// Create a restored window with its tabs
fn create_restored_window(
    config: &Config,
    theme: &Theme,
    window_state: WindowUpgradeState,
    handles: &[(RawHandle, RawHandle, RawHandle, RawHandle, u32)],
) -> Result<HWND, Box<dyn std::error::Error>> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::*;

    let class_name: Vec<u16> = crate::window::WINDOW_CLASS
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let title: Vec<u16> = "cterm".encode_utf16().chain(std::iter::once(0)).collect();

    // Create window with restored size
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            window_state.x,
            window_state.y,
            window_state.width,
            window_state.height,
            None,
            None,
            None,
            None,
        )?
    };

    // Create window state
    let mut state = Box::new(WindowState::new(hwnd, config, theme));
    state.init_renderer()?;

    // Reconstruct each tab
    for (tab_idx, tab_state) in window_state.tabs.into_iter().enumerate() {
        log::info!(
            "  Restoring tab {}: id={}, title='{}', handle_index={}, child_pid={}",
            tab_idx,
            tab_state.id,
            tab_state.title,
            tab_state.pty_fd_index,
            tab_state.process_id
        );

        match create_restored_tab(config, tab_state, handles, &mut state, hwnd) {
            Ok(()) => {
                log::info!("Tab {} restored", tab_idx);
            }
            Err(e) => {
                log::error!("Failed to restore tab {}: {}", tab_idx, e);
            }
        }
    }

    // Set active tab
    if window_state.active_tab < state.tabs.len() {
        state.switch_to_tab(window_state.active_tab);
    }

    // Handle maximized/fullscreen state
    if window_state.maximized {
        unsafe {
            let _ = ShowWindow(hwnd, SW_MAXIMIZE);
        }
    }
    if window_state.fullscreen {
        // Toggle fullscreen
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongW, SetWindowLongW, SetWindowPos, GWL_STYLE, HWND_TOP, SWP_FRAMECHANGED,
            SWP_NOMOVE, SWP_NOSIZE, WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU,
            WS_THICKFRAME,
        };

        unsafe {
            let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            let windowed_style =
                WS_CAPTION.0 | WS_SYSMENU.0 | WS_THICKFRAME.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0;
            let new_style = style & !windowed_style;
            SetWindowLongW(hwnd, GWL_STYLE, new_style as i32);
            let _ = ShowWindow(hwnd, SW_MAXIMIZE);
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_FRAMECHANGED,
            );
        }
    }

    // Store state pointer in window
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
    }

    Ok(hwnd)
}

/// Create a restored terminal tab
fn create_restored_tab(
    config: &Config,
    tab_state: TabUpgradeState,
    handles: &[(RawHandle, RawHandle, RawHandle, RawHandle, u32)],
    state: &mut WindowState,
    hwnd: HWND,
) -> Result<(), Box<dyn std::error::Error>> {
    // Get the PTY handles for this tab
    if tab_state.pty_fd_index >= handles.len() {
        return Err(format!(
            "PTY handle index {} out of range (max {})",
            tab_state.pty_fd_index,
            handles.len()
        )
        .into());
    }

    let (hpc, read_pipe, write_pipe, process_handle, process_id) = handles[tab_state.pty_fd_index];

    // Reconstruct Pty from the handles
    let pty =
        unsafe { Pty::from_raw_handles(hpc, read_pipe, write_pipe, process_handle, process_id) };

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
    let terminal = Arc::new(Mutex::new(terminal));

    // Start PTY reader thread
    let reader_handle = start_pty_reader(tab_state.id, Arc::clone(&terminal), hwnd);

    // Create tab entry
    let entry = TabEntry {
        id: tab_state.id,
        title: tab_state.title.clone(),
        terminal,
        color: tab_state.color.clone(),
        has_bell: false,
        title_locked: tab_state.custom_title.is_some(),
        reader_handle: Some(reader_handle),
    };

    state.tabs.push(entry);
    state.active_tab_index = state.tabs.len() - 1;

    // Update tab bar
    state.tab_bar.add_tab(tab_state.id, &tab_state.title);
    state.tab_bar.set_active(tab_state.id);

    // Update next_tab_id to be higher than any restored tab
    let current_max = state.next_tab_id.load(Ordering::SeqCst);
    if tab_state.id >= current_max {
        state.next_tab_id.store(tab_state.id + 1, Ordering::SeqCst);
    }

    Ok(())
}

/// Start the PTY reader thread for a restored terminal
fn start_pty_reader(
    tab_id: u64,
    terminal: Arc<Mutex<Terminal>>,
    hwnd: HWND,
) -> thread::JoinHandle<()> {
    use cterm_core::term::TerminalEvent;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

    let hwnd_ptr = hwnd.0 as usize;

    // Clone the PTY reader handle
    let pty_reader = {
        let term = terminal.lock().unwrap();
        term.pty().and_then(|pty| pty.try_clone_reader().ok())
    };

    thread::spawn(move || {
        let Some(mut reader) = pty_reader else {
            log::error!("Failed to clone PTY reader for restored tab {}", tab_id);
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(hwnd_ptr as *mut _)),
                    crate::window::WM_APP_PTY_EXIT,
                    WPARAM(tab_id as usize),
                    LPARAM(0),
                );
            }
            return;
        };

        let mut buffer = [0u8; 8192];

        loop {
            // Read from the cloned reader WITHOUT holding the terminal lock
            let bytes_read = {
                use std::io::Read;
                match reader.read(&mut buffer) {
                    Ok(0) => break, // EOF
                    Ok(n) => n,
                    Err(_) => break,
                }
            };

            // Process the data (briefly lock the terminal)
            {
                let mut term = terminal.lock().unwrap();
                let events = term.process(&buffer[..bytes_read]);

                // Handle events
                for event in events {
                    match event {
                        TerminalEvent::TitleChanged(_title) => unsafe {
                            let _ = PostMessageW(
                                Some(HWND(hwnd_ptr as *mut _)),
                                crate::window::WM_APP_TITLE_CHANGED,
                                WPARAM(tab_id as usize),
                                LPARAM(0),
                            );
                        },
                        TerminalEvent::Bell => unsafe {
                            let _ = PostMessageW(
                                Some(HWND(hwnd_ptr as *mut _)),
                                crate::window::WM_APP_BELL,
                                WPARAM(tab_id as usize),
                                LPARAM(0),
                            );
                        },
                        TerminalEvent::ProcessExited(_) => {
                            unsafe {
                                let _ = PostMessageW(
                                    Some(HWND(hwnd_ptr as *mut _)),
                                    crate::window::WM_APP_PTY_EXIT,
                                    WPARAM(tab_id as usize),
                                    LPARAM(0),
                                );
                            }
                            return;
                        }
                        _ => {}
                    }
                }
            }

            // Request redraw
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(hwnd_ptr as *mut _)),
                    crate::window::WM_APP_PTY_DATA,
                    WPARAM(tab_id as usize),
                    LPARAM(0),
                );
            }
        }

        // Process exited
        unsafe {
            let _ = PostMessageW(
                Some(HWND(hwnd_ptr as *mut _)),
                crate::window::WM_APP_PTY_EXIT,
                WPARAM(tab_id as usize),
                LPARAM(0),
            );
        }
    })
}
