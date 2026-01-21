//! Terminal view implementation for macOS
//!
//! NSView subclass that renders the terminal using CoreGraphics.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSEvent, NSView};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSPoint, NSRect, NSSize};
use parking_lot::Mutex;

use cterm_app::config::Config;
use cterm_app::upgrade::{
    execute_upgrade, TabUpgradeState, TerminalUpgradeState, UpgradeState, WindowUpgradeState,
};
use cterm_core::screen::{ScreenConfig, SelectionMode};
use cterm_core::{Pty, PtyConfig, PtySize, Terminal};
use cterm_ui::theme::Theme;

use crate::cg_renderer::CGRenderer;
use crate::{clipboard, keycode};

/// Shared state between the view and PTY thread
struct ViewState {
    needs_redraw: AtomicBool,
    pty_closed: AtomicBool,
    /// Set when the view is being deallocated - threads should stop
    view_invalid: AtomicBool,
}

/// Terminal view state
pub struct TerminalViewIvars {
    terminal: Arc<Mutex<Terminal>>,
    pty: RefCell<Option<Pty>>,
    renderer: RefCell<Option<CGRenderer>>,
    cell_width: f64,
    cell_height: f64,
    /// Shared state with PTY thread
    state: Arc<ViewState>,
    /// Whether we're currently in a selection drag
    is_selecting: Cell<bool>,
    /// Template name (if this view was created from a template)
    template_name: RefCell<Option<String>>,
    /// Watchdog FD ID (for crash recovery unregistration)
    #[cfg(unix)]
    watchdog_fd_id: Cell<u64>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "TerminalView"]
    #[ivars = TerminalViewIvars]
    pub struct TerminalView;

    unsafe impl NSObjectProtocol for TerminalView {}

    // Override NSView/NSResponder methods
    impl TerminalView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            // Use top-left origin like most UI frameworks
            true
        }

        #[unsafe(method(viewDidMoveToWindow))]
        fn view_did_move_to_window(&self) {
            // Make ourselves first responder when added to window
            if let Some(window) = self.window() {
                window.makeFirstResponder(Some(self));
                // Trigger initial resize to match window content size
                self.handle_resize();
            }
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, new_size: NSSize) {
            // Call super
            let _: () = unsafe { msg_send![super(self), setFrameSize: new_size] };
            // Handle the resize
            self.handle_resize();
        }

        #[unsafe(method(viewWillMoveToWindow:))]
        fn view_will_move_to_window(&self, new_window: Option<&objc2_app_kit::NSWindow>) {
            // If moving to nil window (being removed), mark view as invalid
            // This tells background threads to stop using the view pointer
            if new_window.is_none() {
                log::debug!("View being removed from window, marking invalid");
                self.ivars().state.view_invalid.store(true, Ordering::SeqCst);

                // Unregister PTY from watchdog when view is removed
                #[cfg(unix)]
                self.unregister_pty_from_watchdog();
            }
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty_rect: NSRect) {
            // Clear the redraw flag
            self.ivars().state.needs_redraw.store(false, Ordering::Relaxed);

            if let Some(ref renderer) = *self.ivars().renderer.borrow() {
                let terminal = self.ivars().terminal.lock();
                renderer.render(&terminal, dirty_rect);
            }
        }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> objc2::runtime::Bool {
            let modifiers = keycode::modifiers_from_event(event);
            let raw_keycode = event.keyCode();

            // Handle Ctrl+Tab / Ctrl+Shift+Tab for tab switching
            // Tab key is virtual keycode 0x30 on macOS
            if raw_keycode == 0x30 && modifiers.contains(cterm_ui::events::Modifiers::CTRL) {
                if let Some(window) = self.window() {
                    if modifiers.contains(cterm_ui::events::Modifiers::SHIFT) {
                        let _: () = unsafe { msg_send![&*window, selectPreviousTab: std::ptr::null::<objc2::runtime::AnyObject>()] };
                    } else {
                        let _: () = unsafe { msg_send![&*window, selectNextTab: std::ptr::null::<objc2::runtime::AnyObject>()] };
                    }
                }
                return objc2::runtime::Bool::YES;
            }

            objc2::runtime::Bool::NO
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            use cterm_core::term::Key;

            let modifiers = keycode::modifiers_from_event(event);

            // Let Command+key combinations pass through to the menu system
            // Command is never part of terminal sequences
            if modifiers.contains(cterm_ui::events::Modifiers::SUPER) {
                // Don't handle - let the responder chain process it for menu shortcuts
                return;
            }

            // Reset scroll offset when any key is pressed (return to current content)
            {
                let mut terminal = self.ivars().terminal.lock();
                if terminal.screen().scroll_offset != 0 {
                    terminal.screen_mut().scroll_offset = 0;
                    drop(terminal);
                    self.set_needs_display();
                }
            }

            // Convert macOS keycode to terminal Key
            let raw_keycode = event.keyCode();
            let key = match raw_keycode {
                // Arrow keys
                0x7E => Some(Key::Up),
                0x7D => Some(Key::Down),
                0x7B => Some(Key::Left),
                0x7C => Some(Key::Right),
                // Navigation
                0x73 => Some(Key::Home),
                0x77 => Some(Key::End),
                0x74 => Some(Key::PageUp),
                0x79 => Some(Key::PageDown),
                // Editing
                0x72 => Some(Key::Insert),
                0x75 => Some(Key::Delete),
                0x33 => Some(Key::Backspace),
                0x24 => Some(Key::Enter),
                0x30 => Some(Key::Tab),
                0x35 => Some(Key::Escape),
                // Function keys
                0x7A => Some(Key::F(1)),
                0x78 => Some(Key::F(2)),
                0x63 => Some(Key::F(3)),
                0x76 => Some(Key::F(4)),
                0x60 => Some(Key::F(5)),
                0x61 => Some(Key::F(6)),
                0x62 => Some(Key::F(7)),
                0x64 => Some(Key::F(8)),
                0x65 => Some(Key::F(9)),
                0x6D => Some(Key::F(10)),
                0x67 => Some(Key::F(11)),
                0x6F => Some(Key::F(12)),
                _ => None,
            };

            // Convert cterm_ui Modifiers to cterm_core Modifiers
            let core_mods = cterm_core::term::Modifiers::from_bits_truncate(modifiers.bits());

            // If it's a special key, use Terminal::handle_key to get the escape sequence
            if let Some(key) = key {
                let terminal = self.ivars().terminal.lock();
                if let Some(data) = terminal.handle_key(key, core_mods) {
                    drop(terminal);
                    log::debug!("Special key: {:?} -> {:?}", key, data);
                    self.write_to_pty(&data);
                    return;
                }
            }

            // Handle Ctrl+key combinations - convert to control characters
            if modifiers.contains(cterm_ui::events::Modifiers::CTRL) {
                if let Some(chars) = keycode::characters_ignoring_modifiers(event) {
                    for c in chars.chars() {
                        // Convert letter to control character (Ctrl+C = 0x03, etc.)
                        let ctrl_char = match c.to_ascii_lowercase() {
                            'a'..='z' => (c.to_ascii_lowercase() as u8 - b'a' + 1) as char,
                            '[' => '\x1b',      // Escape
                            '\\' => '\x1c',     // File separator
                            ']' => '\x1d',      // Group separator
                            '^' => '\x1e',      // Record separator
                            '_' => '\x1f',      // Unit separator
                            '?' => '\x7f',      // Delete (Ctrl+?)
                            _ => continue,
                        };
                        log::debug!("Ctrl+{} -> 0x{:02x}", c, ctrl_char as u8);
                        self.write_to_pty(&[ctrl_char as u8]);
                    }
                }
                return;
            }

            // For regular characters, get the character from the event
            if let Some(chars) = keycode::characters_from_event(event) {
                // Filter out special Unicode characters that macOS uses for function keys
                // (U+F700-U+F8FF is the Private Use Area where macOS puts these)
                let filtered: String = chars
                    .chars()
                    .filter(|c| !('\u{F700}'..='\u{F8FF}').contains(c))
                    .collect();

                if !filtered.is_empty() {
                    log::debug!("Writing to PTY: {:?}", filtered);
                    self.write_to_pty(filtered.as_bytes());
                }
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            // Convert window coordinates to view coordinates
            let location_in_window = event.locationInWindow();
            let location = self.convert_point_from_view(location_in_window, None);

            // Calculate cell position
            let col = (location.x / self.ivars().cell_width).floor() as usize;
            let row = (location.y / self.ivars().cell_height).floor() as usize;

            // Determine selection mode based on click count
            let click_count = event.clickCount();
            let mode = match click_count {
                2 => SelectionMode::Word,
                3 => SelectionMode::Line,
                _ => SelectionMode::Char,
            };

            // Start selection
            let mut terminal = self.ivars().terminal.lock();
            let line = terminal.screen().visible_row_to_absolute_line(row);
            terminal.screen_mut().start_selection(line, col, mode);
            drop(terminal);

            self.ivars().is_selecting.set(true);
            self.set_needs_display();

            log::trace!("Mouse down at row={}, col={}, mode={:?}", row, col, mode);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            if !self.ivars().is_selecting.get() {
                return;
            }

            self.ivars().is_selecting.set(false);

            // Check if selection is empty and clear it, or copy to clipboard
            let terminal = self.ivars().terminal.lock();
            if let Some(selection) = &terminal.screen().selection {
                if selection.anchor == selection.end {
                    // Empty selection - clear it
                    drop(terminal);
                    let mut terminal = self.ivars().terminal.lock();
                    terminal.screen_mut().clear_selection();
                    self.set_needs_display();
                } else {
                    // Copy selection to clipboard
                    if let Some(text) = terminal.screen().get_selected_text() {
                        drop(terminal);
                        clipboard::set_text(&text);
                        log::debug!("Copied {} chars to clipboard", text.len());
                    }
                }
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            if !self.ivars().is_selecting.get() {
                return;
            }

            // Convert window coordinates to view coordinates
            let location_in_window = event.locationInWindow();
            let location = self.convert_point_from_view(location_in_window, None);

            // Calculate cell position (clamp to valid range)
            let col = (location.x / self.ivars().cell_width).floor().max(0.0) as usize;
            let row = (location.y / self.ivars().cell_height).floor().max(0.0) as usize;

            // Extend selection
            let mut terminal = self.ivars().terminal.lock();
            let line = terminal.screen().visible_row_to_absolute_line(row);
            terminal.screen_mut().extend_selection(line, col);
            drop(terminal);

            self.set_needs_display();
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let delta_y = event.scrollingDeltaY();
            log::trace!("Scroll wheel delta: {}", delta_y);

            // Reduce scroll speed by half
            let scroll_lines = (delta_y.abs() / 2.0) as usize;
            if scroll_lines == 0 {
                return;
            }

            let mut terminal = self.ivars().terminal.lock();
            if delta_y > 0.0 {
                terminal.scroll_viewport_up(scroll_lines);
            } else if delta_y < 0.0 {
                terminal.scroll_viewport_down(scroll_lines);
            }
            drop(terminal);

            self.set_needs_display();
        }

        /// Copy selection to clipboard (Command+C)
        #[unsafe(method(copy:))]
        fn action_copy(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let terminal = self.ivars().terminal.lock();
            if let Some(text) = terminal.screen().get_selected_text() {
                drop(terminal);
                clipboard::set_text(&text);
                log::debug!("Copied {} chars to clipboard", text.len());
            }
        }

        /// Paste from clipboard (Command+V)
        #[unsafe(method(paste:))]
        fn action_paste(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            if let Some(text) = clipboard::get_text() {
                // Check if bracketed paste mode is enabled
                let terminal = self.ivars().terminal.lock();
                let bracketed = terminal.screen().modes.bracketed_paste;
                drop(terminal);

                let paste_text = if bracketed {
                    format!("\x1b[200~{}\x1b[201~", text)
                } else {
                    text
                };

                self.write_to_pty(paste_text.as_bytes());
            }
        }

        /// Select all text (Command+A)
        #[unsafe(method(selectAll:))]
        fn action_select_all(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let mut terminal = self.ivars().terminal.lock();
            let total_lines = terminal.screen().total_lines();
            let width = terminal.screen().width();

            // Select from the first line to the last line
            terminal
                .screen_mut()
                .start_selection(0, 0, SelectionMode::Char);
            terminal
                .screen_mut()
                .extend_selection(total_lines.saturating_sub(1), width.saturating_sub(1));
            drop(terminal);

            self.set_needs_display();
        }

        /// Handle modifier key changes (for secret debug menu)
        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            use objc2_app_kit::NSEventModifierFlags;

            let flags = event.modifierFlags();
            let shift_pressed = flags.contains(NSEventModifierFlags::Shift);

            // Show/hide debug menu based on Shift key state
            crate::menu::set_debug_menu_visible(shift_pressed);
        }

        /// Debug: Re-launch cterm with state preservation
        #[unsafe(method(debugRelaunch:))]
        fn action_debug_relaunch(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            log::info!("Debug: Re-launching cterm with state preservation");

            // Get the path to the current executable
            let exe_path = match std::env::current_exe() {
                Ok(path) => path,
                Err(e) => {
                    log::error!("Failed to get executable path: {}", e);
                    return;
                }
            };

            // Build upgrade state
            let mut state = UpgradeState::new(env!("CARGO_PKG_VERSION"));
            let mut fds = Vec::new();

            // Create window state
            let mut window_state = WindowUpgradeState::new();

            // Get window geometry if available
            if let Some(window) = self.window() {
                let frame = window.frame();
                window_state.x = frame.origin.x as i32;
                window_state.y = frame.origin.y as i32;
                window_state.width = frame.size.width as i32;
                window_state.height = frame.size.height as i32;
            }

            // Export terminal state and get PTY FD
            let terminal_state = self.export_state();
            let child_pid = self.child_pid().unwrap_or(0);

            if let Some(fd) = self.dup_pty_fd() {
                let mut tab_state = TabUpgradeState::new(1, fds.len(), child_pid);
                tab_state.title = terminal_state.title.clone();
                tab_state.terminal = terminal_state;
                fds.push(fd);
                window_state.tabs.push(tab_state);
            } else {
                log::error!("Failed to duplicate PTY FD for upgrade");
                return;
            }

            state.windows.push(window_state);

            // Execute upgrade using the existing protocol
            log::info!(
                "Executing upgrade: {} windows, {} FDs",
                state.windows.len(),
                fds.len()
            );

            match execute_upgrade(&exe_path, &state, &fds) {
                Ok(()) => {
                    log::info!("Upgrade successful, terminating old process");
                    // Terminate this instance
                    let app = objc2_app_kit::NSApplication::sharedApplication(
                        objc2_foundation::MainThreadMarker::from(self),
                    );
                    app.terminate(None);
                }
                Err(e) => {
                    log::error!("Upgrade failed: {}", e);
                    // Close the duplicated FDs on failure
                    for fd in fds {
                        unsafe {
                            libc::close(fd);
                        }
                    }
                }
            }
        }

        /// Debug: Dump terminal state
        #[unsafe(method(debugDumpState:))]
        fn action_debug_dump_state(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            log::info!("Debug: Dumping terminal state");

            let terminal = self.ivars().terminal.lock();
            let screen = terminal.screen();

            log::info!("  Screen size: {}x{}", screen.width(), screen.height());
            log::info!("  Cursor: row={}, col={}", screen.cursor.row, screen.cursor.col);
            log::info!("  Total lines (with scrollback): {}", screen.total_lines());
            log::info!("  Selection: {:?}", screen.selection);
            log::info!("  Modes: {:?}", screen.modes);
        }

        /// Debug: Trigger a crash to test crash recovery
        #[unsafe(method(debugCrash:))]
        fn action_debug_crash(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            log::warn!("Debug: Triggering intentional crash for recovery testing");
            // Use abort() to trigger a crash that the watchdog can detect
            std::process::abort();
        }

        #[unsafe(method(triggerRedraw))]
        fn trigger_redraw(&self) {
            self.set_needs_display();
        }
    }
);

impl TerminalView {
    /// Create a new terminal view with a fresh shell
    pub fn new(mtm: MainThreadMarker, config: &Config, theme: &Theme) -> Retained<Self> {
        // Create CoreGraphics renderer first to get cell dimensions
        let font_name = &config.appearance.font.family;
        let font_size = config.appearance.font.size;
        let renderer = CGRenderer::new(mtm, font_name, font_size, theme);
        let (cell_width, cell_height) = renderer.cell_size();

        // Create terminal with default size (will resize later)
        let terminal = Terminal::new(80, 24, ScreenConfig::default());
        let terminal = Arc::new(Mutex::new(terminal));

        // Create shared state for PTY thread communication
        let state = Arc::new(ViewState {
            needs_redraw: AtomicBool::new(false),
            pty_closed: AtomicBool::new(false),
            view_invalid: AtomicBool::new(false),
        });

        // Initial frame
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(800.0, 600.0));

        // Allocate and initialize
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(TerminalViewIvars {
            terminal: terminal.clone(),
            pty: RefCell::new(None),
            renderer: RefCell::new(Some(renderer)),
            cell_width,
            cell_height,
            state: state.clone(),
            is_selecting: Cell::new(false),
            template_name: RefCell::new(None),
            #[cfg(unix)]
            watchdog_fd_id: Cell::new(0),
        });

        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };

        // Store view pointer as usize for thread-safe passing
        // Safety: We only use this pointer on the main thread via dispatch
        let view_ptr = &*this as *const _ as usize;

        // Spawn shell and register with watchdog
        this.spawn_shell(config, state.clone());
        this.register_pty_with_watchdog();

        // Start the redraw check loop
        this.schedule_redraw_check(view_ptr, state);

        this
    }

    /// Create a terminal view from a tab template
    pub fn from_template(
        mtm: MainThreadMarker,
        config: &Config,
        theme: &Theme,
        template: &cterm_app::config::StickyTabConfig,
    ) -> Retained<Self> {
        // Create CoreGraphics renderer first to get cell dimensions
        let font_name = &config.appearance.font.family;
        let font_size = config.appearance.font.size;
        let renderer = CGRenderer::new(mtm, font_name, font_size, theme);
        let (cell_width, cell_height) = renderer.cell_size();

        // Create terminal with default size (will resize later)
        let terminal = Terminal::new(80, 24, ScreenConfig::default());
        let terminal = Arc::new(Mutex::new(terminal));

        // Create shared state for PTY thread communication
        let state = Arc::new(ViewState {
            needs_redraw: AtomicBool::new(false),
            pty_closed: AtomicBool::new(false),
            view_invalid: AtomicBool::new(false),
        });

        // Initial frame
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(800.0, 600.0));

        // Allocate and initialize
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(TerminalViewIvars {
            terminal: terminal.clone(),
            pty: RefCell::new(None),
            renderer: RefCell::new(Some(renderer)),
            cell_width,
            cell_height,
            state: state.clone(),
            is_selecting: Cell::new(false),
            template_name: RefCell::new(Some(template.name.clone())),
            #[cfg(unix)]
            watchdog_fd_id: Cell::new(0),
        });

        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };

        // Store view pointer as usize for thread-safe passing
        let view_ptr = &*this as *const _ as usize;

        // Spawn shell from template and register with watchdog
        this.spawn_template_shell(config, template, state.clone());
        this.register_pty_with_watchdog();

        // Start the redraw check loop
        this.schedule_redraw_check(view_ptr, state);

        this
    }

    /// Create a terminal view from a recovered PTY FD (for crash recovery)
    ///
    /// This creates a fresh terminal screen but reuses the existing PTY connection.
    #[cfg(unix)]
    pub fn from_recovered_fd(
        mtm: MainThreadMarker,
        config: &Config,
        theme: &Theme,
        recovered: &cterm_app::RecoveredFd,
    ) -> Retained<Self> {
        use cterm_core::screen::{Screen, ScreenConfig};
        use std::io::Read;
        use std::os::unix::io::RawFd;

        // Create CoreGraphics renderer first to get cell dimensions
        let font_name = &config.appearance.font.family;
        let font_size = config.appearance.font.size;
        let renderer = CGRenderer::new(mtm, font_name, font_size, theme);
        let (cell_width, cell_height) = renderer.cell_size();

        // Create a fresh screen (we don't preserve screen state in crash recovery)
        let screen = Screen::new(80, 24, ScreenConfig::default());

        // Create Terminal from the recovered FD
        let terminal =
            unsafe { Terminal::from_restored_fd(screen, recovered.fd, recovered.child_pid) };

        // Get a reader for the PTY before wrapping terminal in Arc<Mutex>
        let pty_reader = terminal.pty_reader();

        // Wrap terminal in Arc<Mutex> for sharing
        let terminal = Arc::new(Mutex::new(terminal));

        // Create shared state for PTY thread communication
        let state = Arc::new(ViewState {
            needs_redraw: AtomicBool::new(false),
            pty_closed: AtomicBool::new(false),
            view_invalid: AtomicBool::new(false),
        });

        // Initial frame
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(800.0, 600.0));

        // Allocate and initialize
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(TerminalViewIvars {
            terminal: terminal.clone(),
            pty: RefCell::new(None), // PTY is owned by Terminal
            renderer: RefCell::new(Some(renderer)),
            cell_width,
            cell_height,
            state: state.clone(),
            is_selecting: Cell::new(false),
            template_name: RefCell::new(None),
            watchdog_fd_id: Cell::new(recovered.id), // Already registered with watchdog
        });

        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };

        // Store view pointer for thread-safe passing
        let view_ptr = &*this as *const _ as usize;

        // Start the PTY read loop if we have a reader
        if let Some(mut reader) = pty_reader {
            let terminal_clone = terminal.clone();
            let state_clone = state.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => {
                            log::info!("PTY closed (EOF) - recovered terminal");
                            break;
                        }
                        Ok(n) => {
                            let mut term = terminal_clone.lock();
                            term.process(&buf[..n]);
                            drop(term);
                            state_clone.needs_redraw.store(true, Ordering::Relaxed);
                        }
                        Err(e) => {
                            if e.kind() != std::io::ErrorKind::Interrupted {
                                log::error!("PTY read error (recovered): {}", e);
                                break;
                            }
                        }
                    }
                }
                state_clone.pty_closed.store(true, Ordering::Relaxed);
            });
        } else {
            log::warn!("Recovered terminal has no PTY reader");
        }

        // Start the redraw check loop
        this.schedule_redraw_check(view_ptr, state);

        log::info!(
            "Created recovered terminal (FD {}, PID {}, watchdog_id {})",
            recovered.fd,
            recovered.child_pid,
            recovered.id
        );

        this
    }

    /// Create a terminal view from a restored Terminal (for seamless upgrades)
    ///
    /// The Terminal should already have its PTY attached via `Terminal::from_restored()`.
    #[cfg(unix)]
    pub fn from_restored(
        mtm: MainThreadMarker,
        config: &Config,
        theme: &Theme,
        terminal: Terminal,
    ) -> Retained<Self> {
        use std::io::Read;

        // Create CoreGraphics renderer first to get cell dimensions
        let font_name = &config.appearance.font.family;
        let font_size = config.appearance.font.size;
        let renderer = CGRenderer::new(mtm, font_name, font_size, theme);
        let (cell_width, cell_height) = renderer.cell_size();

        // Get a reader for the PTY before wrapping terminal in Arc<Mutex>
        let pty_reader = terminal.pty_reader();

        // Wrap terminal in Arc<Mutex> for sharing
        let terminal = Arc::new(Mutex::new(terminal));

        // Create shared state for PTY thread communication
        let state = Arc::new(ViewState {
            needs_redraw: AtomicBool::new(false),
            pty_closed: AtomicBool::new(false),
            view_invalid: AtomicBool::new(false),
        });

        // Initial frame
        let frame = NSRect::new(NSPoint::ZERO, NSSize::new(800.0, 600.0));

        // Allocate and initialize
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(TerminalViewIvars {
            terminal: terminal.clone(),
            pty: RefCell::new(None), // PTY is owned by Terminal for restored case
            renderer: RefCell::new(Some(renderer)),
            cell_width,
            cell_height,
            state: state.clone(),
            is_selecting: Cell::new(false),
            template_name: RefCell::new(None), // TODO: restore template name from state
            #[cfg(unix)]
            watchdog_fd_id: Cell::new(0),
        });

        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };

        // Store view pointer as usize for thread-safe passing
        let view_ptr = &*this as *const _ as usize;

        // Start the PTY read loop if we have a reader
        if let Some(mut reader) = pty_reader {
            let terminal_clone = terminal.clone();
            let state_clone = state.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) => {
                            log::info!("PTY closed (EOF) - restored terminal");
                            break;
                        }
                        Ok(n) => {
                            let mut term = terminal_clone.lock();
                            term.process(&buf[..n]);
                            drop(term);
                            state_clone.needs_redraw.store(true, Ordering::Relaxed);
                        }
                        Err(e) => {
                            if e.kind() != std::io::ErrorKind::Interrupted {
                                log::error!("PTY read error (restored): {}", e);
                                break;
                            }
                        }
                    }
                }
                state_clone.pty_closed.store(true, Ordering::Relaxed);
            });
        } else {
            log::warn!("Restored terminal has no PTY reader");
        }

        // Start the redraw check loop
        this.schedule_redraw_check(view_ptr, state);

        this
    }

    fn schedule_redraw_check(&self, view_ptr: usize, state: Arc<ViewState>) {
        // Start a background thread that periodically triggers redraws on main thread
        std::thread::spawn(move || {
            // Wait briefly for app to initialize
            std::thread::sleep(std::time::Duration::from_millis(100));
            loop {
                std::thread::sleep(std::time::Duration::from_millis(16));

                // Check if view has been invalidated (window closed)
                if state.view_invalid.load(Ordering::SeqCst) {
                    log::debug!("View invalidated, stopping redraw thread");
                    break;
                }

                // Check if PTY closed - if so, close the window
                if state.pty_closed.load(Ordering::Relaxed) {
                    log::info!("PTY closed, closing window");
                    // Only close if view is still valid
                    if !state.view_invalid.load(Ordering::SeqCst) {
                        let state_clone = state.clone();
                        #[allow(deprecated)]
                        dispatch2::Queue::main().exec_async(move || {
                            // Double-check validity on main thread
                            if !state_clone.view_invalid.load(Ordering::SeqCst) && view_ptr != 0 {
                                unsafe {
                                    let view = &*(view_ptr as *const TerminalView);
                                    if let Some(window) = view.window() {
                                        window.close();
                                    }
                                }
                            }
                        });
                    }
                    break;
                }

                // Check for redraw
                if state.needs_redraw.swap(false, Ordering::Relaxed) {
                    // Only dispatch if view is still valid
                    if !state.view_invalid.load(Ordering::SeqCst) {
                        let state_clone = state.clone();
                        #[allow(deprecated)]
                        dispatch2::Queue::main().exec_async(move || {
                            // Double-check validity on main thread before accessing view
                            if !state_clone.view_invalid.load(Ordering::SeqCst) && view_ptr != 0 {
                                unsafe {
                                    let view = &*(view_ptr as *const TerminalView);
                                    let _: () = msg_send![view, setNeedsDisplay: true];
                                }
                            }
                        });
                    }
                }
            }
        });
    }

    /// Spawn the shell process
    fn spawn_shell(&self, config: &Config, state: Arc<ViewState>) {
        let shell =
            config.general.default_shell.clone().unwrap_or_else(|| {
                std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string())
            });

        let args: Vec<String> = config.general.shell_args.clone();

        let terminal = self.ivars().terminal.clone();

        let pty_config = PtyConfig {
            size: PtySize {
                cols: 80,
                rows: 24,
                pixel_width: 0,
                pixel_height: 0,
            },
            shell: Some(shell.clone()),
            args,
            cwd: None,
            env: Vec::new(),
        };

        match Pty::new(&pty_config) {
            Ok(pty) => {
                log::info!("Spawned shell: {}", shell);

                // Start reading from PTY in background
                let pty_fd = pty.raw_fd();

                std::thread::spawn(move || {
                    Self::read_pty_loop(pty_fd, terminal, state);
                });

                *self.ivars().pty.borrow_mut() = Some(pty);
            }
            Err(e) => {
                log::error!("Failed to spawn shell: {}", e);
            }
        }
    }

    /// Spawn a shell from a tab template
    fn spawn_template_shell(
        &self,
        config: &Config,
        template: &cterm_app::config::StickyTabConfig,
        state: Arc<ViewState>,
    ) {
        // Get command from template (or use default shell)
        let (shell, args) = template.get_command_args();
        let shell = shell.unwrap_or_else(|| {
            config.general.default_shell.clone().unwrap_or_else(|| {
                std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string())
            })
        });

        // Get working directory from template
        let cwd = template.working_directory.clone();

        // Merge environment variables
        let env: Vec<(String, String)> = template
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        let terminal = self.ivars().terminal.clone();

        let pty_config = PtyConfig {
            size: PtySize {
                cols: 80,
                rows: 24,
                pixel_width: 0,
                pixel_height: 0,
            },
            shell: Some(shell.clone()),
            args,
            cwd,
            env,
        };

        match Pty::new(&pty_config) {
            Ok(pty) => {
                log::info!(
                    "Spawned template shell: {} (template: {})",
                    shell,
                    template.name
                );

                // Start reading from PTY in background
                let pty_fd = pty.raw_fd();

                std::thread::spawn(move || {
                    Self::read_pty_loop(pty_fd, terminal, state);
                });

                *self.ivars().pty.borrow_mut() = Some(pty);
            }
            Err(e) => {
                log::error!("Failed to spawn template shell: {}", e);
            }
        }
    }

    /// Get the template name (if this view was created from a template)
    pub fn template_name(&self) -> Option<String> {
        self.ivars().template_name.borrow().clone()
    }

    /// Background thread to read from PTY
    fn read_pty_loop(pty_fd: i32, terminal: Arc<Mutex<Terminal>>, state: Arc<ViewState>) {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;

        let mut file = unsafe { std::fs::File::from_raw_fd(pty_fd) };
        let mut buf = [0u8; 4096];

        loop {
            match file.read(&mut buf) {
                Ok(0) => {
                    log::info!("PTY closed (EOF)");
                    break;
                }
                Ok(n) => {
                    let mut term = terminal.lock();
                    term.process(&buf[..n]);
                    drop(term);

                    // Signal that we need a redraw
                    state.needs_redraw.store(true, Ordering::Relaxed);
                }
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::Interrupted {
                        log::error!("PTY read error: {}", e);
                        break;
                    }
                }
            }
        }

        // Signal that PTY has closed - window should close
        state.pty_closed.store(true, Ordering::Relaxed);

        // Don't close the fd - it's owned by the Pty struct
        std::mem::forget(file);
    }

    /// Handle window resize
    pub fn handle_resize(&self) {
        let frame = self.frame();
        let cell_width = self.ivars().cell_width;
        let cell_height = self.ivars().cell_height;

        log::debug!(
            "handle_resize: frame={}x{}, cell={}x{}",
            frame.size.width,
            frame.size.height,
            cell_width,
            cell_height
        );

        if cell_width <= 0.0 || cell_height <= 0.0 {
            log::warn!("Invalid cell dimensions: {}x{}", cell_width, cell_height);
            return;
        }

        let cols = (frame.size.width / cell_width).floor() as usize;
        let rows = (frame.size.height / cell_height).floor() as usize;

        if cols > 0 && rows > 0 {
            let mut terminal = self.ivars().terminal.lock();
            // Terminal::resize() handles PTY resize internally if terminal owns the PTY
            terminal.resize(cols, rows);
            drop(terminal);

            // Also resize standalone pty if we have one (normal code path)
            // Note: pty.resize takes (rows, cols) not (cols, rows)
            if let Some(ref mut pty) = *self.ivars().pty.borrow_mut() {
                match pty.resize(rows as u16, cols as u16) {
                    Ok(()) => log::debug!("PTY resized to {}x{}", cols, rows),
                    Err(e) => log::error!("Failed to resize PTY: {}", e),
                }
            } else {
                log::debug!("No standalone PTY to resize");
            }

            log::debug!("Resized terminal to {}x{}", cols, rows);
        }
    }

    /// Write data to the PTY (handles both standalone and terminal-owned PTY)
    fn write_to_pty(&self, data: &[u8]) {
        // Try standalone pty first (normal case)
        if let Some(ref mut pty) = *self.ivars().pty.borrow_mut() {
            if let Err(e) = pty.write(data) {
                log::error!("Failed to write to PTY: {}", e);
            }
            return;
        }

        // Fall back to terminal's internal PTY (restored case)
        let mut terminal = self.ivars().terminal.lock();
        if let Err(e) = terminal.write(data) {
            log::error!("Failed to write to terminal PTY: {}", e);
        }
    }

    /// Get the terminal
    pub fn terminal(&self) -> &Arc<Mutex<Terminal>> {
        &self.ivars().terminal
    }

    /// Check if there's a foreground process running (other than the shell)
    #[cfg(unix)]
    pub fn has_foreground_process(&self) -> bool {
        // Check standalone PTY first
        if let Some(ref pty) = *self.ivars().pty.borrow() {
            return pty.has_foreground_process();
        }
        // Fall back to terminal's internal PTY
        self.ivars().terminal.lock().has_foreground_process()
    }

    /// Get the name of the foreground process (if any)
    #[cfg(unix)]
    pub fn foreground_process_name(&self) -> Option<String> {
        // Check standalone PTY first
        if let Some(ref pty) = *self.ivars().pty.borrow() {
            return pty.foreground_process_name();
        }
        // Fall back to terminal's internal PTY
        self.ivars().terminal.lock().foreground_process_name()
    }

    /// Request display update
    fn set_needs_display(&self) {
        unsafe {
            let _: () = msg_send![self, setNeedsDisplay: true];
        }
    }

    /// Get frame rectangle
    fn frame(&self) -> NSRect {
        unsafe { msg_send![self, frame] }
    }

    /// Convert point from window coordinates to view coordinates
    fn convert_point_from_view(&self, point: NSPoint, view: Option<&NSView>) -> NSPoint {
        unsafe { msg_send![self, convertPoint: point, fromView: view] }
    }

    /// Copy current selection to clipboard
    pub fn copy_selection(&self) {
        let terminal = self.ivars().terminal.lock();
        if let Some(text) = terminal.screen().get_selected_text() {
            drop(terminal);
            clipboard::set_text(&text);
            log::debug!("Copied {} chars to clipboard", text.len());
        }
    }

    /// Get selected text if any
    pub fn get_selected_text(&self) -> Option<String> {
        let terminal = self.ivars().terminal.lock();
        terminal.screen().get_selected_text()
    }

    /// Clear current selection
    pub fn clear_selection(&self) {
        let mut terminal = self.ivars().terminal.lock();
        terminal.screen_mut().clear_selection();
        drop(terminal);
        self.set_needs_display();
    }

    /// Export terminal state for seamless upgrade
    #[cfg(unix)]
    pub fn export_state(&self) -> TerminalUpgradeState {
        let term = self.ivars().terminal.lock();
        let screen = term.screen();

        TerminalUpgradeState {
            cols: screen.grid().width(),
            rows: screen.grid().height(),
            grid: screen.grid().clone(),
            scrollback: screen.scrollback().iter().cloned().collect(),
            alternate_grid: screen.alternate_grid().cloned(),
            cursor: screen.cursor.clone(),
            saved_cursor: screen.saved_cursor().cloned(),
            alt_saved_cursor: screen.alt_saved_cursor().cloned(),
            scroll_region: *screen.scroll_region(),
            style: screen.style.clone(),
            modes: screen.modes.clone(),
            title: screen.title.clone(),
            scroll_offset: screen.scroll_offset,
            tab_stops: screen.tab_stops().to_vec(),
            alternate_active: screen.alternate_grid().is_some(),
            cursor_style: screen.cursor.style,
            mouse_mode: screen.modes.mouse_mode,
        }
    }

    /// Duplicate the PTY file descriptor for upgrade transfer
    #[cfg(unix)]
    pub fn dup_pty_fd(&self) -> Option<std::os::unix::io::RawFd> {
        self.ivars()
            .pty
            .borrow()
            .as_ref()
            .and_then(|pty| pty.dup_fd().ok())
    }

    /// Get the child process ID
    #[cfg(unix)]
    pub fn child_pid(&self) -> Option<i32> {
        self.ivars()
            .pty
            .borrow()
            .as_ref()
            .map(|pty| pty.child_pid())
    }

    /// Register PTY FD with watchdog for crash recovery
    #[cfg(unix)]
    fn register_pty_with_watchdog(&self) {
        use std::os::unix::io::AsRawFd;

        let Some(watchdog_fd) = crate::app::get_watchdog_fd() else {
            return; // Not running under watchdog
        };

        let Some(pty_fd) = self.ivars().pty.borrow().as_ref().map(|p| p.as_raw_fd()) else {
            return; // No PTY yet
        };

        match cterm_app::register_fd_with_watchdog(watchdog_fd, pty_fd) {
            Ok(id) => {
                self.ivars().watchdog_fd_id.set(id);
                log::debug!("Registered PTY FD {} with watchdog (id={})", pty_fd, id);
            }
            Err(e) => {
                log::error!("Failed to register PTY with watchdog: {}", e);
            }
        }
    }

    /// Unregister PTY FD from watchdog (called when terminal is closed)
    #[cfg(unix)]
    pub fn unregister_pty_from_watchdog(&self) {
        let id = self.ivars().watchdog_fd_id.get();
        if id == 0 {
            return; // Not registered
        }

        let Some(watchdog_fd) = crate::app::get_watchdog_fd() else {
            return; // Not running under watchdog
        };

        if let Err(e) = cterm_app::unregister_fd_with_watchdog(watchdog_fd, id) {
            log::error!("Failed to unregister PTY from watchdog: {}", e);
        } else {
            log::debug!("Unregistered PTY from watchdog (id={})", id);
        }

        self.ivars().watchdog_fd_id.set(0);
    }
}
