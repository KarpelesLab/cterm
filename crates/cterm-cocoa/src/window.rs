//! Main window implementation for macOS
//!
//! Handles NSWindow creation and management using native macOS window tabbing.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, DefinedClass, MainThreadOnly};
use objc2_app_kit::{
    NSAlertFirstButtonReturn, NSAlertStyle, NSApplication, NSMenu, NSMenuItem, NSWindow,
    NSWindowDelegate, NSWindowStyleMask, NSWindowTabbingMode,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSNotification, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};

use cterm_app::config::Config;
use cterm_app::shortcuts::ShortcutManager;
use cterm_ui::theme::Theme;

use crate::quick_open::{OpenTabEntry, QuickOpenOverlay, QUICK_OPEN_HEIGHT};
use crate::terminal_view::TerminalView;

/// Window state stored in ivars
pub struct CtermWindowIvars {
    config: Config,
    theme: Theme,
    shortcuts: ShortcutManager,
    active_terminal: RefCell<Option<Retained<TerminalView>>>,
    pending_tab_color: RefCell<Option<String>>,
    quick_open: RefCell<Option<Retained<QuickOpenOverlay>>>,
    /// Whether this window has an active bell notification
    has_active_bell: std::cell::Cell<bool>,
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
                // Send focus in event if DECSET 1004 is enabled
                terminal.send_focus_event(true);
            }

            // Clear bell indicator from window title if present
            let current_title: Retained<NSString> = unsafe { msg_send![self, title] };
            let title_str = current_title.to_string();
            if let Some(stripped) = title_str.strip_prefix("🔔 ") {
                self.setTitle(&NSString::from_str(stripped));
            }

            // Clear bell state and update dock badge
            self.set_bell(false);

            // Apply pending tab color if any (tab property becomes available after joining tab group)
            // Try immediately, and schedule a retry in case the tab isn't ready yet
            if !self.apply_pending_tab_color() {
                self.schedule_tab_color_retry();
            }
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _notification: &NSNotification) {
            log::debug!("Window resigned key");
            // Send focus out event if DECSET 1004 is enabled
            if let Some(terminal) = self.ivars().active_terminal.borrow().as_ref() {
                terminal.send_focus_event(false);
            }
        }

        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _sender: &NSWindow) -> objc2::runtime::Bool {
            // Check if config says to confirm close with running processes
            if !self.ivars().config.general.confirm_close_with_running {
                return objc2::runtime::Bool::YES;
            }

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

            // Update Quick Open overlay width
            if let Some(ref overlay) = *self.ivars().quick_open.borrow() {
                let width = self.frame().size.width;
                overlay.update_width(width);
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

            // Get the current working directory from the active terminal
            #[cfg(unix)]
            let cwd = self
                .ivars()
                .active_terminal
                .borrow()
                .as_ref()
                .and_then(|t| t.foreground_cwd());
            #[cfg(not(unix))]
            let cwd: Option<String> = None;

            let new_window =
                CtermWindow::new_with_cwd(mtm, &self.ivars().config, &self.ivars().theme, cwd);

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

        /// Set tab color via color picker dialog
        #[unsafe(method(setTabColor:))]
        fn action_set_tab_color(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            let mtm = MainThreadMarker::from(self);
            let current = self.ivars().pending_tab_color.borrow().clone();
            match crate::dialogs::show_color_picker_dialog(mtm, current.as_deref()) {
                crate::dialogs::ColorPickerResult::Color(color) => {
                    self.set_tab_color(Some(&color));
                }
                crate::dialogs::ColorPickerResult::Clear => {
                    self.set_tab_color(None);
                }
                crate::dialogs::ColorPickerResult::Cancel => {
                    // Do nothing
                }
            }
        }

        // Window positioning actions
        #[unsafe(method(windowFill:))]
        fn action_window_fill(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.position_fill();
        }

        #[unsafe(method(windowCenter:))]
        fn action_window_center(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.position_center();
        }

        #[unsafe(method(windowLeftHalf:))]
        fn action_window_left_half(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.position_left_half();
        }

        #[unsafe(method(windowRightHalf:))]
        fn action_window_right_half(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.position_right_half();
        }

        #[unsafe(method(windowTopHalf:))]
        fn action_window_top_half(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.position_top_half();
        }

        #[unsafe(method(windowBottomHalf:))]
        fn action_window_bottom_half(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.position_bottom_half();
        }

        #[unsafe(method(windowTopLeftQuarter:))]
        fn action_window_top_left_quarter(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.position_top_left_quarter();
        }

        #[unsafe(method(windowTopRightQuarter:))]
        fn action_window_top_right_quarter(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.position_top_right_quarter();
        }

        #[unsafe(method(windowBottomLeftQuarter:))]
        fn action_window_bottom_left_quarter(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.position_bottom_left_quarter();
        }

        #[unsafe(method(windowBottomRightQuarter:))]
        fn action_window_bottom_right_quarter(&self, _sender: Option<&objc2::runtime::AnyObject>) {
            self.position_bottom_right_quarter();
        }
    }
);

/// Approximate ratio of cell width to font size
const CELL_WIDTH_RATIO: f64 = 0.6;
/// Approximate ratio of cell height to font size
const CELL_HEIGHT_RATIO: f64 = 1.2;

impl CtermWindow {
    /// Common window initialization: calculate size, allocate, init NSWindow,
    /// set min size, tabbing mode, and delegate.
    fn init_window(
        mtm: MainThreadMarker,
        config: &Config,
        theme: &Theme,
        title: &str,
        pending_tab_color: Option<String>,
    ) -> Retained<Self> {
        let cell_width = config.appearance.font.size * CELL_WIDTH_RATIO;
        let cell_height = config.appearance.font.size * CELL_HEIGHT_RATIO;
        let width = cell_width * 80.0;
        let height = cell_height * 24.0;

        let content_rect = NSRect::new(NSPoint::new(200.0, 200.0), NSSize::new(width, height));
        let style_mask = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;

        let this = mtm.alloc::<Self>();
        let this = this.set_ivars(CtermWindowIvars {
            config: config.clone(),
            theme: theme.clone(),
            shortcuts: ShortcutManager::from_config(&config.shortcuts),
            active_terminal: RefCell::new(None),
            pending_tab_color: RefCell::new(pending_tab_color),
            quick_open: RefCell::new(None),
            has_active_bell: std::cell::Cell::new(false),
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

        this.setTitle(&NSString::from_str(title));
        this.setMinSize(NSSize::new(400.0, 200.0));
        unsafe { this.setReleasedWhenClosed(false) };
        this.setTabbingMode(NSWindowTabbingMode::Preferred);
        this.setDelegate(Some(ProtocolObject::from_ref(&*this)));

        this
    }

    /// Attach a terminal view to this window as content and store it
    fn attach_terminal_view(&self, terminal: Retained<TerminalView>) {
        self.setContentView(Some(&terminal));
        let (cell_width, cell_height) = terminal.cell_size();
        self.setContentResizeIncrements(NSSize::new(cell_width, cell_height));
        *self.ivars().active_terminal.borrow_mut() = Some(terminal);
    }

    pub fn new(mtm: MainThreadMarker, config: &Config, theme: &Theme) -> Retained<Self> {
        Self::new_with_cwd(mtm, config, theme, None)
    }

    pub fn new_with_cwd(
        mtm: MainThreadMarker,
        config: &Config,
        theme: &Theme,
        cwd: Option<String>,
    ) -> Retained<Self> {
        let this = Self::init_window(mtm, config, theme, "Terminal", None);
        this.spawn_initial_daemon_session(cwd);
        this
    }

    /// Spawn a daemon session in the background and attach the terminal when ready.
    /// Used for initial window creation where the window must exist immediately.
    fn spawn_initial_daemon_session(&self, cwd: Option<String>) {
        let config = self.ivars().config.clone();
        let opts = cterm_client::CreateSessionOpts {
            cols: 80,
            rows: 24,
            shell: config.general.default_shell.clone(),
            args: config.general.shell_args.clone(),
            cwd,
            ..Default::default()
        };
        self.spawn_initial_daemon_session_with_opts(opts, None);
    }

    /// Spawn a daemon session with custom options in the background and attach when ready.
    fn spawn_initial_daemon_session_with_opts(
        &self,
        opts: cterm_client::CreateSessionOpts,
        background_color: Option<String>,
    ) {
        let config = self.ivars().config.clone();
        let theme = self.ivars().theme.clone();
        let window_ptr = self as *const Self as usize;

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();

            let result = match rt {
                Ok(rt) => rt.block_on(async {
                    let conn = cterm_client::DaemonConnection::connect_local().await?;
                    let session = conn.create_session(opts).await?;
                    Ok::<_, cterm_client::ClientError>(session)
                }),
                Err(e) => Err(cterm_client::ClientError::Connection(e.to_string())),
            };

            match result {
                Ok(session) => {
                    dispatch2::Queue::main().exec_async(move || {
                        let mtm = unsafe { MainThreadMarker::new_unchecked() };
                        let window: &CtermWindow = unsafe { &*(window_ptr as *const CtermWindow) };
                        let terminal_view =
                            TerminalView::from_daemon(mtm, &config, &theme, session);
                        if let Some(ref bg) = background_color {
                            terminal_view.set_background_override(Some(bg));
                        }
                        window.attach_terminal_view(terminal_view);
                    });
                }
                Err(e) => {
                    log::error!("Failed to create initial daemon session: {}", e);
                }
            }
        });
    }

    /// Create a window and spawn a daemon session with specific options
    pub fn new_daemon(
        mtm: MainThreadMarker,
        config: &Config,
        theme: &Theme,
        opts: cterm_client::CreateSessionOpts,
        title: String,
        color: Option<String>,
        background_color: Option<String>,
    ) -> Retained<Self> {
        let this = Self::init_window(mtm, config, theme, &title, color.clone());
        this.spawn_initial_daemon_session_with_opts(opts, background_color);
        this
    }

    /// Create a window connected to a daemon session
    pub fn from_daemon(
        mtm: MainThreadMarker,
        config: &Config,
        theme: &Theme,
        session: cterm_client::SessionHandle,
    ) -> Retained<Self> {
        let this = Self::init_window(mtm, config, theme, "Terminal", None);
        let terminal_view = TerminalView::from_daemon(mtm, config, theme, session);
        this.attach_terminal_view(terminal_view);
        this
    }

    /// Create a window connected to a reconnected daemon session (with screen snapshot)
    pub fn from_daemon_with_screen(
        mtm: MainThreadMarker,
        config: &Config,
        theme: &Theme,
        recon: cterm_app::daemon_reconnect::ReconnectedSession,
    ) -> Retained<Self> {
        let has_custom_title = !recon.custom_title.is_empty();
        let title = if has_custom_title {
            recon.custom_title.clone()
        } else if !recon.title.is_empty() {
            recon.title.clone()
        } else {
            "Terminal".to_string()
        };
        let tab_color = if recon.tab_color.is_empty() {
            None
        } else {
            Some(recon.tab_color.clone())
        };
        let this = Self::init_window(mtm, config, theme, &title, tab_color);
        let terminal_view = TerminalView::from_daemon_with_screen(mtm, config, theme, recon);
        if has_custom_title {
            terminal_view.set_title_locked(true);
        }
        this.attach_terminal_view(terminal_view);
        this
    }

    /// Create a window connected to a mosh session
    pub fn from_mosh(
        mtm: MainThreadMarker,
        config: &Config,
        theme: &Theme,
        session: cterm_mosh::MoshSession,
    ) -> Retained<Self> {
        let this = Self::init_window(mtm, config, theme, "Mosh", None);
        let terminal_view = TerminalView::from_mosh(mtm, config, theme, session);
        this.attach_terminal_view(terminal_view);
        this
    }

    /// Create a new tab connected to a daemon session (using native macOS window tabbing)
    pub fn create_daemon_tab(&self, session: cterm_client::SessionHandle) {
        let mtm = MainThreadMarker::from(self);

        let new_window =
            CtermWindow::from_daemon(mtm, &self.ivars().config, &self.ivars().theme, session);

        // Register with AppDelegate
        let app = NSApplication::sharedApplication(mtm);
        if let Some(delegate) = app.delegate() {
            let _: () = unsafe { msg_send![&*delegate, registerWindow: &*new_window] };
        }

        // Add as tab to this window
        self.addTabbedWindow_ordered(&new_window, objc2_app_kit::NSWindowOrderingMode::Above);
        new_window.makeKeyAndOrderFront(None);

        log::info!("Created daemon tab");
    }

    /// Create a new tab (daemon-backed via ctermd)
    pub fn create_new_tab(&self) {
        // Get the current working directory from the active terminal
        #[cfg(unix)]
        let cwd = self
            .ivars()
            .active_terminal
            .borrow()
            .as_ref()
            .and_then(|t| t.foreground_cwd());
        #[cfg(not(unix))]
        let cwd: Option<String> = None;

        let config = self.ivars().config.clone();
        let opts = cterm_client::CreateSessionOpts {
            cols: 80,
            rows: 24,
            shell: config.general.default_shell.clone(),
            args: config.general.shell_args.clone(),
            cwd,
            ..Default::default()
        };

        self.spawn_daemon_tab(opts, None, None, None, None);
    }

    /// Spawn a daemon session in a background thread and create a tab when ready.
    ///
    /// If `remote` is `Some((manager, name, host))`, the session is created on
    /// the remote ctermd (connecting via SSH if needed). Otherwise it uses the
    /// local daemon.
    pub fn spawn_daemon_tab(
        &self,
        opts: cterm_client::CreateSessionOpts,
        template_name: Option<String>,
        color: Option<String>,
        background_color: Option<String>,
        remote: Option<(cterm_client::RemoteManager, String, String)>,
    ) {
        let config = self.ivars().config.clone();
        let theme = self.ivars().theme.clone();
        // SAFETY: self is MainThreadOnly, we use the raw pointer only inside
        // dispatch2::Queue::main().exec_async() which runs on the main thread
        let window_ptr = self as *const Self as usize;

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();

            let result = match rt {
                Ok(rt) => rt.block_on(async {
                    let conn = if let Some((mgr, ref name, ref host)) = remote {
                        mgr.get_or_connect(name, host).await?
                    } else {
                        cterm_client::DaemonConnection::connect_local().await?
                    };
                    let session = conn.create_session(opts).await?;
                    Ok::<_, cterm_client::ClientError>(session)
                }),
                Err(e) => Err(cterm_client::ClientError::Connection(e.to_string())),
            };

            match result {
                Ok(session) => {
                    dispatch2::Queue::main().exec_async(move || {
                        let mtm = unsafe { MainThreadMarker::new_unchecked() };
                        let window: &CtermWindow = unsafe { &*(window_ptr as *const CtermWindow) };

                        let title = template_name
                            .clone()
                            .unwrap_or_else(|| "Terminal".to_string());

                        let new_window = CtermWindow::from_daemon(mtm, &config, &theme, session);
                        new_window.setTitle(&NSString::from_str(&title));

                        // Store template name and apply background color on the terminal view
                        if let Some(tv) = new_window.active_terminal() {
                            if let Some(ref name) = template_name {
                                tv.set_template_name(Some(name.clone()));
                                tv.set_template_name_on_daemon(name);
                            }
                            if let Some(ref bg) = background_color {
                                tv.set_background_override(Some(bg));
                            }
                        }

                        let app = NSApplication::sharedApplication(mtm);
                        if let Some(delegate) = app.delegate() {
                            let _: () =
                                unsafe { msg_send![&*delegate, registerWindow: &*new_window] };
                        }

                        window.addTabbedWindow_ordered(
                            &new_window,
                            objc2_app_kit::NSWindowOrderingMode::Above,
                        );
                        new_window.makeKeyAndOrderFront(None);

                        if let Some(ref c) = color {
                            new_window.set_tab_color(Some(c));
                        }

                        log::info!("Created daemon tab: {}", title);
                    });
                }
                Err(e) => {
                    log::error!("Failed to create daemon session: {}", e);
                }
            }
        });
    }

    /// Spawn a new tab backed by a mosh session.
    ///
    /// Launches mosh-server via SSH in a background thread, then creates the
    /// terminal view on the main thread when the connection is established.
    pub fn spawn_mosh_tab(
        &self,
        mosh_config: cterm_mosh::MoshConfig,
        template_name: Option<String>,
        color: Option<String>,
        background_color: Option<String>,
    ) {
        let config = self.ivars().config.clone();
        let theme = self.ivars().theme.clone();
        let window_ptr = self as *const Self as usize;

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();

            let result = match rt {
                Ok(rt) => {
                    rt.block_on(async { cterm_mosh::MoshSession::connect(mosh_config).await })
                }
                Err(e) => Err(cterm_mosh::MoshError::SshFailed(e.to_string())),
            };

            match result {
                Ok(session) => {
                    dispatch2::Queue::main().exec_async(move || {
                        let mtm = unsafe { MainThreadMarker::new_unchecked() };
                        let window: &CtermWindow = unsafe { &*(window_ptr as *const CtermWindow) };

                        let title = template_name.clone().unwrap_or_else(|| "Mosh".to_string());

                        let new_window = CtermWindow::from_mosh(mtm, &config, &theme, session);
                        new_window.setTitle(&NSString::from_str(&title));

                        if let Some(tv) = new_window.active_terminal() {
                            if let Some(ref name) = template_name {
                                tv.set_template_name(Some(name.clone()));
                            }
                            if let Some(ref bg) = background_color {
                                tv.set_background_override(Some(bg));
                            }
                        }

                        let app = NSApplication::sharedApplication(mtm);
                        if let Some(delegate) = app.delegate() {
                            let _: () =
                                unsafe { msg_send![&*delegate, registerWindow: &*new_window] };
                        }

                        window.addTabbedWindow_ordered(
                            &new_window,
                            objc2_app_kit::NSWindowOrderingMode::Above,
                        );
                        new_window.makeKeyAndOrderFront(None);

                        if let Some(ref c) = color {
                            new_window.set_tab_color(Some(c));
                        }

                        log::info!("Created mosh tab: {}", title);
                    });
                }
                Err(e) => {
                    log::error!("Failed to create mosh session: {}", e);
                }
            }
        });
    }

    /// Close current tab
    pub fn close_current_tab(&self) {
        // With native tabbing, just close the window
        // macOS will handle showing the next tab
        // Use performClose to trigger windowShouldClose: delegate method
        self.performClose(None);
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

    /// Set the bell state for this window and update dock badge
    pub fn set_bell(&self, active: bool) {
        let was_active = self.ivars().has_active_bell.get();
        if active == was_active {
            return; // No change
        }
        self.ivars().has_active_bell.set(active);

        let mtm = MainThreadMarker::from(self);
        let app = NSApplication::sharedApplication(mtm);
        if let Some(delegate) = app.delegate() {
            // Cast to our AppDelegate type via raw pointer
            let delegate_ptr = Retained::as_ptr(&delegate) as *const crate::app::AppDelegate;
            let app_delegate: &crate::app::AppDelegate = unsafe { &*delegate_ptr };
            if active {
                app_delegate.increment_bell_count();
            } else {
                app_delegate.decrement_bell_count();
            }
        }
    }

    /// Check if this window has an active bell notification
    pub fn has_bell(&self) -> bool {
        self.ivars().has_active_bell.get()
    }

    /// Show the Quick Open overlay for template selection and tab switching
    pub fn show_quick_open(&self) {
        let mtm = MainThreadMarker::from(self);

        // Load templates
        let templates = cterm_app::config::load_sticky_tabs().unwrap_or_default();

        // Collect open tabs with custom names
        let open_tabs = self.collect_open_tabs();

        // Create the overlay if it doesn't exist
        if self.ivars().quick_open.borrow().is_none() {
            let width = self.frame().size.width;
            let overlay = QuickOpenOverlay::new(mtm, width, templates.clone());

            // Set up the callback to open the selected template
            let window_ptr = self as *const Self;
            overlay.set_on_select(move |template| unsafe {
                let window = &*window_ptr;
                window.open_template_tab(&template);
            });

            // Set up callback for switching to an open tab
            overlay.set_on_switch_tab(move |target_ptr| unsafe {
                let target_window = target_ptr as *const NSWindow;
                let _: () = msg_send![target_window, makeKeyAndOrderFront: std::ptr::null::<objc2::runtime::AnyObject>()];
            });

            overlay.set_open_tabs(open_tabs);

            // Add to window content view
            if let Some(content_view) = self.contentView() {
                unsafe {
                    content_view.addSubview(&overlay);
                }

                // Position at top of window
                let content_bounds = content_view.bounds();
                let overlay_frame = NSRect::new(
                    NSPoint::new(0.0, 0.0),
                    NSSize::new(content_bounds.size.width, QUICK_OPEN_HEIGHT),
                );
                unsafe {
                    let _: () = msg_send![&*overlay, setFrame: overlay_frame];
                }
            }

            *self.ivars().quick_open.borrow_mut() = Some(overlay);
        } else {
            // Update templates and open tabs in case they changed
            if let Some(ref overlay) = *self.ivars().quick_open.borrow() {
                overlay.set_templates_and_tabs(templates, open_tabs);
            }
        }

        // Show the overlay
        if let Some(ref overlay) = *self.ivars().quick_open.borrow() {
            overlay.show();
        }
    }

    /// Collect open tabs with custom names for Quick Open
    fn collect_open_tabs(&self) -> Vec<OpenTabEntry> {
        let mut entries = Vec::new();

        // Get all tabbed windows in this window group
        let tabbed_windows: Option<Retained<NSArray<NSWindow>>> =
            unsafe { msg_send![self, tabbedWindows] };

        if let Some(windows) = tabbed_windows {
            for window in windows.iter() {
                // Try to cast to CtermWindow and check for custom title
                let window_ptr = Retained::as_ptr(&window) as *const CtermWindow;
                let cterm_window: &CtermWindow = unsafe { &*window_ptr };

                if let Some(terminal_view) = cterm_window.active_terminal() {
                    if terminal_view.is_title_locked() {
                        let title = window.title().to_string();
                        if !title.is_empty() {
                            entries.push(OpenTabEntry {
                                name: title,
                                window_ptr: Retained::as_ptr(&window) as usize,
                            });
                        }
                    }
                }
            }
        }

        entries
    }

    /// Open a new tab from a template (daemon-backed via ctermd)
    fn open_template_tab(&self, template: &cterm_app::config::StickyTabConfig) {
        // Prepare working directory (clone from git if needed)
        if let Some(ref working_dir) = template.working_directory {
            if let Err(e) =
                cterm_app::prepare_working_directory(working_dir, template.git_remote.as_deref())
            {
                log::error!("Failed to prepare working directory: {}", e);
            }
        }

        let config = &self.ivars().config;
        let opts = cterm_client::CreateSessionOpts {
            cols: 80,
            rows: 24,
            shell: template
                .command
                .clone()
                .or_else(|| config.general.default_shell.clone()),
            args: if template.args.is_empty() && template.command.is_none() {
                config.general.shell_args.clone()
            } else {
                template.args.clone()
            },
            cwd: template
                .working_directory
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            env: template
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            ..Default::default()
        };

        self.spawn_daemon_tab(
            opts,
            Some(template.name.clone()),
            template.color.clone(),
            template.background_color.clone(),
            None,
        );
    }

    /// Get the current tab color
    pub fn tab_color(&self) -> Option<String> {
        self.ivars().pending_tab_color.borrow().clone()
    }

    /// Set the tab color indicator for native macOS tabs
    ///
    /// Creates a small colored circle as the tab's accessory view.
    /// If the tab is not yet available, stores the color for later application.
    pub fn set_tab_color(&self, color: Option<&str>) {
        // Store the color for later if needed
        *self.ivars().pending_tab_color.borrow_mut() = color.map(|s| s.to_string());

        // Persist to daemon
        if let Some(ref tv) = *self.ivars().active_terminal.borrow() {
            tv.set_tab_color_on_daemon(color.unwrap_or(""));
        }

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

    // Window positioning methods

    /// Get the visible frame of the screen (excluding menu bar and dock)
    fn screen_visible_frame(&self) -> NSRect {
        if let Some(screen) = self.screen() {
            screen.visibleFrame()
        } else {
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 600.0))
        }
    }

    /// Fill the screen (like maximize but respects menu bar and dock)
    fn position_fill(&self) {
        let frame = self.screen_visible_frame();
        self.setFrame_display(frame, true);
    }

    /// Center the window on screen
    fn position_center(&self) {
        self.center();
    }

    /// Position window to left half of screen
    fn position_left_half(&self) {
        let screen = self.screen_visible_frame();
        let frame = NSRect::new(
            NSPoint::new(screen.origin.x, screen.origin.y),
            NSSize::new(screen.size.width / 2.0, screen.size.height),
        );
        self.setFrame_display(frame, true);
    }

    /// Position window to right half of screen
    fn position_right_half(&self) {
        let screen = self.screen_visible_frame();
        let frame = NSRect::new(
            NSPoint::new(screen.origin.x + screen.size.width / 2.0, screen.origin.y),
            NSSize::new(screen.size.width / 2.0, screen.size.height),
        );
        self.setFrame_display(frame, true);
    }

    /// Position window to top half of screen
    fn position_top_half(&self) {
        let screen = self.screen_visible_frame();
        let frame = NSRect::new(
            NSPoint::new(screen.origin.x, screen.origin.y + screen.size.height / 2.0),
            NSSize::new(screen.size.width, screen.size.height / 2.0),
        );
        self.setFrame_display(frame, true);
    }

    /// Position window to bottom half of screen
    fn position_bottom_half(&self) {
        let screen = self.screen_visible_frame();
        let frame = NSRect::new(
            NSPoint::new(screen.origin.x, screen.origin.y),
            NSSize::new(screen.size.width, screen.size.height / 2.0),
        );
        self.setFrame_display(frame, true);
    }

    /// Position window to top-left quarter of screen
    fn position_top_left_quarter(&self) {
        let screen = self.screen_visible_frame();
        let frame = NSRect::new(
            NSPoint::new(screen.origin.x, screen.origin.y + screen.size.height / 2.0),
            NSSize::new(screen.size.width / 2.0, screen.size.height / 2.0),
        );
        self.setFrame_display(frame, true);
    }

    /// Position window to top-right quarter of screen
    fn position_top_right_quarter(&self) {
        let screen = self.screen_visible_frame();
        let frame = NSRect::new(
            NSPoint::new(
                screen.origin.x + screen.size.width / 2.0,
                screen.origin.y + screen.size.height / 2.0,
            ),
            NSSize::new(screen.size.width / 2.0, screen.size.height / 2.0),
        );
        self.setFrame_display(frame, true);
    }

    /// Position window to bottom-left quarter of screen
    fn position_bottom_left_quarter(&self) {
        let screen = self.screen_visible_frame();
        let frame = NSRect::new(
            NSPoint::new(screen.origin.x, screen.origin.y),
            NSSize::new(screen.size.width / 2.0, screen.size.height / 2.0),
        );
        self.setFrame_display(frame, true);
    }

    /// Position window to bottom-right quarter of screen
    fn position_bottom_right_quarter(&self) {
        let screen = self.screen_visible_frame();
        let frame = NSRect::new(
            NSPoint::new(screen.origin.x + screen.size.width / 2.0, screen.origin.y),
            NSSize::new(screen.size.width / 2.0, screen.size.height / 2.0),
        );
        self.setFrame_display(frame, true);
    }
}
