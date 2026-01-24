//! Main window implementation for macOS
//!
//! Handles NSWindow creation and management using native macOS window tabbing.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAlertFirstButtonReturn, NSAlertStyle, NSApplication, NSWindow, NSWindowDelegate,
    NSWindowStyleMask, NSWindowTabbingMode,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use cterm_app::config::Config;
use cterm_app::shortcuts::ShortcutManager;
use cterm_ui::theme::Theme;

use cterm_core::Terminal;

use crate::terminal_view::TerminalView;

/// Window state stored in ivars
pub struct CtermWindowIvars {
    config: Config,
    theme: Theme,
    shortcuts: ShortcutManager,
    active_terminal: RefCell<Option<Retained<TerminalView>>>,
}

define_class!(
    #[unsafe(super(NSWindow))]
    #[thread_kind = MainThreadOnly]
    #[name = "CtermWindow"]
    #[ivars = CtermWindowIvars]
    pub struct CtermWindow;

    unsafe impl NSObjectProtocol for CtermWindow {}

    unsafe impl NSWindowDelegate for CtermWindow {
        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _notification: &NSNotification) {
            log::debug!("Window became key");
            // Make the terminal view first responder so it can receive keyboard input
            if let Some(terminal) = self.ivars().active_terminal.borrow().as_ref() {
                self.makeFirstResponder(Some(terminal));
            }
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _notification: &NSNotification) {
            log::debug!("Window resigned key");
        }

        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _sender: &NSWindow) -> objc2::runtime::Bool {
            // Check if there's a foreground process running
            #[cfg(unix)]
            if let Some(terminal) = self.ivars().active_terminal.borrow().as_ref() {
                if terminal.has_foreground_process() {
                    let process_name = terminal
                        .foreground_process_name()
                        .unwrap_or_else(|| "a process".to_string());

                    // Show confirmation dialog
                    return objc2::runtime::Bool::new(self.show_close_confirmation(&process_name));
                }
            }
            objc2::runtime::Bool::YES
        }

        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            log::debug!("Window will close");

            // Notify AppDelegate to remove this window from tracking
            let mtm = MainThreadMarker::from(self);
            let app = NSApplication::sharedApplication(mtm);
            if let Some(delegate) = app.delegate() {
                // Call our custom method to remove the window
                let _: () = unsafe { msg_send![&*delegate, windowDidClose: self] };
            }
        }

        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notification: &NSNotification) {
            log::debug!("Window did resize");
            // Update terminal dimensions
            if let Some(terminal) = self.ivars().active_terminal.borrow().as_ref() {
                terminal.handle_resize();
            }
        }
    }

    // Menu action handlers
    impl CtermWindow {
        #[unsafe(method(newTab:))]
        fn action_new_tab(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.create_new_tab();
        }

        #[unsafe(method(closeTab:))]
        fn action_close_tab(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.close_current_tab();
        }

        /// Called by macOS native tabbing when Command-T or tab bar + is pressed.
        /// Returns a new default window (not a template duplicate).
        #[unsafe(method(newWindowForTab:))]
        fn new_window_for_tab(&self, _sender: Option<&objc2::runtime::AnyObject>) -> *mut NSWindow {
            let mtm = MainThreadMarker::from(self);
            let new_window = CtermWindow::new(mtm, &self.ivars().config, &self.ivars().theme);

            // Register with AppDelegate for tracking
            let app = NSApplication::sharedApplication(mtm);
            if let Some(delegate) = app.delegate() {
                let _: () = unsafe { msg_send![&*delegate, registerWindow: &*new_window] };
            }

            // Explicitly add to tab group (macOS automatic tabbing doesn't always work)
            self.addTabbedWindow_ordered(&new_window, objc2_app_kit::NSWindowOrderingMode::Above);

            // Make the new tab key and visible
            new_window.makeKeyAndOrderFront(None);

            log::info!("Created new default tab via newWindowForTab:");
            Retained::into_raw(Retained::into_super(new_window))
        }
    }
);

impl CtermWindow {
    pub fn new(mtm: MainThreadMarker, config: &Config, theme: &Theme) -> Retained<Self> {
        // Calculate initial window size for 80x24 terminal
        let cell_width = config.appearance.font.size * 0.6; // Approximate
        let cell_height = config.appearance.font.size * 1.2;
        let width = cell_width * 80.0;
        let height = cell_height * 24.0;

        let content_rect = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(width, height));

        let style_mask = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;

        // Allocate and initialize
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(CtermWindowIvars {
            config: config.clone(),
            theme: theme.clone(),
            shortcuts: ShortcutManager::from_config(&config.shortcuts),
            active_terminal: RefCell::new(None),
        });

        let this: Retained<Self> = unsafe {
            msg_send![
                super(this),
                initWithContentRect: content_rect,
                styleMask: style_mask,
                backing: 2u64, // NSBackingStoreBuffered
                defer: false
            ]
        };

        // Set window title
        this.setTitle(&NSString::from_str("cterm"));

        // Set minimum size
        this.setMinSize(NSSize::new(400.0, 200.0));

        // Prevent macOS from releasing window on close (we manage lifetime)
        unsafe { this.setReleasedWhenClosed(false) };

        // Enable native macOS window tabbing
        this.setTabbingMode(NSWindowTabbingMode::Preferred);

        // Set self as delegate
        this.setDelegate(Some(ProtocolObject::from_ref(&*this)));

        // Create the terminal view
        let terminal = TerminalView::new(mtm, config, theme);
        this.setContentView(Some(&terminal));

        // Set content resize increments to snap to character grid
        let (cell_width, cell_height) = terminal.cell_size();
        this.setContentResizeIncrements(NSSize::new(cell_width, cell_height));

        *this.ivars().active_terminal.borrow_mut() = Some(terminal);

        this
    }

    /// Create a window from a restored Terminal (for seamless upgrades)
    #[cfg(unix)]
    pub fn from_restored(
        mtm: MainThreadMarker,
        config: &Config,
        theme: &Theme,
        terminal: Terminal,
    ) -> Retained<Self> {
        // Calculate initial window size for 80x24 terminal
        let cell_width = config.appearance.font.size * 0.6;
        let cell_height = config.appearance.font.size * 1.2;
        let width = cell_width * 80.0;
        let height = cell_height * 24.0;

        let content_rect = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(width, height));

        let style_mask = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;

        // Allocate and initialize
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(CtermWindowIvars {
            config: config.clone(),
            theme: theme.clone(),
            shortcuts: ShortcutManager::from_config(&config.shortcuts),
            active_terminal: RefCell::new(None),
        });

        let this: Retained<Self> = unsafe {
            msg_send![
                super(this),
                initWithContentRect: content_rect,
                styleMask: style_mask,
                backing: 2u64,
                defer: false
            ]
        };

        // Set window title from restored terminal
        let title = {
            let term = terminal.screen();
            if term.title.is_empty() {
                "cterm".to_string()
            } else {
                term.title.clone()
            }
        };
        this.setTitle(&NSString::from_str(&title));

        // Set minimum size
        this.setMinSize(NSSize::new(400.0, 200.0));

        // Prevent macOS from releasing window on close (we manage lifetime)
        unsafe { this.setReleasedWhenClosed(false) };

        // Enable native macOS window tabbing
        this.setTabbingMode(NSWindowTabbingMode::Preferred);

        // Set self as delegate
        this.setDelegate(Some(ProtocolObject::from_ref(&*this)));

        // Create the terminal view from the restored terminal
        let terminal_view = TerminalView::from_restored(mtm, config, theme, terminal);
        this.setContentView(Some(&terminal_view));

        // Set content resize increments to snap to character grid
        let (cell_width, cell_height) = terminal_view.cell_size();
        this.setContentResizeIncrements(NSSize::new(cell_width, cell_height));

        *this.ivars().active_terminal.borrow_mut() = Some(terminal_view);

        this
    }

    /// Create a window from a recovered FD (for crash recovery)
    #[cfg(unix)]
    pub fn from_recovered_fd(
        mtm: MainThreadMarker,
        config: &Config,
        theme: &Theme,
        recovered: &cterm_app::RecoveredFd,
    ) -> Retained<Self> {
        // Calculate initial window size for 80x24 terminal
        let cell_width = config.appearance.font.size * 0.6;
        let cell_height = config.appearance.font.size * 1.2;
        let width = cell_width * 80.0;
        let height = cell_height * 24.0;

        let content_rect = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(width, height));

        let style_mask = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;

        // Allocate and initialize
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(CtermWindowIvars {
            config: config.clone(),
            theme: theme.clone(),
            shortcuts: ShortcutManager::from_config(&config.shortcuts),
            active_terminal: RefCell::new(None),
        });

        let this: Retained<Self> = unsafe {
            msg_send![
                super(this),
                initWithContentRect: content_rect,
                styleMask: style_mask,
                backing: 2u64,
                defer: false
            ]
        };

        // Set window title for recovered terminal
        this.setTitle(&NSString::from_str("cterm"));

        // Set minimum size
        this.setMinSize(NSSize::new(400.0, 200.0));

        // Prevent macOS from releasing window on close (we manage lifetime)
        unsafe { this.setReleasedWhenClosed(false) };

        // Enable native macOS window tabbing
        this.setTabbingMode(NSWindowTabbingMode::Preferred);

        // Set self as delegate
        this.setDelegate(Some(ProtocolObject::from_ref(&*this)));

        // Create the terminal view from the recovered FD
        let terminal_view = TerminalView::from_recovered_fd(mtm, config, theme, recovered);
        this.setContentView(Some(&terminal_view));

        // Set content resize increments to snap to character grid
        let (cell_width, cell_height) = terminal_view.cell_size();
        this.setContentResizeIncrements(NSSize::new(cell_width, cell_height));

        *this.ivars().active_terminal.borrow_mut() = Some(terminal_view);

        this
    }

    /// Create a new tab (using native macOS window tabbing)
    pub fn create_new_tab(&self) {
        let mtm = MainThreadMarker::from(self);

        // Create a new window with the same configuration
        let new_window = CtermWindow::new(mtm, &self.ivars().config, &self.ivars().theme);

        // Register with AppDelegate for tracking (important for relaunch/upgrade)
        let app = NSApplication::sharedApplication(mtm);
        if let Some(delegate) = app.delegate() {
            let _: () = unsafe { msg_send![&*delegate, registerWindow: &*new_window] };
        }

        // Add the new window as a tab to this window
        self.addTabbedWindow_ordered(&new_window, objc2_app_kit::NSWindowOrderingMode::Above);

        // Make the new tab's window key
        new_window.makeKeyAndOrderFront(None);

        log::info!("Created new tab");
    }

    /// Close current tab
    pub fn close_current_tab(&self) {
        // With native tabbing, just close the window
        // macOS will handle showing the next tab
        self.close();
    }

    /// Get config reference
    pub fn config(&self) -> &Config {
        &self.ivars().config
    }

    /// Get theme reference
    pub fn theme(&self) -> &Theme {
        &self.ivars().theme
    }

    /// Get a reference to the active terminal view
    pub fn active_terminal(&self) -> Option<Retained<TerminalView>> {
        self.ivars().active_terminal.borrow().clone()
    }

    /// Create a window from a tab template
    pub fn from_template(
        mtm: MainThreadMarker,
        config: &Config,
        theme: &Theme,
        template: &cterm_app::config::StickyTabConfig,
    ) -> Retained<Self> {
        // Calculate initial window size for 80x24 terminal
        let cell_width = config.appearance.font.size * 0.6;
        let cell_height = config.appearance.font.size * 1.2;
        let width = cell_width * 80.0;
        let height = cell_height * 24.0;

        let content_rect = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(width, height));

        let style_mask = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;

        // Allocate and initialize
        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(CtermWindowIvars {
            config: config.clone(),
            theme: theme.clone(),
            shortcuts: ShortcutManager::from_config(&config.shortcuts),
            active_terminal: RefCell::new(None),
        });

        let this: Retained<Self> = unsafe {
            msg_send![
                super(this),
                initWithContentRect: content_rect,
                styleMask: style_mask,
                backing: 2u64,
                defer: false
            ]
        };

        // Set window title from template
        this.setTitle(&NSString::from_str(&template.name));

        // Set minimum size
        this.setMinSize(NSSize::new(400.0, 200.0));

        // Prevent macOS from releasing window on close (we manage lifetime)
        unsafe { this.setReleasedWhenClosed(false) };

        // Enable native macOS window tabbing
        this.setTabbingMode(NSWindowTabbingMode::Preferred);

        // Set self as delegate
        this.setDelegate(Some(ProtocolObject::from_ref(&*this)));

        // Create the terminal view from template
        let terminal_view = TerminalView::from_template(mtm, config, theme, template);
        this.setContentView(Some(&terminal_view));

        // Set content resize increments to snap to character grid
        let (cell_width, cell_height) = terminal_view.cell_size();
        this.setContentResizeIncrements(NSSize::new(cell_width, cell_height));

        *this.ivars().active_terminal.borrow_mut() = Some(terminal_view);

        this
    }

    /// Show a confirmation dialog when closing with a running process
    fn show_close_confirmation(&self, process_name: &str) -> bool {
        use objc2_app_kit::NSAlert;

        let mtm = MainThreadMarker::from(self);
        let alert = NSAlert::new(mtm);

        alert.setMessageText(&NSString::from_str(&format!(
            "\"{}\" is still running",
            process_name
        )));
        alert.setInformativeText(&NSString::from_str(
            "Closing this terminal will terminate the running process. Are you sure you want to close?",
        ));
        alert.setAlertStyle(NSAlertStyle::Warning);

        alert.addButtonWithTitle(&NSString::from_str("Close"));
        alert.addButtonWithTitle(&NSString::from_str("Cancel"));

        let response = alert.runModal();
        response == NSAlertFirstButtonReturn
    }
}
