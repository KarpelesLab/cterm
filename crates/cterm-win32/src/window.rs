//! Main window implementation
//!
//! Manages the main window, tabs, terminal rendering, and message handling.

use std::collections::HashMap;
use std::ptr;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use parking_lot::RwLock;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U;
use windows::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, HBRUSH, PAINTSTRUCT};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::Win32::UI::WindowsAndMessaging::*;

use cterm_app::config::Config;
use cterm_app::file_transfer::PendingFileManager;
use cterm_app::shortcuts::ShortcutManager;
use cterm_core::pty::{Pty, PtyConfig, PtySize};
use cterm_core::screen::{FileTransferOperation, ScreenConfig};
use cterm_core::term::{Terminal, TerminalEvent};
use cterm_ui::events::{Action, KeyCode, Modifiers};
use cterm_ui::theme::Theme;

use crate::clipboard;
use crate::dpi::{self, DpiInfo};
use crate::keycode;
use crate::menu::{self, MenuAction};
use crate::mouse::MouseState;
use crate::notification_bar::{NotificationAction, NotificationBar};
use crate::tab_bar::{TabBar, TAB_BAR_HEIGHT};
use crate::terminal_canvas::{CellDimensions, TerminalRenderer};

/// Custom window messages
pub const WM_APP_PTY_DATA: u32 = WM_APP + 1;
pub const WM_APP_PTY_EXIT: u32 = WM_APP + 2;
pub const WM_APP_TITLE_CHANGED: u32 = WM_APP + 3;
pub const WM_APP_BELL: u32 = WM_APP + 4;

/// Tab entry
pub struct TabEntry {
    pub id: u64,
    pub title: String,
    pub terminal: Arc<Mutex<Terminal>>,
    pub color: Option<String>,
    pub has_bell: bool,
    reader_handle: Option<thread::JoinHandle<()>>,
}

/// Window state
pub struct WindowState {
    pub hwnd: HWND,
    pub config: Config,
    pub theme: Theme,
    pub shortcuts: ShortcutManager,
    pub tabs: Vec<TabEntry>,
    pub active_tab_index: usize,
    pub next_tab_id: AtomicU64,
    pub renderer: Option<TerminalRenderer>,
    pub tab_bar: TabBar,
    pub notification_bar: NotificationBar,
    pub file_manager: PendingFileManager,
    pub dpi: DpiInfo,
    pub mouse_state: MouseState,
    menu_handle: winapi::shared::windef::HMENU,
}

impl WindowState {
    /// Create a new window state
    pub fn new(hwnd: HWND, config: &Config, theme: &Theme) -> Self {
        let shortcuts = ShortcutManager::from_config(&config.shortcuts);
        let dpi = DpiInfo::for_window(hwnd);

        let mut tab_bar = TabBar::new(theme);
        tab_bar.set_dpi(dpi);

        let mut notification_bar = NotificationBar::new(theme);
        notification_bar.set_dpi(dpi);

        // Create menu
        let menu_handle = menu::create_menu_bar(false);
        menu::set_window_menu(hwnd.0 as *mut _, menu_handle);

        Self {
            hwnd,
            config: config.clone(),
            theme: theme.clone(),
            shortcuts,
            tabs: Vec::new(),
            active_tab_index: 0,
            next_tab_id: AtomicU64::new(0),
            renderer: None,
            tab_bar,
            notification_bar,
            file_manager: PendingFileManager::new(),
            dpi,
            mouse_state: MouseState::new(),
            menu_handle,
        }
    }

    /// Initialize the renderer
    pub fn init_renderer(&mut self) -> windows::core::Result<()> {
        let font_family = &self.config.appearance.font.family;
        let font_size = self.config.appearance.font.size as f32;

        let renderer = TerminalRenderer::new(self.hwnd, &self.theme, font_family, font_size)?;
        self.renderer = Some(renderer);
        Ok(())
    }

    /// Create a new tab
    pub fn new_tab(&mut self) -> Result<u64, Box<dyn std::error::Error>> {
        let tab_id = self.next_tab_id.fetch_add(1, Ordering::SeqCst);

        // Get terminal size
        let (cols, rows) = self.terminal_size();

        // Create terminal
        let screen_config = ScreenConfig {
            scrollback_lines: self.config.general.scrollback_lines,
            ..Default::default()
        };

        let pty_config = PtyConfig {
            size: PtySize {
                cols: cols as u16,
                rows: rows as u16,
                pixel_width: 0,
                pixel_height: 0,
            },
            shell: self.config.general.default_shell.clone(),
            args: self.config.general.shell_args.clone(),
            cwd: self.config.general.working_directory.clone(),
            env: self
                .config
                .general
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        };

        let terminal = Terminal::with_shell(cols, rows, screen_config, &pty_config)?;
        let terminal = Arc::new(Mutex::new(terminal));

        // Start PTY reader thread
        let reader_handle = self.start_pty_reader(tab_id, Arc::clone(&terminal));

        let entry = TabEntry {
            id: tab_id,
            title: "Terminal".to_string(),
            terminal,
            color: None,
            has_bell: false,
            reader_handle: Some(reader_handle),
        };

        self.tabs.push(entry);
        self.active_tab_index = self.tabs.len() - 1;

        // Update tab bar
        self.tab_bar.add_tab(tab_id, "Terminal");
        self.tab_bar.set_active(tab_id);

        Ok(tab_id)
    }

    /// Start the PTY reader thread
    fn start_pty_reader(
        &self,
        tab_id: u64,
        terminal: Arc<Mutex<Terminal>>,
    ) -> thread::JoinHandle<()> {
        let hwnd = self.hwnd.0 as usize;

        thread::spawn(move || {
            let mut buffer = [0u8; 8192];

            loop {
                // Try to read from PTY
                let bytes_read = {
                    let mut term = terminal.lock().unwrap();
                    if let Some(pty) = term.pty_mut() {
                        match pty.read(&mut buffer) {
                            Ok(0) => break, // EOF
                            Ok(n) => n,
                            Err(_) => break,
                        }
                    } else {
                        break;
                    }
                };

                // Process the data
                {
                    let mut term = terminal.lock().unwrap();
                    let events = term.process(&buffer[..bytes_read]);

                    // Handle events
                    for event in events {
                        match event {
                            TerminalEvent::TitleChanged(_title) => {
                                // Post title change message
                                // Note: We'd need to pass the title somehow
                                unsafe {
                                    PostMessageW(
                                        HWND(hwnd as *mut _),
                                        WM_APP_TITLE_CHANGED,
                                        WPARAM(tab_id as usize),
                                        LPARAM(0),
                                    );
                                }
                            }
                            TerminalEvent::Bell => unsafe {
                                PostMessageW(
                                    HWND(hwnd as *mut _),
                                    WM_APP_BELL,
                                    WPARAM(tab_id as usize),
                                    LPARAM(0),
                                );
                            },
                            TerminalEvent::ProcessExited(_) => {
                                unsafe {
                                    PostMessageW(
                                        HWND(hwnd as *mut _),
                                        WM_APP_PTY_EXIT,
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
                    PostMessageW(
                        HWND(hwnd as *mut _),
                        WM_APP_PTY_DATA,
                        WPARAM(tab_id as usize),
                        LPARAM(0),
                    );
                }
            }

            // Process exited
            unsafe {
                PostMessageW(
                    HWND(hwnd as *mut _),
                    WM_APP_PTY_EXIT,
                    WPARAM(tab_id as usize),
                    LPARAM(0),
                );
            }
        })
    }

    /// Close a tab
    pub fn close_tab(&mut self, tab_id: u64) {
        if let Some(index) = self.tabs.iter().position(|t| t.id == tab_id) {
            self.tabs.remove(index);
            self.tab_bar.remove_tab(tab_id);

            if self.tabs.is_empty() {
                // Close window
                unsafe { PostMessageW(self.hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)) };
            } else {
                // Adjust active tab index
                if self.active_tab_index >= self.tabs.len() {
                    self.active_tab_index = self.tabs.len() - 1;
                }
                let new_active_id = self.tabs[self.active_tab_index].id;
                self.tab_bar.set_active(new_active_id);
            }
        }
    }

    /// Switch to tab
    pub fn switch_to_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_tab_index = index;
            let tab_id = self.tabs[index].id;
            self.tab_bar.set_active(tab_id);
            self.tab_bar.clear_bell(tab_id);
            self.tabs[index].has_bell = false;
            self.invalidate();
        }
    }

    /// Switch to next tab
    pub fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            let next = (self.active_tab_index + 1) % self.tabs.len();
            self.switch_to_tab(next);
        }
    }

    /// Switch to previous tab
    pub fn prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            let prev = if self.active_tab_index == 0 {
                self.tabs.len() - 1
            } else {
                self.active_tab_index - 1
            };
            self.switch_to_tab(prev);
        }
    }

    /// Get the active terminal
    pub fn active_terminal(&self) -> Option<Arc<Mutex<Terminal>>> {
        self.tabs
            .get(self.active_tab_index)
            .map(|t| Arc::clone(&t.terminal))
    }

    /// Get terminal size in cells
    pub fn terminal_size(&self) -> (usize, usize) {
        let mut rect = RECT::default();
        unsafe { GetClientRect(self.hwnd, &mut rect).ok() };

        let width = (rect.right - rect.left) as u32;
        let height = (rect.bottom - rect.top) as u32;

        // Subtract chrome heights
        let tab_bar_height = self.tab_bar.height() as u32;
        let notification_bar_height = self.notification_bar.height() as u32;
        let terminal_height = height.saturating_sub(tab_bar_height + notification_bar_height);

        if let Some(ref renderer) = self.renderer {
            renderer.terminal_size(width, terminal_height)
        } else {
            (80, 24)
        }
    }

    /// Handle window resize
    pub fn on_resize(&mut self, width: u32, height: u32) {
        if let Some(ref mut renderer) = self.renderer {
            renderer.resize(width, height).ok();
        }

        // Resize all terminals
        let (cols, rows) = self.terminal_size();
        for tab in &self.tabs {
            let mut term = tab.terminal.lock().unwrap();
            term.resize(cols, rows);
        }
    }

    /// Handle DPI change
    pub fn on_dpi_changed(&mut self, dpi: u32) {
        self.dpi = DpiInfo::from_dpi(dpi);
        self.tab_bar.set_dpi(self.dpi);
        self.notification_bar.set_dpi(self.dpi);

        if let Some(ref mut renderer) = self.renderer {
            renderer.update_dpi(dpi).ok();
        }
    }

    /// Invalidate and request redraw
    pub fn invalidate(&self) {
        unsafe { InvalidateRect(self.hwnd, None, false) };
    }

    /// Render the window
    pub fn render(&mut self) -> windows::core::Result<()> {
        if self.renderer.is_none() {
            return Ok(());
        }

        // Get the active terminal first (before borrowing renderer)
        let terminal = self.active_terminal();

        // Render active terminal
        if let Some(terminal) = terminal {
            let term = terminal.lock().unwrap();
            // Now get the renderer and render
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.render(term.screen())?;
            }
        }

        Ok(())
    }

    /// Handle keyboard input
    pub fn on_key_down(&mut self, vk: u16, _scancode: u16) -> bool {
        let modifiers = keycode::get_modifiers();

        // Check for shortcuts first
        if let Some(key) = keycode::vk_to_keycode(vk) {
            if let Some(action) = self.shortcuts.match_event(key, modifiers) {
                self.handle_action(action.clone());
                return true;
            }
        }

        // Check modifier-only keys
        if keycode::is_modifier_key(vk) {
            return false;
        }

        // Send to terminal
        if let Some(terminal) = self.active_terminal() {
            let mut term = terminal.lock().unwrap();
            let app_cursor = term.screen().modes.application_cursor;

            // Get terminal sequence for special keys
            if let Some(seq) = keycode::vk_to_terminal_seq(vk, modifiers, app_cursor) {
                term.write(seq.as_bytes()).ok();
                self.invalidate();
                return true;
            }
        }

        false
    }

    /// Handle character input
    pub fn on_char(&mut self, c: char) {
        if let Some(terminal) = self.active_terminal() {
            let mut term = terminal.lock().unwrap();
            let mut buf = [0u8; 4];
            let s = c.encode_utf8(&mut buf);
            term.write(s.as_bytes()).ok();
            self.invalidate();
        }
    }

    /// Handle an action
    fn handle_action(&mut self, action: Action) {
        match action {
            Action::NewTab => {
                self.new_tab().ok();
                self.invalidate();
            }
            Action::CloseTab => {
                if let Some(tab) = self.tabs.get(self.active_tab_index) {
                    let id = tab.id;
                    self.close_tab(id);
                }
            }
            Action::NextTab => self.next_tab(),
            Action::PrevTab => self.prev_tab(),
            Action::Tab(n) => {
                let idx = (n as usize).saturating_sub(1);
                self.switch_to_tab(idx);
            }
            Action::Copy => self.copy_selection(),
            Action::Paste => self.paste(),
            Action::ZoomIn => {
                // TODO: Implement zoom
            }
            Action::ZoomOut => {
                // TODO: Implement zoom
            }
            Action::ZoomReset => {
                // TODO: Implement zoom reset
            }
            Action::CloseWindow => {
                unsafe { PostMessageW(self.hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)) };
            }
            Action::NewWindow => {
                // TODO: Implement new window
            }
            Action::FindText => {
                // TODO: Show find dialog
            }
            Action::ResetTerminal => {
                if let Some(terminal) = self.active_terminal() {
                    let mut term = terminal.lock().unwrap();
                    term.screen_mut().reset();
                    self.invalidate();
                }
            }
            _ => {}
        }
    }

    /// Handle menu command
    pub fn on_menu_command(&mut self, cmd: u16) {
        if let Some(action) = MenuAction::from_id(cmd) {
            match action {
                MenuAction::NewTab => {
                    self.new_tab().ok();
                }
                MenuAction::NewWindow => { /* TODO */ }
                MenuAction::CloseTab => {
                    if let Some(tab) = self.tabs.get(self.active_tab_index) {
                        let id = tab.id;
                        self.close_tab(id);
                    }
                }
                MenuAction::CloseOtherTabs => {
                    // Close all but active
                    let active_id = self.tabs.get(self.active_tab_index).map(|t| t.id);
                    if let Some(active_id) = active_id {
                        let ids: Vec<_> = self
                            .tabs
                            .iter()
                            .filter(|t| t.id != active_id)
                            .map(|t| t.id)
                            .collect();
                        for id in ids {
                            self.close_tab(id);
                        }
                    }
                }
                MenuAction::Quit => {
                    unsafe { PostMessageW(self.hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)) };
                }
                MenuAction::Copy => self.copy_selection(),
                MenuAction::Paste => self.paste(),
                MenuAction::PrevTab => self.prev_tab(),
                MenuAction::NextTab => self.next_tab(),
                MenuAction::Tab1 => self.switch_to_tab(0),
                MenuAction::Tab2 => self.switch_to_tab(1),
                MenuAction::Tab3 => self.switch_to_tab(2),
                MenuAction::Tab4 => self.switch_to_tab(3),
                MenuAction::Tab5 => self.switch_to_tab(4),
                MenuAction::Tab6 => self.switch_to_tab(5),
                MenuAction::Tab7 => self.switch_to_tab(6),
                MenuAction::Tab8 => self.switch_to_tab(7),
                MenuAction::Tab9 => self.switch_to_tab(8),
                MenuAction::About => {
                    crate::dialogs::show_about_dialog(self.hwnd.0 as *mut _);
                }
                MenuAction::Reset => {
                    if let Some(terminal) = self.active_terminal() {
                        let mut term = terminal.lock().unwrap();
                        term.screen_mut().reset();
                        self.invalidate();
                    }
                }
                MenuAction::ClearReset => {
                    if let Some(terminal) = self.active_terminal() {
                        let mut term = terminal.lock().unwrap();
                        // reset() clears scrollback as well as screen
                        term.screen_mut().reset();
                        self.invalidate();
                    }
                }
                _ => {}
            }
        }
    }

    /// Copy selection to clipboard
    fn copy_selection(&mut self) {
        if let Some(terminal) = self.active_terminal() {
            let term = terminal.lock().unwrap();
            if let Some(text) = term.screen().get_selected_text() {
                clipboard::copy_to_clipboard(&text).ok();
            }
        }
    }

    /// Paste from clipboard
    fn paste(&mut self) {
        if let Ok(text) = clipboard::paste_from_clipboard() {
            if let Some(terminal) = self.active_terminal() {
                let mut term = terminal.lock().unwrap();
                term.write(text.as_bytes()).ok();
                self.invalidate();
            }
        }
    }

    /// Handle PTY data received
    pub fn on_pty_data(&mut self, tab_id: u64) {
        // Check for file transfers from the terminal
        if let Some(tab) = self.tabs.iter().find(|t| t.id == tab_id) {
            if let Ok(mut terminal) = tab.terminal.lock() {
                let transfers = terminal.screen_mut().take_file_transfers();
                for transfer in transfers {
                    match transfer {
                        FileTransferOperation::FileReceived { id, name, data } => {
                            log::info!(
                                "File received: id={}, name={:?}, size={}",
                                id,
                                name,
                                data.len()
                            );
                            let size = data.len();
                            self.file_manager.set_pending(id, name.clone(), data);
                            self.notification_bar.show_file(id, name.as_deref(), size);
                        }
                        FileTransferOperation::StreamingFileReceived { id, result } => {
                            log::info!(
                                "Streaming file received: id={}, name={:?}, size={}",
                                id,
                                result.params.name,
                                result.total_bytes
                            );
                            let size = result.total_bytes;
                            let name = result.params.name.clone();
                            self.file_manager
                                .set_pending_streaming(id, name.clone(), result.data);
                            self.notification_bar.show_file(id, name.as_deref(), size);
                        }
                    }
                }
            }
        }

        // Invalidate to redraw
        self.invalidate();
    }

    /// Handle PTY exit
    pub fn on_pty_exit(&mut self, tab_id: u64) {
        self.close_tab(tab_id);
    }

    /// Handle bell
    pub fn on_bell(&mut self, tab_id: u64) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.has_bell = true;
            self.tab_bar.set_bell(tab_id, true);
        }
    }

    /// Handle mouse down
    pub fn on_mouse_down(&mut self, x: f32, y: f32) {
        // Check if click is in notification bar area
        let tab_bar_height = self.dpi.scale_f32(TAB_BAR_HEIGHT as f32);
        let notification_height = self.notification_bar.height() as f32;

        // Notification bar is right below tab bar
        if y >= tab_bar_height && y < tab_bar_height + notification_height {
            // Adjust y coordinate relative to notification bar
            let rel_y = y - tab_bar_height;
            if let Some(action) = self.notification_bar.hit_test(x, rel_y) {
                self.handle_notification_action(action);
            }
        }
    }

    /// Handle notification bar action
    fn handle_notification_action(&mut self, action: NotificationAction) {
        if let Some(file_id) = self.notification_bar.pending_file_id() {
            match action {
                NotificationAction::Save => {
                    self.save_file(file_id, false);
                }
                NotificationAction::SaveAs => {
                    self.save_file(file_id, true);
                }
                NotificationAction::Discard => {
                    self.file_manager.discard(file_id);
                    self.notification_bar.hide();
                    self.invalidate();
                }
            }
        }
    }

    /// Save file (optionally with dialog)
    fn save_file(&mut self, file_id: u64, show_dialog: bool) {
        // Get default path from file manager
        let default_path = self.file_manager.default_save_path();

        let save_path = if show_dialog {
            // Show save dialog - need a path or empty path
            if let Some(ref path) = default_path {
                crate::dialogs::show_save_dialog(self.hwnd, path)
            } else {
                crate::dialogs::show_save_dialog(self.hwnd, std::path::Path::new("download"))
            }
        } else {
            default_path
        };

        if let Some(path) = save_path {
            match self.file_manager.save_to_path(file_id, &path) {
                Ok(_size) => {
                    log::info!("File saved to {:?}", path);
                }
                Err(e) => {
                    log::error!("Failed to save file: {}", e);
                    crate::dialogs::show_error_msg(
                        self.hwnd,
                        &format!("Failed to save file: {}", e),
                    );
                }
            }
        }

        self.notification_bar.hide();
        self.invalidate();
    }
}

/// Window class name
pub const WINDOW_CLASS: &str = "ctermWindow";

/// Register the window class
pub fn register_window_class() -> windows::core::Result<()> {
    let class_name: Vec<u16> = WINDOW_CLASS
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC,
        lpfnWndProc: Some(window_proc),
        cbClsExtra: 0,
        cbWndExtra: std::mem::size_of::<*mut WindowState>() as i32,
        hInstance: unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None)? }.into(),
        hIcon: HICON::default(),
        hCursor: unsafe { LoadCursorW(None, IDC_IBEAM)? },
        hbrBackground: HBRUSH::default(),
        lpszMenuName: PCWSTR::null(),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        hIconSm: HICON::default(),
    };

    let atom = unsafe { RegisterClassExW(&wc) };
    if atom == 0 {
        return Err(windows::core::Error::from_win32());
    }

    Ok(())
}

/// Create the main window
pub fn create_window(config: &Config, theme: &Theme) -> windows::core::Result<HWND> {
    let class_name: Vec<u16> = WINDOW_CLASS
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let title: Vec<u16> = "cterm".encode_utf16().chain(std::iter::once(0)).collect();

    let dpi = dpi::get_system_dpi();
    let width = dpi::scale_by_dpi(800, dpi);
    let height = dpi::scale_by_dpi(600, dpi);

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            width,
            height,
            None,
            None,
            None,
            None,
        )?
    };

    // Create window state
    let mut state = Box::new(WindowState::new(hwnd, config, theme));
    state.init_renderer()?;
    state.new_tab().map_err(|e| {
        log::error!("Failed to create initial tab: {}", e);
        windows::core::Error::from_win32()
    })?;

    // Store state pointer in window
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
    }

    Ok(hwnd)
}

/// Window procedure
extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Get window state
    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WindowState;

    if state_ptr.is_null() {
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }

    let state = unsafe { &mut *state_ptr };

    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            unsafe { BeginPaint(hwnd, &mut ps) };
            state.render().ok();
            unsafe { EndPaint(hwnd, &ps) };
            LRESULT(0)
        }

        WM_SIZE => {
            let width = (lparam.0 & 0xFFFF) as u32;
            let height = ((lparam.0 >> 16) & 0xFFFF) as u32;
            state.on_resize(width, height);
            LRESULT(0)
        }

        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            state.on_dpi_changed(dpi);
            // Resize window to suggested rect
            let rect = unsafe { &*(lparam.0 as *const RECT) };
            unsafe {
                SetWindowPos(
                    hwnd,
                    None,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
            }
            .ok();
            LRESULT(0)
        }

        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = (wparam.0 & 0xFFFF) as u16;
            let scancode = ((lparam.0 >> 16) & 0xFF) as u16;
            if state.on_key_down(vk, scancode) {
                LRESULT(0)
            } else {
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }

        WM_CHAR => {
            if let Some(c) = char::from_u32(wparam.0 as u32) {
                if !c.is_control() || c == '\r' || c == '\t' || c == '\x08' {
                    state.on_char(c);
                }
            }
            LRESULT(0)
        }

        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as f32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
            state.on_mouse_down(x, y);
            LRESULT(0)
        }

        WM_COMMAND => {
            let cmd = (wparam.0 & 0xFFFF) as u16;
            state.on_menu_command(cmd);
            LRESULT(0)
        }

        WM_APP_PTY_DATA => {
            let tab_id = wparam.0 as u64;
            state.on_pty_data(tab_id);
            LRESULT(0)
        }

        WM_APP_PTY_EXIT => {
            let tab_id = wparam.0 as u64;
            state.on_pty_exit(tab_id);
            LRESULT(0)
        }

        WM_APP_BELL => {
            let tab_id = wparam.0 as u64;
            state.on_bell(tab_id);
            LRESULT(0)
        }

        WM_DESTROY => {
            // Clean up
            let state = unsafe { Box::from_raw(state_ptr) };
            drop(state);
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) };
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }

        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
