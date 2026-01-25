//! Terminal view implementation for macOS
//!
//! NSView subclass that renders the terminal using CoreGraphics.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSEvent, NSMenu, NSMenuItem, NSTextInputClient, NSView};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSAttributedString, NSNumber, NSObjectProtocol, NSPoint, NSRange,
    NSRect, NSSize, NSString,
};
use parking_lot::Mutex;

use cterm_app::config::Config;
use cterm_app::upgrade::{
    execute_upgrade, TabUpgradeState, TerminalUpgradeState, UpgradeState, WindowUpgradeState,
};
use cterm_core::screen::{ScreenConfig, SelectionMode};
use cterm_core::{Pty, PtyConfig, PtySize, Terminal};
use cterm_ui::theme::Theme;

use crate::cg_renderer::CGRenderer;
use crate::file_transfer::PendingFileManager;
use crate::mouse::{self, MouseButton, MouseModifiers};
use crate::notification_bar::{NotificationBar, NOTIFICATION_BAR_HEIGHT};
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
    /// Marked text for IME input (Japanese, Chinese, etc.)
    marked_text: RefCell<String>,
    /// Notification bar for file transfers
    notification_bar: RefCell<Option<Retained<NotificationBar>>>,
    /// Pending file manager for file transfers
    file_manager: RefCell<PendingFileManager>,
    /// Color palette for HTML export
    color_palette: cterm_core::color::ColorPalette,
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
        fn draw_rect(&self, _dirty_rect: NSRect) {
            // Clear the redraw flag
            self.ivars().state.needs_redraw.store(false, Ordering::Relaxed);

            // Check for file transfers
            self.check_file_transfers();

            if let Some(ref renderer) = *self.ivars().renderer.borrow() {
                let terminal = self.ivars().terminal.lock();
                // Always use full view bounds for rendering to avoid artifacts
                // from partial dirty_rect updates after resize/fullscreen
                let bounds: NSRect = unsafe { msg_send![self, bounds] };
                renderer.render(&terminal, bounds);

                // Render IME marked text if present
                let marked_text = self.ivars().marked_text.borrow();
                if !marked_text.is_empty() {
                    let cursor = &terminal.screen().cursor;
                    renderer.render_marked_text(&marked_text, cursor.row, cursor.col);
                }
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

            log::debug!("keyDown: keyCode={}, modifiers={:?}", event.keyCode(), event.modifierFlags());

            let modifiers = keycode::modifiers_from_event(event);

            // Let Command+key combinations pass through to the menu system
            // Command is never part of terminal sequences
            if modifiers.contains(cterm_ui::events::Modifiers::SUPER) {
                // Don't handle - let the responder chain process it for menu shortcuts
                return;
            }

            // Check if IME composition is in progress (has marked text)
            let has_marked_text = !self.ivars().marked_text.borrow().is_empty();

            // If IME composition is in progress, route ALL keys through the input method system
            // This allows Enter to confirm composition, arrow keys to navigate candidates, etc.
            if has_marked_text {
                log::debug!("IME composition in progress, routing through interpretKeyEvents");
                let events = NSArray::from_slice(&[event]);
                self.interpretKeyEvents(&events);
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

            // Route through input method system for IME support (Japanese, Chinese, etc.)
            // This will call insertText: for regular characters or setMarkedText: for composing
            log::debug!("Routing key event through interpretKeyEvents for IME");
            let events = NSArray::from_slice(&[event]);
            self.interpretKeyEvents(&events);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            use objc2_app_kit::NSEventModifierFlags;

            // Convert window coordinates to view coordinates
            let location_in_window = event.locationInWindow();
            let location = self.convert_point_from_view(location_in_window, None);

            // Calculate cell position
            let col = (location.x / self.ivars().cell_width).floor().max(0.0) as usize;
            let row = (location.y / self.ivars().cell_height).floor().max(0.0) as usize;

            // Check for Cmd+click on hyperlinks
            let flags = event.modifierFlags();
            if flags.contains(NSEventModifierFlags::Command) {
                let terminal = self.ivars().terminal.lock();
                let absolute_line = terminal.screen().visible_row_to_absolute_line(row);

                if let Some(cell) = terminal.screen().get_cell_with_scrollback(absolute_line, col) {
                    if let Some(ref hyperlink) = cell.hyperlink {
                        let uri = hyperlink.uri.clone();
                        drop(terminal);
                        self.open_url(&uri);
                        return;
                    }
                }
                drop(terminal);
            }

            // Check if mouse reporting is active
            let terminal = self.ivars().terminal.lock();
            let mouse_mode = terminal.screen().modes.mouse_mode;
            let sgr_mouse = terminal.screen().modes.sgr_mouse;
            drop(terminal);

            if mouse::should_capture_mouse(mouse_mode) {
                // Send mouse event to application
                let button = match event.buttonNumber() {
                    0 => MouseButton::Left,
                    1 => MouseButton::Right,
                    2 => MouseButton::Middle,
                    _ => MouseButton::Left,
                };
                let modifiers = self.get_mouse_modifiers(event);

                if let Some(seq) = mouse::encode_mouse_event(
                    mouse_mode,
                    sgr_mouse,
                    button,
                    col,
                    row,
                    modifiers,
                    false,
                ) {
                    self.write_to_pty(&seq);
                }
                // Store that we're in a mouse reporting drag
                self.ivars().is_selecting.set(true);
                return;
            }

            // Normal selection mode
            // Determine selection mode based on click count and modifiers
            let click_count = event.clickCount();
            let mode = if flags.contains(NSEventModifierFlags::Option) {
                // Option+drag = block/rectangular selection
                SelectionMode::Block
            } else {
                match click_count {
                    2 => SelectionMode::Word,
                    3 => SelectionMode::Line,
                    _ => SelectionMode::Char,
                }
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
        fn mouse_up(&self, event: &NSEvent) {
            if !self.ivars().is_selecting.get() {
                return;
            }

            self.ivars().is_selecting.set(false);

            // Check if mouse reporting is active
            let terminal = self.ivars().terminal.lock();
            let mouse_mode = terminal.screen().modes.mouse_mode;
            let sgr_mouse = terminal.screen().modes.sgr_mouse;
            drop(terminal);

            if mouse::should_capture_mouse(mouse_mode) {
                // Send mouse release event to application
                let location_in_window = event.locationInWindow();
                let location = self.convert_point_from_view(location_in_window, None);
                let col = (location.x / self.ivars().cell_width).floor().max(0.0) as usize;
                let row = (location.y / self.ivars().cell_height).floor().max(0.0) as usize;
                let modifiers = self.get_mouse_modifiers(event);

                if let Some(seq) = mouse::encode_mouse_event(
                    mouse_mode,
                    sgr_mouse,
                    MouseButton::Release,
                    col,
                    row,
                    modifiers,
                    false,
                ) {
                    self.write_to_pty(&seq);
                }
                return;
            }

            // Normal selection mode - check if selection is empty and clear it, or copy to clipboard
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

            // Check if mouse reporting is active
            let terminal = self.ivars().terminal.lock();
            let mouse_mode = terminal.screen().modes.mouse_mode;
            let sgr_mouse = terminal.screen().modes.sgr_mouse;
            drop(terminal);

            if mouse::should_capture_mouse(mouse_mode) {
                // Send drag event to application (ButtonEvent or AnyEvent mode)
                let button = match event.buttonNumber() {
                    0 => MouseButton::Left,
                    1 => MouseButton::Right,
                    2 => MouseButton::Middle,
                    _ => MouseButton::Left,
                };
                let modifiers = self.get_mouse_modifiers(event);

                if let Some(seq) = mouse::encode_mouse_event(
                    mouse_mode,
                    sgr_mouse,
                    button,
                    col,
                    row,
                    modifiers,
                    true, // is_drag
                ) {
                    self.write_to_pty(&seq);
                }
                return;
            }

            // Normal selection mode - extend selection
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

            // Check if mouse reporting is active
            let terminal = self.ivars().terminal.lock();
            let mouse_mode = terminal.screen().modes.mouse_mode;
            let sgr_mouse = terminal.screen().modes.sgr_mouse;
            let in_alternate_screen = terminal.screen().modes.alternate_screen;
            drop(terminal);

            // If mouse reporting is active (and we're in alternate screen like vim/less),
            // send scroll events to the application
            if mouse::should_capture_mouse(mouse_mode) && in_alternate_screen {
                let location_in_window = event.locationInWindow();
                let location = self.convert_point_from_view(location_in_window, None);
                let col = (location.x / self.ivars().cell_width).floor().max(0.0) as usize;
                let row = (location.y / self.ivars().cell_height).floor().max(0.0) as usize;
                let modifiers = self.get_mouse_modifiers(event);

                // Send multiple scroll events based on delta
                let scroll_count = (delta_y.abs() / 2.0).max(1.0) as usize;
                let button = if delta_y > 0.0 {
                    MouseButton::WheelUp
                } else {
                    MouseButton::WheelDown
                };

                for _ in 0..scroll_count {
                    if let Some(seq) = mouse::encode_mouse_event(
                        mouse_mode,
                        sgr_mouse,
                        button,
                        col,
                        row,
                        modifiers,
                        false,
                    ) {
                        self.write_to_pty(&seq);
                    }
                }
                return;
            }

            // Normal scrollback mode
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

        /// Set up mouse tracking area for hover detection
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            use objc2_app_kit::{NSTrackingArea, NSTrackingAreaOptions};

            // Call super first
            let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };

            // Remove existing tracking areas
            let existing: Retained<objc2_foundation::NSArray<NSTrackingArea>> =
                unsafe { msg_send![self, trackingAreas] };
            for area in existing.iter() {
                let _: () = unsafe { msg_send![self, removeTrackingArea: &*area] };
            }

            // Create new tracking area covering the entire view
            let mtm = MainThreadMarker::from(self);
            let options = NSTrackingAreaOptions::MouseMoved
                | NSTrackingAreaOptions::ActiveInKeyWindow
                | NSTrackingAreaOptions::InVisibleRect;

            let bounds: NSRect = unsafe { msg_send![self, bounds] };
            let tracking_area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    mtm.alloc(),
                    bounds,
                    options,
                    Some(self),
                    None,
                )
            };

            let _: () = unsafe { msg_send![self, addTrackingArea: &*tracking_area] };
        }

        /// Handle mouse movement for hyperlink hover
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            let location_in_window = event.locationInWindow();
            let location = self.convert_point_from_view(location_in_window, None);

            let col = (location.x / self.ivars().cell_width).floor().max(0.0) as usize;
            let row = (location.y / self.ivars().cell_height).floor().max(0.0) as usize;

            // Check if we're over a hyperlink
            let terminal = self.ivars().terminal.lock();
            let absolute_line = terminal.screen().visible_row_to_absolute_line(row);

            if let Some(cell) = terminal.screen().get_cell_with_scrollback(absolute_line, col) {
                if let Some(ref hyperlink) = cell.hyperlink {
                    // Show tooltip with URL
                    let uri = hyperlink.uri.clone();
                    drop(terminal);
                    self.set_tooltip(&uri);

                    // Set cursor to pointing hand
                    unsafe {
                        let cursor: Retained<AnyObject> = msg_send![class!(NSCursor), pointingHandCursor];
                        let _: () = msg_send![&*cursor, set];
                    }
                    return;
                }
            }
            drop(terminal);

            // Clear tooltip and reset cursor if not over a hyperlink
            self.clear_tooltip();
            unsafe {
                let cursor: Retained<AnyObject> = msg_send![class!(NSCursor), IBeamCursor];
                let _: () = msg_send![&*cursor, set];
            }
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

        /// Copy selection to clipboard as HTML (Command+Shift+C)
        #[unsafe(method(copyAsHTML:))]
        fn action_copy_as_html(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let terminal = self.ivars().terminal.lock();
            let palette = &self.ivars().color_palette;
            if let Some(html) = terminal.screen().get_selected_html(palette) {
                let plain_text = terminal.screen().get_selected_text().unwrap_or_default();
                drop(terminal);
                clipboard::set_html(&html, &plain_text);
                log::debug!("Copied {} chars as HTML to clipboard", html.len());
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

        /// Reset terminal to initial state
        #[unsafe(method(resetTerminal:))]
        fn action_reset_terminal(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let mut terminal = self.ivars().terminal.lock();
            terminal.screen_mut().reset();
            drop(terminal);
            self.set_needs_display();
            log::debug!("Terminal reset");
        }

        /// Clear screen and reset terminal
        #[unsafe(method(clearAndResetTerminal:))]
        fn action_clear_and_reset_terminal(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            use cterm_core::screen::ClearMode;
            let mut terminal = self.ivars().terminal.lock();
            terminal.screen_mut().clear(ClearMode::All);
            terminal.screen_mut().reset();
            drop(terminal);
            self.set_needs_display();
            log::debug!("Terminal cleared and reset");
        }

        /// Set terminal title via dialog
        #[unsafe(method(setTerminalTitle:))]
        fn action_set_terminal_title(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSTextField};
            use objc2_foundation::{NSRect, NSString};

            let mtm = objc2_foundation::MainThreadMarker::from(self);

            // Create alert with text field for input
            let alert = NSAlert::new(mtm);
            alert.setMessageText(&NSString::from_str("Set Terminal Title"));
            alert.setInformativeText(&NSString::from_str("Enter a new title for this terminal:"));
            alert.setAlertStyle(NSAlertStyle::Informational);
            alert.addButtonWithTitle(&NSString::from_str("OK"));
            alert.addButtonWithTitle(&NSString::from_str("Cancel"));

            // Create text field for input
            let input_frame = NSRect::new(
                objc2_foundation::NSPoint::new(0.0, 0.0),
                objc2_foundation::NSSize::new(300.0, 24.0),
            );
            let input = unsafe { NSTextField::initWithFrame(mtm.alloc(), input_frame) };

            // Get current title as placeholder
            if let Some(window) = self.window() {
                input.setStringValue(&window.title());
            }

            alert.setAccessoryView(Some(&input));

            // Run modal and check result
            let response = alert.runModal();
            if response == NSAlertFirstButtonReturn {
                let new_title = input.stringValue();
                if let Some(window) = self.window() {
                    window.setTitle(&new_title);
                }
            }
        }

        #[unsafe(method(triggerRedraw))]
        fn trigger_redraw(&self) {
            self.set_needs_display();
        }

        // Signal action handlers
        #[unsafe(method(sendSignalHup:))]
        fn action_send_signal_hup(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            TerminalView::do_send_signal(self, 1); // SIGHUP
        }

        #[unsafe(method(sendSignalInt:))]
        fn action_send_signal_int(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            TerminalView::do_send_signal(self, 2); // SIGINT
        }

        #[unsafe(method(sendSignalQuit:))]
        fn action_send_signal_quit(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            TerminalView::do_send_signal(self, 3); // SIGQUIT
        }

        #[unsafe(method(sendSignalTerm:))]
        fn action_send_signal_term(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            TerminalView::do_send_signal(self, 15); // SIGTERM
        }

        #[unsafe(method(sendSignalKill:))]
        fn action_send_signal_kill(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            TerminalView::do_send_signal(self, 9); // SIGKILL
        }

        #[unsafe(method(sendSignalUsr1:))]
        fn action_send_signal_usr1(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            TerminalView::do_send_signal(self, 10); // SIGUSR1
        }

        #[unsafe(method(sendSignalUsr2:))]
        fn action_send_signal_usr2(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            TerminalView::do_send_signal(self, 12); // SIGUSR2
        }

        /// Notification bar save action
        #[unsafe(method(saveFile:))]
        fn save_file(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            if self.ivars().file_manager.borrow().has_pending() {
                if let Some(ref bar) = *self.ivars().notification_bar.borrow() {
                    self.handle_file_save(bar.file_id());
                }
            }
        }

        /// Notification bar save as action
        #[unsafe(method(saveFileAs:))]
        fn save_file_as(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            if self.ivars().file_manager.borrow().has_pending() {
                if let Some(ref bar) = *self.ivars().notification_bar.borrow() {
                    self.handle_file_save_as(bar.file_id());
                }
            }
        }

        /// Notification bar discard action
        #[unsafe(method(discardFile:))]
        fn discard_file(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            if self.ivars().file_manager.borrow().has_pending() {
                if let Some(ref bar) = *self.ivars().notification_bar.borrow() {
                    self.handle_file_discard(bar.file_id());
                }
            }
        }

        /// Right-click handler for context menu
        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            // Convert window coordinates to view coordinates
            let location_in_window = event.locationInWindow();
            let location = self.convert_point_from_view(location_in_window, None);

            // Calculate cell position
            let col = (location.x / self.ivars().cell_width).floor().max(0.0) as usize;
            let row = (location.y / self.ivars().cell_height).floor().max(0.0) as usize;

            let terminal = self.ivars().terminal.lock();
            let absolute_line = terminal.screen().visible_row_to_absolute_line(row);

            // Check if we clicked on an image
            if let Some(image) = terminal.screen().image_at_position(row, col) {
                let image_id = image.id;
                drop(terminal);
                self.show_image_context_menu(event, image_id);
                return;
            }

            // Check if we clicked on a hyperlink
            if let Some(cell) = terminal.screen().get_cell_with_scrollback(absolute_line, col) {
                if let Some(ref hyperlink) = cell.hyperlink {
                    let uri = hyperlink.uri.clone();
                    drop(terminal);
                    self.show_hyperlink_context_menu(event, &uri);
                    return;
                }
            }
            drop(terminal);

            // Default: no context menu for now
        }

        /// Copy image to clipboard
        #[unsafe(method(copyImage:))]
        fn copy_image(&self, sender: Option<&NSMenuItem>) {
            let Some(sender) = sender else { return };
            let image_id = self.get_image_id_from_menu_item(sender);
            if image_id == 0 {
                return;
            }

            let terminal = self.ivars().terminal.lock();
            if let Some(image) = terminal.screen().image_by_id(image_id) {
                // Copy image to pasteboard as PNG
                self.copy_image_to_pasteboard(image);
            }
        }

        /// Save image as...
        #[unsafe(method(saveImageAs:))]
        fn save_image_as(&self, sender: Option<&NSMenuItem>) {
            let Some(sender) = sender else { return };
            let image_id = self.get_image_id_from_menu_item(sender);
            if image_id == 0 {
                return;
            }

            let terminal = self.ivars().terminal.lock();
            if let Some(image) = terminal.screen().image_by_id(image_id) {
                let data = image.data.clone();
                let width = image.pixel_width;
                let height = image.pixel_height;
                drop(terminal);

                let mtm = MainThreadMarker::from(self);
                if let Some(path) = crate::dialogs::show_save_panel(
                    mtm,
                    self.window().as_deref(),
                    Some("image.png"),
                    None,
                ) {
                    // Encode as PNG and save
                    if let Err(e) = self.save_image_as_png(&data, width, height, &path) {
                        log::error!("Failed to save image: {}", e);
                    } else {
                        log::info!("Saved image to {:?}", path);
                    }
                }
            }
        }

        /// Open image in default application
        #[unsafe(method(openImage:))]
        fn open_image(&self, sender: Option<&NSMenuItem>) {
            let Some(sender) = sender else { return };
            let image_id = self.get_image_id_from_menu_item(sender);
            if image_id == 0 {
                return;
            }

            let terminal = self.ivars().terminal.lock();
            if let Some(image) = terminal.screen().image_by_id(image_id) {
                let data = image.data.clone();
                let width = image.pixel_width;
                let height = image.pixel_height;
                drop(terminal);

                // Save to temp file and open
                let temp_path = std::env::temp_dir().join(format!("cterm_image_{}.png", image_id));
                if let Err(e) = self.save_image_as_png(&data, width, height, &temp_path) {
                    log::error!("Failed to save temp image: {}", e);
                    return;
                }

                // Open with default application
                use objc2_app_kit::NSWorkspace;
                use objc2_foundation::NSURL;
                let workspace = NSWorkspace::sharedWorkspace();
                let url = NSURL::fileURLWithPath(&NSString::from_str(
                    temp_path.to_str().unwrap_or(""),
                ));
                workspace.openURL(&url);
            }
        }

        /// Open URL from context menu
        #[unsafe(method(openURL:))]
        fn open_url_action(&self, sender: Option<&NSMenuItem>) {
            let Some(sender) = sender else { return };
            if let Some(url) = self.get_url_from_menu_item(sender) {
                self.open_url(&url);
            }
        }

        /// Copy URL to clipboard from context menu
        #[unsafe(method(copyURL:))]
        fn copy_url_action(&self, sender: Option<&NSMenuItem>) {
            let Some(sender) = sender else { return };
            if let Some(url) = self.get_url_from_menu_item(sender) {
                clipboard::set_text(&url);
                log::debug!("Copied URL to clipboard: {}", url);
            }
        }
    }

    // NSTextInputClient protocol for IME support (Japanese, Chinese, Korean, etc.)
    unsafe impl NSTextInputClient for TerminalView {
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text_replacement_range(
            &self,
            string: &AnyObject,
            _replacement_range: NSRange,
        ) {
            log::debug!("NSTextInputClient: insertText:replacementRange: called");

            // Clear marked text
            self.ivars().marked_text.borrow_mut().clear();

            // Get the string content (could be NSString or NSAttributedString)
            let text: String = unsafe {
                if msg_send![string, isKindOfClass: objc2::class!(NSAttributedString)] {
                    let attr_str: &NSAttributedString = &*(string as *const _ as *const _);
                    attr_str.string().to_string()
                } else {
                    let ns_str: &NSString = &*(string as *const _ as *const _);
                    ns_str.to_string()
                }
            };

            if !text.is_empty() {
                log::debug!("IME insert text: {:?}", text);
                self.write_to_pty(text.as_bytes());
            }
        }

        /// Called when the input method wants to perform a command (e.g., delete, move cursor)
        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, selector: objc2::runtime::Sel) {
            log::debug!("NSTextInputClient: doCommandBySelector: {:?}", selector.name());
            // For "noop:" selector, just ignore - this is sent for unhandled keys
            if selector.name().to_str() == Ok("noop:") {
                return;
            }
            // We don't handle any commands - let the responder chain handle them
            // This is important: we need to call super or the input system won't work
            let _: () = unsafe { msg_send![super(self), doCommandBySelector: selector] };
        }

        /// Older insertText: method (some input methods use this instead of insertText:replacementRange:)
        #[unsafe(method(insertText:))]
        fn insert_text(&self, string: &AnyObject) {
            log::debug!("NSTextInputClient: insertText: called (old method)");
            // Clear marked text and send the text
            self.ivars().marked_text.borrow_mut().clear();

            // Get the string content (could be NSString or NSAttributedString)
            let text: String = unsafe {
                if msg_send![string, isKindOfClass: objc2::class!(NSAttributedString)] {
                    let attr_str: &NSAttributedString = &*(string as *const _ as *const _);
                    attr_str.string().to_string()
                } else {
                    let ns_str: &NSString = &*(string as *const _ as *const _);
                    ns_str.to_string()
                }
            };

            if !text.is_empty() {
                log::debug!("IME insert text (old method): {:?}", text);
                self.write_to_pty(text.as_bytes());
            }
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text_selected_range_replacement_range(
            &self,
            string: &AnyObject,
            _selected_range: NSRange,
            _replacement_range: NSRange,
        ) {
            // Get the string content
            let text: String = unsafe {
                if msg_send![string, isKindOfClass: objc2::class!(NSAttributedString)] {
                    let attr_str: &NSAttributedString = &*(string as *const _ as *const _);
                    attr_str.string().to_string()
                } else {
                    let ns_str: &NSString = &*(string as *const _ as *const _);
                    ns_str.to_string()
                }
            };

            log::debug!("NSTextInputClient: setMarkedText: {:?}", text);
            *self.ivars().marked_text.borrow_mut() = text;
            // Trigger redraw to show the composition text
            self.set_needs_display();
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            log::debug!("NSTextInputClient: unmarkText");
            self.ivars().marked_text.borrow_mut().clear();
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            log::trace!("NSTextInputClient: selectedRange");
            NSRange::new(0, 0)
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            let marked = self.ivars().marked_text.borrow();
            log::trace!("NSTextInputClient: markedRange (len={})", marked.len());
            if marked.is_empty() {
                NSRange::new(usize::MAX, 0) // NSNotFound
            } else {
                NSRange::new(0, marked.len())
            }
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            let has = !self.ivars().marked_text.borrow().is_empty();
            log::trace!("NSTextInputClient: hasMarkedText -> {}", has);
            has
        }

        #[unsafe(method(attributedSubstringForProposedRange:actualRange:))]
        fn attributed_substring_for_proposed_range_actual_range(
            &self,
            _range: NSRange,
            _actual_range: *mut NSRange,
        ) -> *mut NSAttributedString {
            std::ptr::null_mut()
        }

        #[unsafe(method(validAttributesForMarkedText))]
        fn valid_attributes_for_marked_text(&self) -> *mut NSArray<NSString> {
            // Return an empty array - we don't support any special attributes
            Retained::into_raw(NSArray::new())
        }

        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn first_rect_for_character_range_actual_range(
            &self,
            _range: NSRange,
            _actual_range: *mut NSRange,
        ) -> NSRect {
            // Return the rect where the IME candidate window should appear
            // Use cursor position
            let terminal = self.ivars().terminal.lock();
            let cursor = &terminal.screen().cursor;
            let cell_width = self.ivars().cell_width;
            let cell_height = self.ivars().cell_height;

            let x = cursor.col as f64 * cell_width;
            let y = cursor.row as f64 * cell_height;
            drop(terminal);

            // Convert to screen coordinates
            let view_rect = NSRect::new(
                NSPoint::new(x, y + cell_height),
                NSSize::new(cell_width, cell_height),
            );

            if let Some(window) = self.window() {
                let window_rect = self.convertRect_toView(view_rect, None);
                window.convertRectToScreen(window_rect)
            } else {
                view_rect
            }
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index_for_point(&self, _point: NSPoint) -> usize {
            0
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
        let mut terminal = Terminal::new(80, 24, ScreenConfig::default());
        // Set cell height hint for sixel image positioning
        terminal.screen_mut().set_cell_height_hint(cell_height);
        terminal.screen_mut().set_cell_width_hint(cell_width);
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
            renderer: RefCell::new(Some(renderer)),
            cell_width,
            cell_height,
            state: state.clone(),
            is_selecting: Cell::new(false),
            template_name: RefCell::new(None),
            #[cfg(unix)]
            watchdog_fd_id: Cell::new(0),
            marked_text: RefCell::new(String::new()),
            notification_bar: RefCell::new(None),
            file_manager: RefCell::new(PendingFileManager::new()),
            color_palette: theme.colors.clone(),
        });

        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };

        // Create notification bar
        this.setup_notification_bar(mtm);

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
        let mut renderer = CGRenderer::new(mtm, font_name, font_size, theme);

        // Apply background color override from template if set
        if let Some(ref bg_color) = template.background_color {
            renderer.set_background_override(Some(bg_color));
        }

        let (cell_width, cell_height) = renderer.cell_size();

        // Create terminal with default size (will resize later)
        let mut terminal = Terminal::new(80, 24, ScreenConfig::default());
        // Set cell height hint for sixel image positioning
        terminal.screen_mut().set_cell_height_hint(cell_height);
        terminal.screen_mut().set_cell_width_hint(cell_width);
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
            renderer: RefCell::new(Some(renderer)),
            cell_width,
            cell_height,
            state: state.clone(),
            is_selecting: Cell::new(false),
            template_name: RefCell::new(Some(template.name.clone())),
            #[cfg(unix)]
            watchdog_fd_id: Cell::new(0),
            marked_text: RefCell::new(String::new()),
            notification_bar: RefCell::new(None),
            file_manager: RefCell::new(PendingFileManager::new()),
            color_palette: theme.colors.clone(),
        });

        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };

        // Create notification bar
        this.setup_notification_bar(mtm);

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
        let mut screen = Screen::new(80, 24, ScreenConfig::default());
        // Set cell height hint for sixel image positioning
        screen.set_cell_height_hint(cell_height);
        screen.set_cell_width_hint(cell_width);

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
            renderer: RefCell::new(Some(renderer)),
            cell_width,
            cell_height,
            state: state.clone(),
            is_selecting: Cell::new(false),
            template_name: RefCell::new(None),
            watchdog_fd_id: Cell::new(recovered.id), // Already registered with watchdog
            marked_text: RefCell::new(String::new()),
            notification_bar: RefCell::new(None),
            file_manager: RefCell::new(PendingFileManager::new()),
            color_palette: theme.colors.clone(),
        });

        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };

        // Create notification bar
        this.setup_notification_bar(mtm);

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
        mut terminal: Terminal,
    ) -> Retained<Self> {
        use std::io::Read;

        // Create CoreGraphics renderer first to get cell dimensions
        let font_name = &config.appearance.font.family;
        let font_size = config.appearance.font.size;
        let renderer = CGRenderer::new(mtm, font_name, font_size, theme);
        let (cell_width, cell_height) = renderer.cell_size();

        // Set cell height hint for sixel image positioning
        terminal.screen_mut().set_cell_height_hint(cell_height);
        terminal.screen_mut().set_cell_width_hint(cell_width);

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
            renderer: RefCell::new(Some(renderer)),
            cell_width,
            cell_height,
            state: state.clone(),
            is_selecting: Cell::new(false),
            template_name: RefCell::new(None), // Caller should use set_template_name() if needed
            #[cfg(unix)]
            watchdog_fd_id: Cell::new(0),
            marked_text: RefCell::new(String::new()),
            notification_bar: RefCell::new(None),
            file_manager: RefCell::new(PendingFileManager::new()),
            color_palette: theme.colors.clone(),
        });

        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };

        // Create notification bar
        this.setup_notification_bar(mtm);

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
            term: config.general.term.clone(),
        };

        match Pty::new(&pty_config) {
            Ok(pty) => {
                log::info!("Spawned shell: {}", shell);

                // Start reading from PTY in background
                let pty_fd = pty.raw_fd();

                std::thread::spawn(move || {
                    Self::read_pty_loop(pty_fd, terminal, state);
                });

                // Store PTY in Terminal (the canonical location)
                self.ivars().terminal.lock().set_pty(pty);
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
            term: config.general.term.clone(),
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

                // Store PTY in Terminal (the canonical location)
                self.ivars().terminal.lock().set_pty(pty);
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

    /// Set the template name (for restoration from saved state)
    pub fn set_template_name(&self, name: Option<String>) {
        *self.ivars().template_name.borrow_mut() = name;
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
        // Use into_raw_fd() to consume the File without closing the fd
        use std::os::unix::io::IntoRawFd;
        let _ = file.into_raw_fd();
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

        // Update notification bar width
        if let Some(ref bar) = *self.ivars().notification_bar.borrow() {
            bar.update_width(frame.size.width);
        }

        if cell_width <= 0.0 || cell_height <= 0.0 {
            log::warn!("Invalid cell dimensions: {}x{}", cell_width, cell_height);
            return;
        }

        let cols = (frame.size.width / cell_width).floor() as usize;
        let rows = (frame.size.height / cell_height).floor() as usize;

        if cols > 0 && rows > 0 {
            let mut terminal = self.ivars().terminal.lock();
            terminal.resize(cols, rows);
            log::debug!("Resized terminal to {}x{}", cols, rows);
        }
    }

    /// Write data to the PTY
    fn write_to_pty(&self, data: &[u8]) {
        let mut terminal = self.ivars().terminal.lock();
        if let Err(e) = terminal.write(data) {
            log::error!("Failed to write to PTY: {}", e);
        }
    }

    /// Get the terminal
    pub fn terminal(&self) -> &Arc<Mutex<Terminal>> {
        &self.ivars().terminal
    }

    /// Get the cell size (width, height) for grid snapping
    pub fn cell_size(&self) -> (f64, f64) {
        (self.ivars().cell_width, self.ivars().cell_height)
    }

    /// Send focus event to terminal if focus events mode is enabled (DECSET 1004)
    /// `focused`: true for focus in (\x1b[I), false for focus out (\x1b[O)
    pub fn send_focus_event(&self, focused: bool) {
        let mut terminal = self.ivars().terminal.lock();
        if terminal.screen().modes.focus_events {
            let sequence = if focused { b"\x1b[I" } else { b"\x1b[O" };
            if let Err(e) = terminal.write(sequence) {
                log::error!("Failed to send focus event: {}", e);
            }
        }
    }

    /// Check if there's a foreground process running (other than the shell)
    #[cfg(unix)]
    pub fn has_foreground_process(&self) -> bool {
        self.ivars().terminal.lock().has_foreground_process()
    }

    /// Get the name of the foreground process (if any)
    #[cfg(unix)]
    pub fn foreground_process_name(&self) -> Option<String> {
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

    /// Get mouse modifiers from NSEvent
    fn get_mouse_modifiers(&self, event: &NSEvent) -> MouseModifiers {
        use objc2_app_kit::NSEventModifierFlags;

        let flags = event.modifierFlags();
        MouseModifiers {
            shift: flags.contains(NSEventModifierFlags::Shift),
            alt: flags.contains(NSEventModifierFlags::Option),
            ctrl: flags.contains(NSEventModifierFlags::Control),
        }
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

    /// Send a signal to the terminal's child process
    #[cfg(unix)]
    fn do_send_signal(&self, signal: i32) {
        let terminal = self.ivars().terminal.lock();
        if let Err(e) = terminal.send_signal(signal) {
            log::error!("Failed to send signal {}: {}", signal, e);
        } else {
            log::info!("Sent signal {} to terminal process", signal);
        }
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
        self.ivars().terminal.lock().dup_pty_fd()
    }

    /// Get the child process ID
    #[cfg(unix)]
    pub fn child_pid(&self) -> Option<i32> {
        self.ivars().terminal.lock().child_pid()
    }

    /// Register PTY FD with watchdog for crash recovery
    #[cfg(unix)]
    fn register_pty_with_watchdog(&self) {
        use std::os::unix::io::AsRawFd;

        let Some(watchdog_fd) = crate::app::get_watchdog_fd() else {
            return; // Not running under watchdog
        };

        let terminal = self.ivars().terminal.lock();
        let Some(pty_fd) = terminal.pty().map(|p| p.as_raw_fd()) else {
            drop(terminal);
            return; // No PTY yet
        };
        drop(terminal);

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

    /// Get the watchdog FD ID (for crash state saving)
    #[cfg(unix)]
    pub fn watchdog_fd_id(&self) -> u64 {
        self.ivars().watchdog_fd_id.get()
    }

    /// Setup the notification bar
    fn setup_notification_bar(&self, mtm: MainThreadMarker) {
        let frame = self.frame();
        let bar = NotificationBar::new(mtm, frame.size.width);

        // Position at top of view
        let bar_frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(frame.size.width, NOTIFICATION_BAR_HEIGHT),
        );
        unsafe {
            let _: () = msg_send![&*bar, setFrame: bar_frame];
            self.addSubview(&bar);
        }

        // Set action target to self (TerminalView) for button actions
        bar.set_action_target(self);

        *self.ivars().notification_bar.borrow_mut() = Some(bar);
    }

    /// Check for pending file transfers and show notification if needed
    pub fn check_file_transfers(&self) {
        let mut terminal = self.ivars().terminal.lock();
        let transfers = terminal.screen_mut().take_file_transfers();
        drop(terminal);

        for transfer in transfers {
            match transfer {
                cterm_core::FileTransferOperation::FileReceived { id, name, data } => {
                    let size = data.len();
                    self.ivars()
                        .file_manager
                        .borrow_mut()
                        .set_pending(id, name.clone(), data);

                    if let Some(ref bar) = *self.ivars().notification_bar.borrow() {
                        bar.show_file(id, name.as_deref(), size);
                    }

                    log::info!(
                        "File transfer received: {:?} ({} bytes)",
                        name.as_deref().unwrap_or("unnamed"),
                        size
                    );
                }
                cterm_core::FileTransferOperation::StreamingFileReceived { id, result } => {
                    let name = result.params.name.clone();
                    let size = result.total_bytes;

                    self.ivars()
                        .file_manager
                        .borrow_mut()
                        .set_pending_streaming(id, name.clone(), result.data);

                    if let Some(ref bar) = *self.ivars().notification_bar.borrow() {
                        bar.show_file(id, name.as_deref(), size);
                    }

                    log::info!(
                        "Streaming file transfer received: {:?} ({} bytes)",
                        name.as_deref().unwrap_or("unnamed"),
                        size
                    );
                }
            }
        }
    }

    /// Handle save button click from notification bar
    pub fn handle_file_save(&self, file_id: u64) {
        let mut manager = self.ivars().file_manager.borrow_mut();

        if let Some(path) = manager.default_save_path() {
            match manager.save_to_path(file_id, &path) {
                Ok(size) => {
                    log::info!("Saved {} bytes to {:?}", size, path);
                }
                Err(e) => {
                    log::error!("Failed to save file: {}", e);
                }
            }
        }
        drop(manager);

        if let Some(ref bar) = *self.ivars().notification_bar.borrow() {
            bar.hide();
        }
    }

    /// Handle save-as button click from notification bar
    pub fn handle_file_save_as(&self, file_id: u64) {
        let mtm = MainThreadMarker::from(self);
        let manager = self.ivars().file_manager.borrow();

        let suggested_name = manager.suggested_filename().map(|s| s.to_string());
        let suggested_dir = manager.last_save_dir().cloned();
        drop(manager);

        if let Some(path) = crate::dialogs::show_save_panel(
            mtm,
            self.window().as_deref(),
            suggested_name.as_deref(),
            suggested_dir.as_deref(),
        ) {
            let mut manager = self.ivars().file_manager.borrow_mut();
            match manager.save_to_path(file_id, &path) {
                Ok(size) => {
                    log::info!("Saved {} bytes to {:?}", size, path);
                }
                Err(e) => {
                    log::error!("Failed to save file: {}", e);
                }
            }
        }

        if let Some(ref bar) = *self.ivars().notification_bar.borrow() {
            bar.hide();
        }
    }

    /// Handle discard button click from notification bar
    pub fn handle_file_discard(&self, file_id: u64) {
        self.ivars().file_manager.borrow_mut().discard(file_id);

        if let Some(ref bar) = *self.ivars().notification_bar.borrow() {
            bar.hide();
        }

        log::debug!("Discarded file {}", file_id);
    }

    /// Set tooltip for the view (shows URL on hyperlink hover)
    fn set_tooltip(&self, text: &str) {
        let ns_text = NSString::from_str(text);
        let _: () = unsafe { msg_send![self, setToolTip: &*ns_text] };
    }

    /// Clear the tooltip
    fn clear_tooltip(&self) {
        let _: () = unsafe { msg_send![self, setToolTip: std::ptr::null::<NSString>()] };
    }

    /// Open a URL in the default browser
    fn open_url(&self, url: &str) {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::NSURL;

        let workspace = NSWorkspace::sharedWorkspace();
        if let Some(ns_url) = unsafe { NSURL::URLWithString(&NSString::from_str(url)) } {
            workspace.openURL(&ns_url);
            log::debug!("Opened URL: {}", url);
        } else {
            log::warn!("Failed to parse URL: {}", url);
        }
    }

    /// Show context menu for an image
    fn show_image_context_menu(&self, event: &NSEvent, image_id: u64) {
        let mtm = MainThreadMarker::from(self);
        let menu = NSMenu::new(mtm);

        // Copy Image
        let copy_item = NSMenuItem::new(mtm);
        copy_item.setTitle(&NSString::from_str("Copy Image"));
        unsafe {
            copy_item.setTarget(Some(self));
            copy_item.setAction(Some(sel!(copyImage:)));
            copy_item.setRepresentedObject(Some(&NSNumber::new_u64(image_id)));
        }
        menu.addItem(&copy_item);

        // Save As...
        let save_item = NSMenuItem::new(mtm);
        save_item.setTitle(&NSString::from_str("Save Image As..."));
        unsafe {
            save_item.setTarget(Some(self));
            save_item.setAction(Some(sel!(saveImageAs:)));
            save_item.setRepresentedObject(Some(&NSNumber::new_u64(image_id)));
        }
        menu.addItem(&save_item);

        // Open
        let open_item = NSMenuItem::new(mtm);
        open_item.setTitle(&NSString::from_str("Open Image"));
        unsafe {
            open_item.setTarget(Some(self));
            open_item.setAction(Some(sel!(openImage:)));
            open_item.setRepresentedObject(Some(&NSNumber::new_u64(image_id)));
        }
        menu.addItem(&open_item);

        // Show the menu
        NSMenu::popUpContextMenu_withEvent_forView(&menu, event, self);
    }

    /// Show context menu for a hyperlink
    fn show_hyperlink_context_menu(&self, event: &NSEvent, url: &str) {
        let mtm = MainThreadMarker::from(self);
        let menu = NSMenu::new(mtm);

        // Create NSString for the URL to use as represented object
        let url_string = NSString::from_str(url);

        // Open URL
        let open_item = NSMenuItem::new(mtm);
        open_item.setTitle(&NSString::from_str("Open URL"));
        unsafe {
            open_item.setTarget(Some(self));
            open_item.setAction(Some(sel!(openURL:)));
            open_item.setRepresentedObject(Some(&*url_string));
        }
        menu.addItem(&open_item);

        // Copy URL
        let copy_item = NSMenuItem::new(mtm);
        copy_item.setTitle(&NSString::from_str("Copy URL"));
        unsafe {
            copy_item.setTarget(Some(self));
            copy_item.setAction(Some(sel!(copyURL:)));
            copy_item.setRepresentedObject(Some(&*url_string));
        }
        menu.addItem(&copy_item);

        // Show the menu
        NSMenu::popUpContextMenu_withEvent_forView(&menu, event, self);
    }

    /// Get URL string from menu item's represented object
    fn get_url_from_menu_item(&self, item: &NSMenuItem) -> Option<String> {
        if let Some(obj) = item.representedObject() {
            // The represented object is an NSString
            let ns_str: &NSString = unsafe { &*(&*obj as *const _ as *const NSString) };
            Some(ns_str.to_string())
        } else {
            None
        }
    }

    /// Get image ID from menu item's represented object
    fn get_image_id_from_menu_item(&self, item: &NSMenuItem) -> u64 {
        if let Some(obj) = item.representedObject() {
            // Try to get the value as NSNumber
            let num: *const NSNumber = &*obj as *const _ as *const _;
            unsafe { (*num).unsignedLongLongValue() }
        } else {
            0
        }
    }

    /// Copy an image to the pasteboard
    fn copy_image_to_pasteboard(&self, image: &cterm_core::TerminalImage) {
        use objc2_app_kit::{NSBitmapImageRep, NSPasteboard};
        use objc2_foundation::NSDictionary;

        let mtm = MainThreadMarker::from(self);
        let pasteboard = NSPasteboard::generalPasteboard();

        // Create NSBitmapImageRep from RGBA data
        let width = image.pixel_width as isize;
        let height = image.pixel_height as isize;
        let data_ptr = image.data.as_ptr();

        unsafe {
            let rep = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                mtm.alloc(),
                std::ptr::null_mut(), // planes - will allocate
                width,
                height,
                8, // bits per sample
                4, // samples per pixel (RGBA)
                true, // has alpha
                false, // not planar
                &NSString::from_str("NSDeviceRGBColorSpace"),
                width * 4, // bytes per row
                32, // bits per pixel
            );

            if let Some(ref rep) = rep {
                // Copy data to the bitmap
                let bitmap_data = rep.bitmapData();
                if !bitmap_data.is_null() {
                    std::ptr::copy_nonoverlapping(
                        data_ptr,
                        bitmap_data,
                        (width * height * 4) as usize,
                    );
                }

                // Get PNG data and put on pasteboard
                let empty_dict: Retained<NSDictionary<NSString, objc2::runtime::AnyObject>> =
                    NSDictionary::new();
                if let Some(png_data) = rep.representationUsingType_properties(
                    objc2_app_kit::NSBitmapImageFileType::PNG,
                    &empty_dict,
                ) {
                    pasteboard.clearContents();
                    pasteboard.setData_forType(Some(&*png_data), &NSString::from_str("public.png"));
                    log::debug!("Copied image to pasteboard");
                }
            }
        }
    }

    /// Save image data as PNG to a path
    fn save_image_as_png(
        &self,
        rgba_data: &[u8],
        width: usize,
        height: usize,
        path: &std::path::Path,
    ) -> std::io::Result<()> {
        use image::ImageEncoder;

        // Use image crate to encode PNG
        let img = image::RgbaImage::from_raw(width as u32, height as u32, rgba_data.to_vec())
            .ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidData, "Invalid image data")
            })?;

        let file = std::fs::File::create(path)?;
        let encoder = image::codecs::png::PngEncoder::new(file);
        encoder
            .write_image(
                &img,
                width as u32,
                height as u32,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(std::io::Error::other)?;

        Ok(())
    }

    /// Restore terminal display state from saved crash state
    #[cfg(unix)]
    pub fn restore_display_state(&self, state: &TerminalUpgradeState) {
        let mut terminal = self.ivars().terminal.lock();

        // Restore scrollback and screen content
        let screen_config = cterm_core::screen::ScreenConfig {
            scrollback_lines: 10000, // Default, could get from config
        };

        let restored_screen = cterm_core::Screen::from_upgrade_state(
            state.grid.clone(),
            state.scrollback.clone(),
            state.alternate_grid.clone(),
            state.cursor.clone(),
            state.saved_cursor.clone(),
            state.alt_saved_cursor.clone(),
            state.scroll_region,
            state.style.clone(),
            state.modes.clone(),
            state.title.clone(),
            state.scroll_offset,
            state.tab_stops.clone(),
            screen_config,
        );

        // Replace the terminal's screen with the restored one
        terminal.restore_screen(restored_screen);

        drop(terminal);

        // Trigger a redraw to show the restored content
        self.set_needs_display();

        log::info!(
            "Restored display state: {}x{}, {} scrollback lines",
            state.cols,
            state.rows,
            state.scrollback.len()
        );
    }
}
