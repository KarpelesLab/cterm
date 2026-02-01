//! Main window implementation
//!
//! Manages the main window, tabs, terminal rendering, and message handling.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, UpdateWindow, HBRUSH, PAINTSTRUCT,
};
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::Win32::UI::WindowsAndMessaging::*;

use cterm_app::config::Config;
use cterm_app::file_transfer::PendingFileManager;
use cterm_app::shortcuts::ShortcutManager;
use cterm_core::color::Rgb;
use cterm_core::pty::{PtyConfig, PtySize};
use cterm_core::screen::{FileTransferOperation, ScreenConfig};
use cterm_core::term::{Terminal, TerminalEvent};
use cterm_ui::events::{Action, Modifiers};
use cterm_ui::theme::Theme;

use crate::clipboard;
use crate::dpi::{self, DpiInfo};
use crate::keycode;
use crate::menu::{self, MenuAction};
use crate::mouse::MouseState;
use crate::notification_bar::{NotificationAction, NotificationBar};
use crate::tab_bar::{TabBar, TAB_BAR_HEIGHT};
use crate::terminal_canvas::TerminalRenderer;

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
    /// Whether title was explicitly set (locks out OSC updates)
    pub title_locked: bool,
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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
            term: self.config.general.term.clone(),
        };

        let terminal = Terminal::with_shell(cols, rows, screen_config, &pty_config)?;
        let terminal = Arc::new(Mutex::new(terminal));

        // Get shell basename for initial title
        let shell = self
            .config
            .general
            .default_shell
            .clone()
            .unwrap_or_else(|| std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string()));
        let initial_title = std::path::Path::new(&shell)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Terminal")
            .to_string();

        // Start PTY reader thread
        let reader_handle = self.start_pty_reader(tab_id, Arc::clone(&terminal));

        let entry = TabEntry {
            id: tab_id,
            title: initial_title.clone(),
            terminal,
            color: None,
            has_bell: false,
            title_locked: false,
            reader_handle: Some(reader_handle),
        };

        self.tabs.push(entry);
        self.active_tab_index = self.tabs.len() - 1;

        // Update tab bar with shell basename
        self.tab_bar.add_tab(tab_id, &initial_title);
        self.tab_bar.set_active(tab_id);

        Ok(tab_id)
    }

    /// Create a new tab from a template
    pub fn new_tab_from_template(
        &mut self,
        template: &cterm_app::config::StickyTabConfig,
    ) -> Result<u64, Box<dyn std::error::Error>> {
        let tab_id = self.next_tab_id.fetch_add(1, Ordering::SeqCst);

        // Get terminal size
        let (cols, rows) = self.terminal_size();

        // Create terminal
        let screen_config = ScreenConfig {
            scrollback_lines: self.config.general.scrollback_lines,
        };

        // Build the shell command and args from the template
        let (shell, args) = if let Some(ref docker) = template.docker {
            // Docker tab
            match docker.mode {
                cterm_app::config::DockerMode::Exec => {
                    // Docker exec into container
                    let container = docker.container.clone().unwrap_or_default();
                    let shell_cmd = docker
                        .shell
                        .clone()
                        .unwrap_or_else(|| "/bin/sh".to_string());
                    (
                        Some("docker".to_string()),
                        vec!["exec".to_string(), "-it".to_string(), container, shell_cmd],
                    )
                }
                cterm_app::config::DockerMode::Run
                | cterm_app::config::DockerMode::DevContainer => {
                    // Docker run image
                    let image = docker.image.clone().unwrap_or_else(|| "ubuntu".to_string());
                    let mut run_args = vec!["run".to_string(), "-it".to_string()];
                    if docker.auto_remove {
                        run_args.push("--rm".to_string());
                    }
                    run_args.push(image);
                    (Some("docker".to_string()), run_args)
                }
            }
        } else if let Some(ref cmd) = template.command {
            // Use template command
            (Some(cmd.clone()), template.args.clone())
        } else {
            // Use default shell
            (self.config.general.default_shell.clone(), Vec::new())
        };

        let pty_config = PtyConfig {
            size: PtySize {
                cols: cols as u16,
                rows: rows as u16,
                pixel_width: 0,
                pixel_height: 0,
            },
            shell,
            args,
            cwd: template
                .working_directory
                .clone()
                .or_else(|| self.config.general.working_directory.clone()),
            env: template
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .chain(
                    self.config
                        .general
                        .env
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone())),
                )
                .collect(),
            term: self.config.general.term.clone(),
        };

        let terminal = Terminal::with_shell(cols, rows, screen_config, &pty_config)?;
        let terminal = Arc::new(Mutex::new(terminal));

        // Start PTY reader thread
        let reader_handle = self.start_pty_reader(tab_id, Arc::clone(&terminal));

        let entry = TabEntry {
            id: tab_id,
            title: template.name.clone(),
            terminal,
            color: template.color.clone(),
            has_bell: false,
            title_locked: true, // Lock title for template tabs
            reader_handle: Some(reader_handle),
        };

        self.tabs.push(entry);
        self.active_tab_index = self.tabs.len() - 1;

        // Update tab bar
        self.tab_bar.add_tab(tab_id, &template.name);
        self.tab_bar.set_active(tab_id);

        // Set tab color if specified
        if let Some(ref color_hex) = template.color {
            let rgb = parse_hex_color(color_hex);
            self.tab_bar.set_color(tab_id, rgb);
        }

        self.invalidate();

        Ok(tab_id)
    }

    /// Start the PTY reader thread
    fn start_pty_reader(
        &self,
        tab_id: u64,
        terminal: Arc<Mutex<Terminal>>,
    ) -> thread::JoinHandle<()> {
        let hwnd = self.hwnd.0 as usize;

        // Clone the PTY reader handle so we can read without holding the terminal lock.
        // This is critical: pty.read() is blocking I/O, and holding the mutex during
        // the read would prevent the UI thread from rendering or handling input.
        let pty_reader = {
            let term = terminal.lock().unwrap();
            term.pty().and_then(|pty| pty.try_clone_reader().ok())
        };

        thread::spawn(move || {
            let Some(mut reader) = pty_reader else {
                log::error!("Failed to clone PTY reader for tab {}", tab_id);
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd as *mut _)),
                        WM_APP_PTY_EXIT,
                        WPARAM(tab_id as usize),
                        LPARAM(0),
                    );
                }
                return;
            };

            let mut buffer = [0u8; 8192];

            loop {
                // Read from the cloned reader WITHOUT holding the terminal lock.
                // This allows the UI thread to render and handle input concurrently.
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
                            TerminalEvent::TitleChanged(_title) => {
                                // Post title change message
                                // Note: We'd need to pass the title somehow
                                unsafe {
                                    let _ = PostMessageW(
                                        Some(HWND(hwnd as *mut _)),
                                        WM_APP_TITLE_CHANGED,
                                        WPARAM(tab_id as usize),
                                        LPARAM(0),
                                    );
                                }
                            }
                            TerminalEvent::Bell => unsafe {
                                let _ = PostMessageW(
                                    Some(HWND(hwnd as *mut _)),
                                    WM_APP_BELL,
                                    WPARAM(tab_id as usize),
                                    LPARAM(0),
                                );
                            },
                            TerminalEvent::ProcessExited(_) => {
                                unsafe {
                                    let _ = PostMessageW(
                                        Some(HWND(hwnd as *mut _)),
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
                    let _ = PostMessageW(
                        Some(HWND(hwnd as *mut _)),
                        WM_APP_PTY_DATA,
                        WPARAM(tab_id as usize),
                        LPARAM(0),
                    );
                }
            }

            // Process exited
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(hwnd as *mut _)),
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
                unsafe {
                    let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                };
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

    /// Send focus event to the active terminal if focus events mode is enabled (DECSET 1004)
    /// `focused`: true for focus in (\x1b[I), false for focus out (\x1b[O)
    pub fn send_focus_event(&self, focused: bool) {
        if let Some(terminal) = self.active_terminal() {
            let mut term = terminal.lock().unwrap();
            if term.screen().modes.focus_events {
                let sequence = if focused { b"\x1b[I" } else { b"\x1b[O" };
                if let Err(e) = term.write(sequence) {
                    log::error!("Failed to send focus event: {}", e);
                }
            }
        }
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
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
            // Force immediate repaint - without UpdateWindow, WM_PAINT may be
            // deferred until the message queue is empty, causing blank terminal
            let _ = UpdateWindow(self.hwnd);
        };
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
                // Drop the lock before invalidate() — UpdateWindow dispatches WM_PAINT
                // synchronously, and render() needs to lock the terminal.
                drop(term);
                self.invalidate();
                return true;
            }

            // Ctrl+letter → send control character (Ctrl+A=0x01 .. Ctrl+Z=0x1a)
            // We handle this here rather than in WM_CHAR to keep all terminal
            // input in one place and avoid double-send issues.
            if modifiers.contains(Modifiers::CTRL)
                && !modifiers.contains(Modifiers::ALT)
                && (0x41..=0x5A).contains(&(vk as i32))
            {
                let ctrl_char = (vk as u8) - b'A' + 1;
                term.write(&[ctrl_char]).ok();
                drop(term);
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
            // Drop the lock before invalidate() — UpdateWindow dispatches WM_PAINT
            // synchronously, and render() needs to lock the terminal.
            drop(term);
        }
        self.invalidate();
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
                unsafe {
                    let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                };
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
                    drop(term);
                }
                self.invalidate();
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
                MenuAction::NewWindow => {
                    // Launch a new instance of the application
                    if let Ok(exe) = std::env::current_exe() {
                        std::process::Command::new(exe).spawn().ok();
                    }
                }
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
                MenuAction::QuickOpen => {
                    // Show Quick Open dialog
                    let templates = cterm_app::load_sticky_tabs().unwrap_or_default();
                    if let Some(template) = crate::quick_open::show_quick_open(self.hwnd, templates)
                    {
                        // Create a new tab with the selected template
                        log::info!("Quick open selected: {}", template.name);
                        self.new_tab_from_template(&template).ok();
                    }
                }
                MenuAction::DockerPicker => {
                    // Show Docker picker dialog
                    if let Some(selection) =
                        crate::docker_dialog::show_docker_picker(self.hwnd.0 as *mut _)
                    {
                        // Create a new tab with the selected Docker configuration
                        match selection {
                            crate::docker_dialog::DockerSelection::ExecContainer(container) => {
                                log::info!("Docker exec into container: {}", container.name);
                                // TODO: Create new tab with docker exec command
                            }
                            crate::docker_dialog::DockerSelection::RunImage(image) => {
                                log::info!("Docker run image: {}:{}", image.repository, image.tag);
                                // TODO: Create new tab with docker run command
                            }
                        }
                    }
                }
                MenuAction::Quit => {
                    unsafe {
                        let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    };
                }
                MenuAction::Copy => self.copy_selection(),
                MenuAction::CopyHtml => self.copy_selection_as_html(),
                MenuAction::Paste => self.paste(),
                MenuAction::SelectAll => self.select_all(),
                MenuAction::ZoomIn => self.zoom_in(),
                MenuAction::ZoomOut => self.zoom_out(),
                MenuAction::ZoomReset => self.zoom_reset(),
                MenuAction::Fullscreen => self.toggle_fullscreen(),
                MenuAction::SetTitle => self.show_set_title_dialog(),
                MenuAction::SetColor => self.show_set_color_dialog(),
                MenuAction::Find => self.show_find_dialog(),
                MenuAction::Reset => {
                    if let Some(terminal) = self.active_terminal() {
                        let mut term = terminal.lock().unwrap();
                        term.screen_mut().reset();
                        drop(term);
                    }
                    self.invalidate();
                }
                MenuAction::ClearReset => {
                    if let Some(terminal) = self.active_terminal() {
                        let mut term = terminal.lock().unwrap();
                        term.screen_mut().reset();
                        drop(term);
                    }
                    self.invalidate();
                }
                MenuAction::SendSignalInt => self.send_signal(2), // SIGINT
                MenuAction::SendSignalKill => self.send_signal(9), // SIGKILL
                MenuAction::SendSignalHup => self.send_signal(1), // SIGHUP
                MenuAction::SendSignalTerm => self.send_signal(15), // SIGTERM
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
                MenuAction::Preferences => {
                    if crate::preferences_dialog::show_preferences_dialog(self.hwnd.0 as *mut _) {
                        // Reload config and apply changes
                        if let Ok(config) = cterm_app::load_config() {
                            self.config = config;
                            // TODO: Apply theme and other changes without restart
                            log::info!("Preferences saved and reloaded");
                        }
                    }
                }
                MenuAction::TabTemplates => {
                    if crate::templates_dialog::show_templates_dialog(self.hwnd.0 as *mut _) {
                        log::info!("Tab templates saved");
                    }
                }
                MenuAction::CheckUpdates => {
                    crate::dialogs::show_check_updates_dialog(self.hwnd.0 as *mut _);
                }
                MenuAction::About => {
                    crate::dialogs::show_about_dialog(self.hwnd.0 as *mut _);
                }
                MenuAction::DebugRelaunch => {
                    // Re-launch the application (for testing upgrade)
                    if let Ok(exe) = std::env::current_exe() {
                        std::process::Command::new(exe).spawn().ok();
                    }
                    unsafe {
                        let _ = PostMessageW(Some(self.hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    };
                }
                MenuAction::DebugDumpState => {
                    log::info!("=== Debug State Dump ===");
                    log::info!("Tabs: {}", self.tabs.len());
                    log::info!("Active tab: {}", self.active_tab_index);
                    for (i, tab) in self.tabs.iter().enumerate() {
                        log::info!("  Tab {}: id={}, title={}", i, tab.id, tab.title);
                    }
                    log::info!("DPI: {:?}", self.dpi);
                    log::info!("========================");
                }
                MenuAction::ViewLogs => {
                    // Show the in-app log viewer
                    crate::log_viewer::show_log_viewer(self.hwnd.0 as *mut _);
                }
            }
        }
    }

    /// Show set title dialog
    fn show_set_title_dialog(&mut self) {
        if let Some(tab) = self.tabs.get(self.active_tab_index) {
            let current_title = tab.title.clone();
            if let Some(new_title) =
                crate::dialogs::show_set_title_dialog(self.hwnd.0 as *mut _, &current_title)
            {
                let tab_id = tab.id;
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                    tab.title = new_title.clone();
                    tab.title_locked = true;
                    self.tab_bar.set_title(tab_id, &new_title);
                    self.invalidate();
                }
            }
        }
    }

    /// Show set color dialog
    fn show_set_color_dialog(&mut self) {
        if let Some(tab) = self.tabs.get(self.active_tab_index) {
            let tab_id = tab.id;
            if let Some(color_result) = crate::dialogs::show_set_color_dialog(self.hwnd.0 as *mut _)
            {
                if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                    tab.color = color_result.clone();
                    // Parse color to Rgb
                    let rgb = color_result.and_then(|c| parse_hex_color(&c));
                    self.tab_bar.set_color(tab_id, rgb);
                    self.invalidate();
                }
            }
        }
    }

    /// Show find dialog
    fn show_find_dialog(&mut self) {
        if let Some(options) = crate::dialogs::show_find_dialog(self.hwnd.0 as *mut _) {
            // Perform search in terminal
            if let Some(terminal) = self.active_terminal() {
                let term = terminal.lock().unwrap();
                let results =
                    term.screen()
                        .find(&options.text, options.case_sensitive, options.regex);
                if !results.is_empty() {
                    log::info!("Found {} matches for: {}", results.len(), options.text);
                    // TODO: Highlight first result and allow navigation
                } else {
                    crate::dialogs::show_message(
                        self.hwnd.0 as *mut _,
                        "Find",
                        &format!("'{}' not found", options.text),
                        winapi::um::winuser::MB_OK | winapi::um::winuser::MB_ICONINFORMATION,
                    );
                }
                drop(term);
            }
            self.invalidate();
        }
    }

    /// Select all text in the terminal
    fn select_all(&mut self) {
        if let Some(terminal) = self.active_terminal() {
            let mut term = terminal.lock().unwrap();
            let screen = term.screen_mut();
            let total_lines = screen.total_lines();
            if total_lines > 0 {
                // Select from first line to last line
                screen.start_selection(0, 0, cterm_core::screen::SelectionMode::Char);
                // Extend to end - use a large column value for last line
                screen.extend_selection(total_lines.saturating_sub(1), usize::MAX);
            }
            drop(term);
        }
        self.invalidate();
    }

    /// Copy selection as HTML (for now, just copy as plain text with formatting note)
    fn copy_selection_as_html(&mut self) {
        if let Some(terminal) = self.active_terminal() {
            let term = terminal.lock().unwrap();
            if let Some(text) = term.screen().get_selected_text() {
                // For now, copy as plain text
                // TODO: Implement HTML conversion with ANSI colors
                clipboard::copy_to_clipboard(&text).ok();
            }
        }
    }

    /// Send a signal to the active terminal's process
    fn send_signal(&mut self, _signal: i32) {
        // On Windows, signals work differently than Unix
        // For now, we'll send a Ctrl+C equivalent for SIGINT
        if let Some(terminal) = self.active_terminal() {
            let mut term = terminal.lock().unwrap();
            // Send Ctrl+C character
            term.write(&[0x03]).ok(); // ETX (Ctrl+C)
            drop(term);
        }
        self.invalidate();
    }

    /// Zoom in (increase font size)
    fn zoom_in(&mut self) {
        if let Some(ref mut renderer) = self.renderer {
            let new_size = renderer.font_size() + 1.0;
            if new_size <= 72.0 {
                renderer.set_font_size(new_size).ok();
                self.on_font_size_changed();
            }
        }
    }

    /// Zoom out (decrease font size)
    fn zoom_out(&mut self) {
        if let Some(ref mut renderer) = self.renderer {
            let new_size = renderer.font_size() - 1.0;
            if new_size >= 6.0 {
                renderer.set_font_size(new_size).ok();
                self.on_font_size_changed();
            }
        }
    }

    /// Reset zoom to default
    fn zoom_reset(&mut self) {
        if let Some(ref mut renderer) = self.renderer {
            let default_size = self.config.appearance.font.size as f32;
            renderer.set_font_size(default_size).ok();
            self.on_font_size_changed();
        }
    }

    /// Called when font size changes to resize terminals
    fn on_font_size_changed(&mut self) {
        let (cols, rows) = self.terminal_size();
        for tab in &self.tabs {
            let mut term = tab.terminal.lock().unwrap();
            term.resize(cols, rows);
        }
        self.invalidate();
    }

    /// Toggle fullscreen mode
    fn toggle_fullscreen(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::{
            GetWindowLongW, SetWindowLongW, SetWindowPos, ShowWindow, GWL_STYLE, HWND_TOP,
            SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SW_MAXIMIZE, SW_RESTORE, WS_CAPTION,
            WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
        };

        unsafe {
            let style = GetWindowLongW(self.hwnd, GWL_STYLE) as u32;
            let windowed_style =
                WS_CAPTION.0 | WS_SYSMENU.0 | WS_THICKFRAME.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0;

            if (style & windowed_style) != 0 {
                // Enter fullscreen
                let new_style = style & !windowed_style;
                SetWindowLongW(self.hwnd, GWL_STYLE, new_style as i32);
                let _ = ShowWindow(self.hwnd, SW_MAXIMIZE);
            } else {
                // Exit fullscreen
                let new_style = style | windowed_style;
                SetWindowLongW(self.hwnd, GWL_STYLE, new_style as i32);
                let _ = ShowWindow(self.hwnd, SW_RESTORE);
            }
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOP),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_FRAMECHANGED,
            );
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
                drop(term);
            }
            self.invalidate();
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

    /// Handle title change from terminal
    pub fn on_title_changed(&mut self, tab_id: u64) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            // Don't update if title is locked (user-set or template)
            if tab.title_locked {
                return;
            }

            // Get title from terminal's screen
            let new_title = {
                let term = tab.terminal.lock().unwrap();
                term.screen().title.clone()
            };

            if !new_title.is_empty() {
                tab.title = new_title.clone();
                self.tab_bar.set_title(tab_id, &new_title);
            }
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

    /// Handle right-click for context menu
    pub fn on_right_click(&mut self, x: f32, y: f32) {
        // Check if click is in tab bar area
        let tab_bar_height = self.dpi.scale_f32(TAB_BAR_HEIGHT as f32);

        if y < tab_bar_height && self.tab_bar.is_visible() {
            // Hit test the tab bar
            let (tab_id, _is_close, _is_new) = self.tab_bar.hit_test(x, y);
            if let Some(tab_id) = tab_id {
                self.show_tab_context_menu(tab_id, x as i32, y as i32);
            }
        }
    }

    /// Show context menu for a tab
    fn show_tab_context_menu(&mut self, tab_id: u64, x: i32, y: i32) {
        use windows::Win32::UI::WindowsAndMessaging::{
            CreatePopupMenu, InsertMenuW, TrackPopupMenu, MF_STRING, TPM_LEFTALIGN, TPM_TOPALIGN,
        };

        const CMD_RENAME: u32 = 10001;
        const CMD_SET_COLOR: u32 = 10002;

        unsafe {
            let menu = CreatePopupMenu().unwrap();

            // Add menu items
            let rename_text: Vec<u16> = "Rename Tab...\0".encode_utf16().collect();
            let _ = InsertMenuW(
                menu,
                0,
                MF_STRING,
                CMD_RENAME as usize,
                PCWSTR(rename_text.as_ptr()),
            );

            let color_text: Vec<u16> = "Set Tab Color...\0".encode_utf16().collect();
            let _ = InsertMenuW(
                menu,
                1,
                MF_STRING,
                CMD_SET_COLOR as usize,
                PCWSTR(color_text.as_ptr()),
            );

            // Get screen coordinates
            let mut pt = windows::Win32::Foundation::POINT { x, y };
            let _ = windows::Win32::Graphics::Gdi::ClientToScreen(self.hwnd, &mut pt);

            // Show the menu
            let cmd = TrackPopupMenu(
                menu,
                TPM_LEFTALIGN
                    | TPM_TOPALIGN
                    | windows::Win32::UI::WindowsAndMessaging::TPM_RETURNCMD,
                pt.x,
                pt.y,
                None,
                self.hwnd,
                None,
            );

            // Handle the selected command
            if cmd.as_bool() {
                match cmd.0 as u32 {
                    CMD_RENAME => {
                        self.handle_tab_rename(tab_id);
                    }
                    CMD_SET_COLOR => {
                        self.handle_tab_set_color(tab_id);
                    }
                    _ => {}
                }
            }

            let _ = windows::Win32::UI::WindowsAndMessaging::DestroyMenu(menu);
        }
    }

    /// Handle tab rename from context menu
    fn handle_tab_rename(&mut self, tab_id: u64) {
        // Get current title
        let current_title = self
            .tabs
            .iter()
            .find(|t| t.id == tab_id)
            .map(|t| t.title.clone())
            .unwrap_or_default();

        // Show input dialog
        if let Some(new_title) = crate::dialogs::show_input_dialog_win(
            self.hwnd,
            "Rename Tab",
            "Enter new tab name:",
            &current_title,
        ) {
            // Update tab title
            if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.title = new_title.clone();
            }
            self.tab_bar.set_title(tab_id, &new_title);
            self.invalidate();
        }
    }

    /// Handle tab set color from context menu
    fn handle_tab_set_color(&mut self, tab_id: u64) {
        // Show color picker dialog
        if let Some(color_opt) = crate::dialogs::show_set_color_dialog_win(self.hwnd) {
            // Update tab color
            let rgb = color_opt.as_ref().and_then(|hex| parse_hex_color(hex));
            if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
                tab.color = color_opt;
            }
            self.tab_bar.set_color(tab_id, rgb);
            self.invalidate();
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
            let _ = unsafe { BeginPaint(hwnd, &mut ps) };
            state.render().ok();
            let _ = unsafe { EndPaint(hwnd, &ps) };
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
                // Only handle printable characters here. Control characters like
                // Enter (\r), Tab (\t), Backspace (\x08), and Escape (\x1b) are
                // already handled in WM_KEYDOWN via vk_to_terminal_seq.
                // TranslateMessage generates WM_CHAR for them too, so we must
                // skip them here to avoid double input.
                if !c.is_control() {
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

        WM_RBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as f32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32;
            state.on_right_click(x, y);
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

        WM_APP_TITLE_CHANGED => {
            let tab_id = wparam.0 as u64;
            state.on_title_changed(tab_id);
            LRESULT(0)
        }

        WM_SETFOCUS => {
            // Send focus in event to terminal if DECSET 1004 is enabled
            state.send_focus_event(true);
            LRESULT(0)
        }

        WM_KILLFOCUS => {
            // Send focus out event to terminal if DECSET 1004 is enabled
            state.send_focus_event(false);
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

/// Parse a hex color string (e.g., "#e74c3c") to Rgb
fn parse_hex_color(hex: &str) -> Option<Rgb> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }

    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;

    Some(Rgb::new(r, g, b))
}
