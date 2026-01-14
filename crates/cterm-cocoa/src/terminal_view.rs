//! Terminal view implementation for macOS
//!
//! NSView subclass that renders the terminal using CoreGraphics.

use std::cell::RefCell;
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, define_class, msg_send, sel, DefinedClass, MainThreadOnly};
use objc2_app_kit::{NSEvent, NSView};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSPoint, NSRect, NSSize};
use parking_lot::Mutex;

use cterm_app::config::Config;
use cterm_core::screen::ScreenConfig;
use cterm_core::{Pty, PtyConfig, PtySize, Terminal};
use cterm_ui::theme::Theme;

use crate::cg_renderer::CGRenderer;
use crate::keycode;

/// Terminal view state
pub struct TerminalViewIvars {
    terminal: Arc<Mutex<Terminal>>,
    pty: RefCell<Option<Pty>>,
    renderer: RefCell<Option<CGRenderer>>,
    cell_width: f64,
    cell_height: f64,
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

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty_rect: NSRect) {
            if let Some(ref renderer) = *self.ivars().renderer.borrow() {
                let terminal = self.ivars().terminal.lock();
                renderer.render(&terminal, dirty_rect);
            }
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            // Get the key code
            if let Some(keycode) = keycode::keycode_from_event(event) {
                let modifiers = keycode::modifiers_from_event(event);
                log::trace!("Key down: {:?} modifiers: {:?}", keycode, modifiers);
            }

            // Get the characters and write to PTY
            if let Some(chars) = keycode::characters_from_event(event) {
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
            // TODO: Handle mouse selection
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
            // TODO: Handle selection drag
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            let delta_y = event.scrollingDeltaY();
            log::trace!("Scroll wheel delta: {}", delta_y);

            // Scroll the terminal
            let mut terminal = self.ivars().terminal.lock();
            if delta_y > 0.0 {
                terminal.scroll_viewport_up(delta_y.abs() as usize);
            } else if delta_y < 0.0 {
                terminal.scroll_viewport_down(delta_y.abs() as usize);
            }
            drop(terminal);

            // Request redraw
            self.set_needs_display();
        }

        #[unsafe(method(refreshTimerFired:))]
        fn refresh_timer_fired(&self, _timer: &AnyObject) {
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
        });

        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };

        // Spawn shell
        this.spawn_shell(config);

        // Start refresh timer for PTY updates
        this.start_refresh_timer();

        this
    }

    /// Spawn the shell process
    fn spawn_shell(&self, config: &Config) {
        let shell = config
            .general
            .default_shell
            .clone()
            .unwrap_or_else(|| std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string()));

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
                    Self::read_pty_loop(pty_fd, terminal);
                });

                *self.ivars().pty.borrow_mut() = Some(pty);
            }
            Err(e) => {
                log::error!("Failed to spawn shell: {}", e);
            }
        }
    }

    /// Background thread to read from PTY
    fn read_pty_loop(pty_fd: i32, terminal: Arc<Mutex<Terminal>>) {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;

        let mut file = unsafe { std::fs::File::from_raw_fd(pty_fd) };
        let mut buf = [0u8; 4096];

        loop {
            match file.read(&mut buf) {
                Ok(0) => {
                    log::info!("PTY closed");
                    break;
                }
                Ok(n) => {
                    let mut term = terminal.lock();
                    term.process(&buf[..n]);
                    drop(term);
                }
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::Interrupted {
                        log::error!("PTY read error: {}", e);
                        break;
                    }
                }
            }
        }

        // Don't close the fd - it's owned by the Pty struct
        std::mem::forget(file);
    }

    /// Start a timer to periodically refresh the display
    fn start_refresh_timer(&self) {
        unsafe {
            let interval: f64 = 1.0 / 60.0; // 60 FPS
            let _: Retained<AnyObject> = msg_send![
                class!(NSTimer),
                scheduledTimerWithTimeInterval: interval,
                target: self,
                selector: sel!(refreshTimerFired:),
                userInfo: std::ptr::null::<AnyObject>(),
                repeats: true
            ];
        }
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
