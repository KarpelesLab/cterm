//! Terminal view implementation for macOS
//!
//! NSView subclass that renders the terminal using CoreGraphics.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSEvent, NSView};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSPoint, NSRect, NSSize};
use parking_lot::Mutex;

use cterm_app::config::Config;
use cterm_core::screen::ScreenConfig;
use cterm_core::{Pty, PtyConfig, PtySize, Terminal};
use cterm_ui::theme::Theme;

use crate::cg_renderer::CGRenderer;
use crate::keycode;

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
            }
        }

        #[unsafe(method(viewWillMoveToWindow:))]
        fn view_will_move_to_window(&self, new_window: Option<&objc2_app_kit::NSWindow>) {
            // If moving to nil window (being removed), mark view as invalid
            // This tells background threads to stop using the view pointer
            if new_window.is_none() {
                log::debug!("View being removed from window, marking invalid");
                self.ivars().state.view_invalid.store(true, Ordering::SeqCst);
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
            let modifiers = keycode::modifiers_from_event(event);

            // Let Command+key combinations pass through to the menu system
            // Command is never part of terminal sequences
            if modifiers.contains(cterm_ui::events::Modifiers::SUPER) {
                // Don't handle - let the responder chain process it for menu shortcuts
                return;
            }

            // Get the characters and write to PTY
            if let Some(chars) = keycode::characters_from_event(event) {
                log::debug!("Writing to PTY: {:?}", chars);
                if let Some(ref mut pty) = *self.ivars().pty.borrow_mut() {
                    if let Err(e) = pty.write(chars.as_bytes()) {
                        log::error!("Failed to write to PTY: {}", e);
                    }
                }
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let location = event.locationInWindow();
            log::trace!("Mouse down at: {:?}", location);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let location = event.locationInWindow();
            log::trace!("Mouse up at: {:?}", location);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let location = event.locationInWindow();
            log::trace!("Mouse dragged at: {:?}", location);
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let delta_y = event.scrollingDeltaY();
            log::trace!("Scroll wheel delta: {}", delta_y);

            let mut terminal = self.ivars().terminal.lock();
            if delta_y > 0.0 {
                terminal.scroll_viewport_up(delta_y.abs() as usize);
            } else if delta_y < 0.0 {
                terminal.scroll_viewport_down(delta_y.abs() as usize);
            }
            drop(terminal);

            self.set_needs_display();
        }

        #[unsafe(method(triggerRedraw))]
        fn trigger_redraw(&self) {
            self.set_needs_display();
        }
    }
);

impl TerminalView {
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
        });

        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };

        // Store view pointer as usize for thread-safe passing
        // Safety: We only use this pointer on the main thread via dispatch
        let view_ptr = &*this as *const _ as usize;

        // Spawn shell
        this.spawn_shell(config, state.clone());

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

        let cols = (frame.size.width / cell_width).floor() as usize;
        let rows = (frame.size.height / cell_height).floor() as usize;

        if cols > 0 && rows > 0 {
            let mut terminal = self.ivars().terminal.lock();
            terminal.resize(cols, rows);
            drop(terminal);

            if let Some(ref mut pty) = *self.ivars().pty.borrow_mut() {
                let _ = pty.resize(cols as u16, rows as u16);
            }

            log::debug!("Resized terminal to {}x{}", cols, rows);
        }
    }

    /// Get the terminal
    pub fn terminal(&self) -> &Arc<Mutex<Terminal>> {
        &self.ivars().terminal
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
}
