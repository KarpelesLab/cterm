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
    pending_tab_color: RefCell<Option<String>>,
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

            // Apply pending tab color if any (tab property becomes available after joining tab group)
            // Try immediately, and schedule a retry in case the tab isn't ready yet
            if !self.apply_pending_tab_color() {
                self.schedule_tab_color_retry();
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

        /// Retry applying tab color (called via performSelector:afterDelay:)
        #[unsafe(method(retryTabColor))]
        fn retry_tab_color(&self) {
            if !self.apply_pending_tab_color() {
                // Still not ready, try again
                self.schedule_tab_color_retry();
            }
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
            pending_tab_color: RefCell::new(None),
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
            pending_tab_color: RefCell::new(None),
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
            pending_tab_color: RefCell::new(None),
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
            pending_tab_color: RefCell::new(template.color.clone()),
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

    /// Set the tab color indicator for native macOS tabs
    ///
    /// Creates a small colored circle as the tab's accessory view.
    /// If the tab is not yet available, stores the color for later application.
    pub fn set_tab_color(&self, color: Option<&str>) {
        // Store the color for later if needed
        *self.ivars().pending_tab_color.borrow_mut() = color.map(|s| s.to_string());

        unsafe {
            // Get the window's tab object
            let tab: *mut objc2::runtime::AnyObject = msg_send![self, tab];
            if tab.is_null() {
                log::debug!("No tab object available, stored for later");
                return;
            }

            self.apply_tab_color_to_tab(tab, color);
        }
    }

    /// Apply pending tab color if the tab is now available
    /// Returns true if color was applied, false if tab not yet available
    fn apply_pending_tab_color(&self) -> bool {
        let pending = self.ivars().pending_tab_color.borrow().clone();
        if pending.is_none() {
            return true; // Nothing to apply
        }

        unsafe {
            let tab: *mut objc2::runtime::AnyObject = msg_send![self, tab];
            if tab.is_null() {
                log::debug!("Tab not available yet for pending color");
                return false;
            }

            self.apply_tab_color_to_tab(tab, pending.as_deref());
            // Clear pending after successful application
            *self.ivars().pending_tab_color.borrow_mut() = None;
            log::debug!("Applied pending tab color: {:?}", pending);
            true
        }
    }

    /// Schedule a retry for applying tab color after a short delay
    fn schedule_tab_color_retry(&self) {
        unsafe {
            let _: () = msg_send![
                self,
                performSelector: objc2::sel!(retryTabColor),
                withObject: std::ptr::null::<objc2::runtime::AnyObject>(),
                afterDelay: 0.1f64
            ];
        }
    }

    /// Internal: Apply color to a tab object
    unsafe fn apply_tab_color_to_tab(
        &self,
        tab: *mut objc2::runtime::AnyObject,
        color: Option<&str>,
    ) {
        if let Some(hex) = color {
            // Parse hex color
            let hex = hex.trim_start_matches('#');
            if hex.len() == 6 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex[0..2], 16),
                    u8::from_str_radix(&hex[2..4], 16),
                    u8::from_str_radix(&hex[4..6], 16),
                ) {
                    // Create a small colored circle view
                    let frame = NSRect::new(NSPoint::ZERO, NSSize::new(12.0, 12.0));
                    let view: *mut objc2::runtime::AnyObject =
                        msg_send![objc2::class!(NSView), alloc];
                    let view: *mut objc2::runtime::AnyObject =
                        msg_send![view, initWithFrame: frame];

                    // Enable layer-backing and set the background color
                    let _: () = msg_send![view, setWantsLayer: true];
                    let layer: *mut objc2::runtime::AnyObject = msg_send![view, layer];
                    if !layer.is_null() {
                        // Create NSColor from RGB
                        let ns_color: *mut objc2::runtime::AnyObject = msg_send![
                            objc2::class!(NSColor),
                            colorWithRed: (r as f64 / 255.0),
                            green: (g as f64 / 255.0),
                            blue: (b as f64 / 255.0),
                            alpha: 1.0f64
                        ];
                        let cg_color: *mut objc2::runtime::AnyObject = msg_send![ns_color, CGColor];
                        let _: () = msg_send![layer, setBackgroundColor: cg_color];
                        // Make it a circle
                        let _: () = msg_send![layer, setCornerRadius: 6.0f64];
                    }

                    // Add width and height constraints (required since translatesAutoresizingMaskIntoConstraints is false)
                    let width_constraint: *mut objc2::runtime::AnyObject = msg_send![
                        objc2::class!(NSLayoutConstraint),
                        constraintWithItem: view,
                        attribute: 7i64,  // NSLayoutAttributeWidth
                        relatedBy: 0i64,  // NSLayoutRelationEqual
                        toItem: std::ptr::null::<objc2::runtime::AnyObject>(),
                        attribute: 0i64,  // NSLayoutAttributeNotAnAttribute
                        multiplier: 1.0f64,
                        constant: 12.0f64
                    ];
                    let height_constraint: *mut objc2::runtime::AnyObject = msg_send![
                        objc2::class!(NSLayoutConstraint),
                        constraintWithItem: view,
                        attribute: 8i64,  // NSLayoutAttributeHeight
                        relatedBy: 0i64,  // NSLayoutRelationEqual
                        toItem: std::ptr::null::<objc2::runtime::AnyObject>(),
                        attribute: 0i64,  // NSLayoutAttributeNotAnAttribute
                        multiplier: 1.0f64,
                        constant: 12.0f64
                    ];
                    let _: () = msg_send![width_constraint, setActive: true];
                    let _: () = msg_send![height_constraint, setActive: true];

                    // Set as tab's accessory view
                    let _: () = msg_send![tab, setAccessoryView: view];
                    log::debug!("Set tab color to #{}", hex);
                }
            }
        } else {
            // Clear the accessory view
            let null_view: *mut objc2::runtime::AnyObject = std::ptr::null_mut();
            let _: () = msg_send![tab, setAccessoryView: null_view];
        }
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
