//! Main window implementation

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    gdk, gio, glib, Application, ApplicationWindow, Box as GtkBox, EventControllerKey, Notebook,
    Orientation, PopoverMenuBar,
};

use cterm_app::config::{Config, ConnectionType, RemoteConfig};
use cterm_app::file_transfer::PendingFileManager;
use cterm_app::shortcuts::ShortcutManager;
use cterm_ui::events::{Action, KeyCode, Modifiers};
use cterm_ui::theme::Theme;

use crate::dialogs;
use crate::docker_dialog::{self, DockerSelection};
use crate::menu;
use crate::notification_bar::NotificationBar;
use crate::quick_open::QuickOpenOverlay;
use crate::tab_bar::TabBar;
use crate::terminal_widget::{CellDimensions, TerminalWidget};

/// Tab entry tracking terminal and its ID
struct TabEntry {
    id: u64,
    title: String,
    terminal: TerminalWidget,
    /// Whether title was explicitly set (locks out OSC updates)
    title_locked: bool,
    /// Tab color override
    color: Option<String>,
    /// Daemon session ID (for upgrade state preservation)
    session_id: Option<String>,
    /// Daemon socket path (None = local daemon, Some = remote/SSH-tunneled)
    daemon_socket: Option<std::path::PathBuf>,
}

/// Main window container
pub struct CtermWindow {
    pub window: ApplicationWindow,
    pub notebook: Notebook,
    pub tab_bar: TabBar,
    pub config: Rc<RefCell<Config>>,
    pub theme: Theme,
    pub shortcuts: ShortcutManager,
    tabs: Rc<RefCell<Vec<TabEntry>>>,
    next_tab_id: Rc<RefCell<u64>>,
    menu_bar: PopoverMenuBar,
    has_bell: Rc<RefCell<bool>>,
    notification_bar: NotificationBar,
    file_manager: Rc<RefCell<PendingFileManager>>,
    quick_open: QuickOpenOverlay,
    remote_manager: cterm_client::RemoteManager,
    unixshells_service: std::sync::Arc<cterm_app::unixshells::DeviceService>,
    unixshells_menu: gio::Menu,
}

/// Show an error dialog when a seamless upgrade fails.
fn show_upgrade_error_dialog(window: &ApplicationWindow, error: &dyn std::fmt::Display) {
    let dialog = gtk4::MessageDialog::new(
        Some(window),
        gtk4::DialogFlags::MODAL,
        gtk4::MessageType::Error,
        gtk4::ButtonsType::Ok,
        format!("Upgrade failed: {}", error),
    );
    dialog.connect_response(|d, _| d.close());
    dialog.present();
}

impl CtermWindow {
    /// Create a new window
    pub fn new(app: &Application, config: &Config, theme: &Theme) -> Self {
        // Calculate cell dimensions for initial window sizing
        let cell_dims = calculate_initial_cell_dimensions(config);

        // Calculate window size for 80x24 terminal plus chrome (menu bar ~30px, tab bar ~24px)
        let chrome_height = 54; // Approximate height for menu bar + tab bar
        let default_width = (cell_dims.width * 80.0).ceil() as i32 + 20; // Add some padding
        let default_height = (cell_dims.height * 24.0).ceil() as i32 + chrome_height + 20;

        // Create the main window
        let window = ApplicationWindow::builder()
            .application(app)
            .title("cterm")
            .default_width(default_width)
            .default_height(default_height)
            .build();

        // Create the main container
        let main_box = GtkBox::new(Orientation::Vertical, 0);

        // Create menu bar
        let menu_result = menu::create_menu_model_with_options(config.general.show_debug_menu);
        let unixshells_menu = menu_result.unixshells_menu.clone();
        let menu_bar = PopoverMenuBar::from_model(Some(&menu_result.menu));
        main_box.append(&menu_bar);

        // Create tab bar
        let tab_bar = TabBar::new();
        main_box.append(tab_bar.widget());

        // Create notification bar for file transfers (initially hidden)
        let notification_bar = NotificationBar::new();
        main_box.append(notification_bar.widget());

        // Create Quick Open overlay (initially hidden)
        let quick_open = QuickOpenOverlay::new();
        main_box.append(quick_open.widget());

        // Create notebook for terminal tabs (hidden tabs, we use custom tab bar)
        let notebook = Notebook::builder()
            .show_tabs(false)
            .show_border(false)
            .vexpand(true)
            .hexpand(true)
            .build();

        main_box.append(&notebook);

        window.set_child(Some(&main_box));

        // Create shortcut manager
        let shortcuts = ShortcutManager::from_config(&config.shortcuts);

        let has_bell = Rc::new(RefCell::new(false));
        let file_manager = Rc::new(RefCell::new(PendingFileManager::new()));

        let cterm_window = Self {
            window: window.clone(),
            notebook: notebook.clone(),
            tab_bar,
            config: Rc::new(RefCell::new(config.clone())),
            theme: theme.clone(),
            shortcuts,
            tabs: Rc::new(RefCell::new(Vec::new())),
            next_tab_id: Rc::new(RefCell::new(0)),
            menu_bar,
            has_bell,
            notification_bar,
            file_manager,
            quick_open,
            remote_manager: cterm_client::RemoteManager::new(),
            unixshells_service: std::sync::Arc::new(cterm_app::unixshells::DeviceService::new(
                cterm_app::config::config_dir().unwrap_or_default(),
                config.latch.relay_username.clone(),
            )),
            unixshells_menu: unixshells_menu.clone(),
        };

        // Set up window actions
        cterm_window.setup_actions();
        cterm_window.setup_unixshells_menu_poller();

        // Set up Quick Open callback
        cterm_window.setup_quick_open();

        // Set up key event handling
        cterm_window.setup_key_handler();

        // Set up window focus handler to clear bell on focus
        cterm_window.setup_focus_handler();

        // Set up terminal focus restoration after menu interactions
        cterm_window.setup_terminal_focus_restore();

        // Set up notification bar callbacks for file transfers
        cterm_window.setup_notification_bar();

        // Create initial tab
        cterm_window.new_tab();

        // Initially hide tab bar (only one tab)
        cterm_window.tab_bar.update_visibility();

        // Set up tab bar callbacks
        cterm_window.setup_tab_bar_callbacks();

        // Update window title when switching tabs
        cterm_window.setup_tab_switch_handler();

        // Set up close request handler for process confirmation
        cterm_window.setup_close_request_handler();

        cterm_window
    }

    /// Create a new window without an initial tab.
    ///
    /// Used for daemon reconnection where tabs will be added from existing sessions.
    /// The caller must add at least one tab before presenting the window.
    pub fn new_empty(app: &Application, config: &Config, theme: &Theme) -> Self {
        // Calculate cell dimensions for initial window sizing
        let cell_dims = calculate_initial_cell_dimensions(config);

        // Calculate window size for 80x24 terminal plus chrome (menu bar ~30px, tab bar ~24px)
        let chrome_height = 54;
        let default_width = (cell_dims.width * 80.0).ceil() as i32 + 20;
        let default_height = (cell_dims.height * 24.0).ceil() as i32 + chrome_height + 20;

        let window = ApplicationWindow::builder()
            .application(app)
            .title("cterm")
            .default_width(default_width)
            .default_height(default_height)
            .build();

        let main_box = GtkBox::new(Orientation::Vertical, 0);

        let menu_result = menu::create_menu_model_with_options(config.general.show_debug_menu);
        let unixshells_menu = menu_result.unixshells_menu.clone();
        let menu_bar = PopoverMenuBar::from_model(Some(&menu_result.menu));
        main_box.append(&menu_bar);

        let tab_bar = TabBar::new();
        main_box.append(tab_bar.widget());

        let notification_bar = NotificationBar::new();
        main_box.append(notification_bar.widget());

        let quick_open = QuickOpenOverlay::new();
        main_box.append(quick_open.widget());

        let notebook = Notebook::builder()
            .show_tabs(false)
            .show_border(false)
            .vexpand(true)
            .hexpand(true)
            .build();

        main_box.append(&notebook);
        window.set_child(Some(&main_box));

        let shortcuts = ShortcutManager::from_config(&config.shortcuts);
        let has_bell = Rc::new(RefCell::new(false));
        let file_manager = Rc::new(RefCell::new(PendingFileManager::new()));

        let cterm_window = Self {
            window: window.clone(),
            notebook: notebook.clone(),
            tab_bar,
            config: Rc::new(RefCell::new(config.clone())),
            theme: theme.clone(),
            shortcuts,
            tabs: Rc::new(RefCell::new(Vec::new())),
            next_tab_id: Rc::new(RefCell::new(0)),
            menu_bar,
            has_bell,
            notification_bar,
            file_manager,
            quick_open,
            remote_manager: cterm_client::RemoteManager::new(),
            unixshells_service: std::sync::Arc::new(cterm_app::unixshells::DeviceService::new(
                cterm_app::config::config_dir().unwrap_or_default(),
                config.latch.relay_username.clone(),
            )),
            unixshells_menu: unixshells_menu.clone(),
        };

        cterm_window.setup_actions();
        cterm_window.setup_unixshells_menu_poller();
        cterm_window.setup_quick_open();
        cterm_window.setup_key_handler();
        cterm_window.setup_focus_handler();
        cterm_window.setup_terminal_focus_restore();
        cterm_window.setup_notification_bar();

        // No initial tab — caller will add tabs

        cterm_window.tab_bar.update_visibility();
        cterm_window.setup_tab_bar_callbacks();
        cterm_window.setup_tab_switch_handler();
        cterm_window.setup_close_request_handler();

        cterm_window
    }

    /// Set up window actions for the menu
    fn setup_actions(&self) {
        let window = &self.window;
        let notebook = self.notebook.clone();
        let tabs = Rc::clone(&self.tabs);
        let next_tab_id = Rc::clone(&self.next_tab_id);
        let config = Rc::clone(&self.config);
        let theme = self.theme.clone();
        let tab_bar = self.tab_bar.clone();
        let has_bell = Rc::clone(&self.has_bell);
        let menu_bar = self.menu_bar.clone();

        // File menu actions
        {
            let notebook = notebook.clone();
            let tabs = Rc::clone(&tabs);
            let next_tab_id = Rc::clone(&next_tab_id);
            let config = Rc::clone(&config);
            let theme = theme.clone();
            let tab_bar = tab_bar.clone();
            let window_clone = window.clone();
            let has_bell = Rc::clone(&has_bell);
            let file_manager = Rc::clone(&self.file_manager);
            let notification_bar = self.notification_bar.clone();
            let action = gio::SimpleAction::new("new-tab", None);
            action.connect_activate(move |_, _| {
                // Get info from the active terminal
                let (cwd, daemon_socket) = {
                    let tabs_borrow = tabs.borrow();
                    if let Some(page_idx) = notebook.current_page() {
                        let entry = tabs_borrow.get(page_idx as usize);
                        #[cfg(unix)]
                        let cwd = entry.and_then(|e| e.terminal.foreground_cwd());
                        #[cfg(not(unix))]
                        let cwd: Option<String> = None;
                        let socket = entry.and_then(|e| e.daemon_socket.clone());
                        (cwd, socket)
                    } else {
                        (None, None)
                    }
                };

                create_new_tab(
                    &notebook,
                    &tabs,
                    &next_tab_id,
                    &config,
                    &theme,
                    &tab_bar,
                    &window_clone,
                    &has_bell,
                    &file_manager,
                    &notification_bar,
                    cwd,
                    daemon_socket,
                );
            });
            window.add_action(&action);
        }

        {
            let app = window.application().unwrap();
            let config = Rc::clone(&config);
            let theme = theme.clone();
            let action = gio::SimpleAction::new("new-window", None);
            action.connect_activate(move |_, _| {
                let cfg = config.borrow();
                if let Some(gtk_app) = app.downcast_ref::<Application>() {
                    let new_win = CtermWindow::new(gtk_app, &cfg, &theme);
                    new_win.present();
                }
            });
            window.add_action(&action);
        }

        {
            let notebook = notebook.clone();
            let tabs = Rc::clone(&tabs);
            let tab_bar = tab_bar.clone();
            let window_clone = window.clone();
            let config = Rc::clone(&config);
            let action = gio::SimpleAction::new("close-tab", None);
            action.connect_activate(move |_, _| {
                close_current_tab(&notebook, &tabs, &tab_bar, &window_clone, &config);
            });
            window.add_action(&action);
        }

        {
            let notebook = notebook.clone();
            let tabs = Rc::clone(&tabs);
            let tab_bar = tab_bar.clone();
            let window_clone = window.clone();
            let action = gio::SimpleAction::new("close-other-tabs", None);
            action.connect_activate(move |_, _| {
                close_other_tabs(&notebook, &tabs, &tab_bar, &window_clone);
            });
            window.add_action(&action);
        }

        {
            let window_clone = window.clone();
            let action = gio::SimpleAction::new("quit", None);
            action.connect_activate(move |_, _| {
                window_clone.close();
            });
            window.add_action(&action);
        }

        // Quick Open Template action
        {
            let quick_open = self.quick_open.clone();
            let action = gio::SimpleAction::new("quick-open", None);
            action.connect_activate(move |_, _| {
                // Load templates and show overlay
                let templates = cterm_app::config::load_sticky_tabs().unwrap_or_default();
                quick_open.set_templates(templates);
                quick_open.show();
            });
            window.add_action(&action);
        }

        // Docker picker action
        {
            let notebook = notebook.clone();
            let tabs = Rc::clone(&tabs);
            let next_tab_id = Rc::clone(&next_tab_id);
            let config = Rc::clone(&config);
            let theme = theme.clone();
            let tab_bar = tab_bar.clone();
            let window_clone = window.clone();
            let has_bell = Rc::clone(&has_bell);
            let file_manager = Rc::clone(&self.file_manager);
            let notification_bar = self.notification_bar.clone();
            let action = gio::SimpleAction::new("docker-picker", None);
            action.connect_activate(move |_, _| {
                let notebook = notebook.clone();
                let tabs = Rc::clone(&tabs);
                let next_tab_id = Rc::clone(&next_tab_id);
                let config = Rc::clone(&config);
                let theme = theme.clone();
                let tab_bar = tab_bar.clone();
                let window_inner = window_clone.clone();
                let has_bell = Rc::clone(&has_bell);
                let file_manager = Rc::clone(&file_manager);
                let notification_bar = notification_bar.clone();

                docker_dialog::show_docker_picker(&window_clone, move |selection| {
                    let (command, args, title) = match &selection {
                        DockerSelection::ExecContainer(c) => {
                            let (cmd, args) = cterm_app::docker::build_exec_command(&c.name, None);
                            (cmd, args, format!("Docker: {}", c.name))
                        }
                        DockerSelection::RunImage(i) => {
                            let (cmd, args) = cterm_app::docker::build_run_command(
                                &format!("{}:{}", i.repository, i.tag),
                                None,
                                true,
                                &[],
                            );
                            (cmd, args, format!("Docker: {}:{}", i.repository, i.tag))
                        }
                    };

                    create_docker_tab(
                        &notebook,
                        &tabs,
                        &next_tab_id,
                        &config,
                        &theme,
                        &tab_bar,
                        &window_inner,
                        &has_bell,
                        &file_manager,
                        &notification_bar,
                        &command,
                        &args,
                        &title,
                    );
                });
            });
            window.add_action(&action);
        }

        // Session actions (daemon attach)
        {
            let notebook = notebook.clone();
            let tabs = Rc::clone(&tabs);
            let next_tab_id = Rc::clone(&next_tab_id);
            let config = Rc::clone(&config);
            let theme = theme.clone();
            let tab_bar = tab_bar.clone();
            let window_clone = window.clone();
            let has_bell = Rc::clone(&has_bell);
            let file_manager = Rc::clone(&self.file_manager);
            let notification_bar = self.notification_bar.clone();
            let action = gio::SimpleAction::new("attach-session", None);
            action.connect_activate(move |_, _| {
                let notebook = notebook.clone();
                let tabs = Rc::clone(&tabs);
                let next_tab_id = Rc::clone(&next_tab_id);
                let config = Rc::clone(&config);
                let theme = theme.clone();
                let tab_bar = tab_bar.clone();
                let window_inner = window_clone.clone();
                let has_bell = Rc::clone(&has_bell);
                let file_manager = Rc::clone(&file_manager);
                let notification_bar = notification_bar.clone();

                crate::session_dialog::show_session_picker(&window_clone, move |session_id| {
                    create_daemon_tab(
                        &notebook,
                        &tabs,
                        &next_tab_id,
                        &config,
                        &theme,
                        &tab_bar,
                        &window_inner,
                        &has_bell,
                        &file_manager,
                        &notification_bar,
                        &session_id,
                    );
                });
            });
            window.add_action(&action);
        }

        // SSH connect action
        {
            let notebook = notebook.clone();
            let tabs = Rc::clone(&tabs);
            let next_tab_id = Rc::clone(&next_tab_id);
            let config = Rc::clone(&config);
            let theme = theme.clone();
            let tab_bar = tab_bar.clone();
            let window_clone = window.clone();
            let has_bell = Rc::clone(&has_bell);
            let file_manager = Rc::clone(&self.file_manager);
            let notification_bar = self.notification_bar.clone();
            let action = gio::SimpleAction::new("ssh-connect", None);
            action.connect_activate(move |_, _| {
                let notebook = notebook.clone();
                let tabs = Rc::clone(&tabs);
                let next_tab_id = Rc::clone(&next_tab_id);
                let config = Rc::clone(&config);
                let theme = theme.clone();
                let tab_bar = tab_bar.clone();
                let window_inner = window_clone.clone();
                let has_bell = Rc::clone(&has_bell);
                let file_manager = Rc::clone(&file_manager);
                let notification_bar = notification_bar.clone();

                crate::session_dialog::show_ssh_dialog(&window_clone, move |sessions| {
                    for recon in sessions {
                        let sid = recon.handle.session_id().to_string();
                        let daemon_socket = recon.handle.socket_path().map(|p| p.to_owned());
                        let (title, title_locked) = if !recon.custom_title.is_empty() {
                            (recon.custom_title.clone(), true)
                        } else if !recon.title.is_empty() {
                            (recon.title.clone(), false)
                        } else {
                            ("SSH".to_string(), false)
                        };

                        let tab_color = if recon.tab_color.is_empty() {
                            None
                        } else {
                            Some(recon.tab_color.clone())
                        };

                        let cfg = config.borrow();
                        let terminal = TerminalWidget::from_daemon_with_screen(recon, &cfg, &theme);
                        drop(cfg);

                        let tab_id = generate_tab_id(&next_tab_id);
                        let page_num =
                            notebook.append_page(terminal.widget(), None::<&gtk4::Widget>);
                        tab_bar.add_tab(tab_id, &title);

                        setup_tab_callbacks(
                            &notebook,
                            &tabs,
                            &config,
                            &tab_bar,
                            &window_inner,
                            &has_bell,
                            &file_manager,
                            &notification_bar,
                            &terminal,
                            tab_id,
                            false,
                        );

                        finalize_new_tab(
                            &notebook,
                            &tabs,
                            &tab_bar,
                            tab_id,
                            page_num,
                            title,
                            terminal,
                            title_locked,
                            Some(sid),
                            daemon_socket,
                        );

                        if let Some(ref color) = tab_color {
                            tab_bar.set_color(tab_id, Some(color));
                            if let Some(tab) = tabs.borrow_mut().iter_mut().find(|t| t.id == tab_id)
                            {
                                tab.color = tab_color;
                            }
                        }
                    }
                });
            });
            window.add_action(&action);
        }

        // Manage Remotes
        {
            let window_clone = window.clone();
            let action = gio::SimpleAction::new("manage-remotes", None);
            action.connect_activate(move |_, _| {
                crate::remotes_dialog::show_remotes_dialog(&window_clone, || {
                    log::info!("Remotes configuration saved");
                });
            });
            window.add_action(&action);
        }

        // Unix Shells actions
        {
            // Sign In
            let window_clone = window.clone();
            let ds = std::sync::Arc::clone(&self.unixshells_service);
            let action = gio::SimpleAction::new("unixshells-signin", None);
            action.connect_activate(move |_, _| {
                crate::unixshells_dialog::show_signin_dialog(&window_clone, &ds);
            });
            window.add_action(&action);

            // Sign Out
            let ds = std::sync::Arc::clone(&self.unixshells_service);
            let action = gio::SimpleAction::new("unixshells-signout", None);
            action.connect_activate(move |_, _| {
                ds.sign_out();
            });
            window.add_action(&action);

            // Connect to device (parameterized action)
            let notebook_us = notebook.clone();
            let tabs_us = Rc::clone(&tabs);
            let next_tab_id_us = Rc::clone(&self.next_tab_id);
            let config_us = Rc::clone(&self.config);
            let theme_us = self.theme.clone();
            let tab_bar_us = self.tab_bar.clone();
            let window_us = window.clone();
            let has_bell_us = Rc::clone(&self.has_bell);
            let file_manager_us = Rc::clone(&self.file_manager);
            let notification_bar_us = self.notification_bar.clone();
            let ds = std::sync::Arc::clone(&self.unixshells_service);
            let action = gio::SimpleAction::new(
                "unixshells-connect",
                Some(glib::VariantTy::new("s").unwrap()),
            );
            action.connect_activate(move |_, param| {
                let Some(device_name) = param.and_then(|v| v.get::<String>()) else {
                    return;
                };

                // Get username from service
                let username = match ds.login_state() {
                    cterm_app::unixshells::LoginState::LoggedIn { username } => username,
                    _ => return,
                };

                // Find session name from device list
                let session_name = {
                    let devices = ds.devices();
                    devices
                        .iter()
                        .find(|d| d.device == device_name)
                        .and_then(|d| d.sessions.first())
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| "default".to_string())
                };

                // Build RemoteConfig for mosh relay connection
                let remote = cterm_app::config::RemoteConfig {
                    name: format!("unixshells-{}", device_name),
                    host: "jump@relay.unixshells.com".to_string(),
                    method: cterm_app::config::ConnectionMethod::Mosh,
                    connection_type: cterm_app::config::ConnectionType::Relay,
                    proxy_jump: None,
                    relay_username: Some(username.clone()),
                    relay_device: Some(device_name.clone()),
                    session_name: Some(session_name),
                    ssh_compression: true,
                };

                let mosh_config = mosh_config_from_remote(&remote, 80, 24);

                spawn_mosh_tab(
                    &notebook_us,
                    &tabs_us,
                    &next_tab_id_us,
                    &config_us,
                    &theme_us,
                    &tab_bar_us,
                    &window_us,
                    &has_bell_us,
                    &file_manager_us,
                    &notification_bar_us,
                    mosh_config,
                    device_name,
                    None,
                    None,
                    false,
                );
            });
            window.add_action(&action);
        }

        // URL actions (for hyperlink context menu)
        {
            let action =
                gio::SimpleAction::new("open-url", Some(glib::VariantTy::new("s").unwrap()));
            action.connect_activate(|_, param| {
                if let Some(url) = param.and_then(|v| v.get::<String>()) {
                    if let Err(e) = open::that(&url) {
                        log::error!("Failed to open URL: {}", e);
                    }
                }
            });
            window.add_action(&action);

            let action =
                gio::SimpleAction::new("copy-url", Some(glib::VariantTy::new("s").unwrap()));
            action.connect_activate(|_, param| {
                if let Some(url) = param.and_then(|v| v.get::<String>()) {
                    if let Some(display) = gdk::Display::default() {
                        let clipboard = display.clipboard();
                        clipboard.set_text(&url);
                    }
                }
            });
            window.add_action(&action);
        }

        // Edit menu actions
        {
            // Copy selection to clipboard
            let notebook_copy = notebook.clone();
            let tabs_copy = Rc::clone(&tabs);
            let action = gio::SimpleAction::new("copy", None);
            action.connect_activate(move |_, _| {
                if let Some(page_idx) = notebook_copy.current_page() {
                    let tabs = tabs_copy.borrow();
                    if let Some(tab) = tabs.get(page_idx as usize) {
                        tab.terminal.copy_selection();
                    }
                }
            });
            window.add_action(&action);
        }

        {
            // Copy as HTML
            let notebook_copy_html = notebook.clone();
            let tabs_copy_html = Rc::clone(&tabs);
            let action = gio::SimpleAction::new("copy-html", None);
            action.connect_activate(move |_, _| {
                if let Some(page_idx) = notebook_copy_html.current_page() {
                    let tabs = tabs_copy_html.borrow();
                    if let Some(tab) = tabs.get(page_idx as usize) {
                        tab.terminal.copy_selection_html();
                    }
                }
            });
            window.add_action(&action);
        }

        {
            let notebook = notebook.clone();
            let tabs = Rc::clone(&tabs);
            let action = gio::SimpleAction::new("paste", None);
            action.connect_activate(move |_, _| {
                if let Some(display) = gdk::Display::default() {
                    let clipboard = display.clipboard();
                    let tabs_paste = Rc::clone(&tabs);
                    let notebook_paste = notebook.clone();
                    clipboard.read_text_async(None::<&gio::Cancellable>, move |result| {
                        if let Ok(Some(text)) = result {
                            if let Some(page_idx) = notebook_paste.current_page() {
                                let tabs = tabs_paste.borrow();
                                if let Some(tab) = tabs.get(page_idx as usize) {
                                    tab.terminal.write_str(&text);
                                }
                            }
                        }
                    });
                }
            });
            window.add_action(&action);
        }

        {
            // Select All
            let notebook_select = notebook.clone();
            let tabs_select = Rc::clone(&tabs);
            let action = gio::SimpleAction::new("select-all", None);
            action.connect_activate(move |_, _| {
                if let Some(page_idx) = notebook_select.current_page() {
                    let tabs = tabs_select.borrow();
                    if let Some(tab) = tabs.get(page_idx as usize) {
                        tab.terminal.select_all();
                    }
                }
            });
            window.add_action(&action);
        }

        // Terminal menu actions
        {
            let window_clone = window.clone();
            let tabs = Rc::clone(&tabs);
            let notebook = notebook.clone();
            let tab_bar = tab_bar.clone();
            let action = gio::SimpleAction::new("set-title", None);
            action.connect_activate(move |_, _| {
                // Find the tab_id for the current page
                let tab_id = notebook
                    .current_page()
                    .and_then(|idx| tabs.borrow().get(idx as usize).map(|t| t.id));
                if let Some(tab_id) = tab_id {
                    show_rename_tab_dialog(&window_clone, &tabs, &tab_bar, tab_id);
                }
            });
            window.add_action(&action);
        }

        {
            let window_clone = window.clone();
            let tabs = Rc::clone(&tabs);
            let notebook = notebook.clone();
            let tab_bar = tab_bar.clone();
            let action = gio::SimpleAction::new("set-color", None);
            action.connect_activate(move |_, _| {
                let tabs_clone = Rc::clone(&tabs);
                let notebook_clone = notebook.clone();
                let tab_bar_clone = tab_bar.clone();
                dialogs::show_set_color_dialog(&window_clone, move |color| {
                    if let Some(page_idx) = notebook_clone.current_page() {
                        let mut tabs = tabs_clone.borrow_mut();
                        if let Some(tab) = tabs.get_mut(page_idx as usize) {
                            tab_bar_clone.set_color(tab.id, color.as_deref());
                            tab.terminal
                                .set_tab_color_on_daemon(color.as_deref().unwrap_or(""));
                            tab.color = color;
                        }
                    }
                });
            });
            window.add_action(&action);
        }

        {
            let window_clone = window.clone();
            let tabs = Rc::clone(&tabs);
            let notebook = notebook.clone();
            let action = gio::SimpleAction::new("find", None);
            action.connect_activate(move |_, _| {
                let tabs = Rc::clone(&tabs);
                let notebook = notebook.clone();
                dialogs::show_find_dialog(&window_clone, move |text, case_sensitive, regex| {
                    log::info!("Find: '{}' case={} regex={}", text, case_sensitive, regex);
                    if let Some(page_idx) = notebook.current_page() {
                        let tabs = tabs.borrow();
                        if let Some(tab) = tabs.get(page_idx as usize) {
                            let count = tab.terminal.find(&text, case_sensitive, regex);
                            log::info!("Found {} matches", count);
                        }
                    }
                });
            });
            window.add_action(&action);
        }

        {
            let action =
                gio::SimpleAction::new("set-encoding", Some(&glib::VariantType::new("s").unwrap()));
            action.connect_activate(|_, param| {
                if let Some(encoding) = param.and_then(|p| p.get::<String>()) {
                    if encoding == "utf8" {
                        log::info!("Encoding set to UTF-8");
                    } else {
                        // Terminal currently only supports UTF-8
                        log::warn!(
                            "Encoding '{}' requested but only UTF-8 is currently supported",
                            encoding
                        );
                    }
                }
            });
            window.add_action(&action);
        }

        {
            let tabs = Rc::clone(&tabs);
            let notebook = notebook.clone();
            let action =
                gio::SimpleAction::new("send-signal", Some(&glib::VariantType::new("s").unwrap()));
            action.connect_activate(move |_, param| {
                if let Some(signal_str) = param.and_then(|p| p.get::<String>()) {
                    if let Ok(signal) = signal_str.parse::<i32>() {
                        if let Some(page_idx) = notebook.current_page() {
                            let tabs = tabs.borrow();
                            if let Some(tab) = tabs.get(page_idx as usize) {
                                log::info!("Sending signal {} to terminal", signal);
                                tab.terminal.send_signal(signal);
                            }
                        }
                    }
                }
            });
            window.add_action(&action);
        }

        {
            let tabs = Rc::clone(&tabs);
            let notebook = notebook.clone();
            let action = gio::SimpleAction::new("reset", None);
            action.connect_activate(move |_, _| {
                if let Some(page_idx) = notebook.current_page() {
                    let tabs = tabs.borrow();
                    if let Some(tab) = tabs.get(page_idx as usize) {
                        tab.terminal.reset();
                    }
                }
            });
            window.add_action(&action);
        }

        {
            let tabs = Rc::clone(&tabs);
            let notebook = notebook.clone();
            let action = gio::SimpleAction::new("clear-reset", None);
            action.connect_activate(move |_, _| {
                if let Some(page_idx) = notebook.current_page() {
                    let tabs = tabs.borrow();
                    if let Some(tab) = tabs.get(page_idx as usize) {
                        tab.terminal.clear_scrollback_and_reset();
                    }
                }
            });
            window.add_action(&action);
        }

        // Tabs menu actions
        {
            let notebook = notebook.clone();
            let tabs = Rc::clone(&tabs);
            let tab_bar = tab_bar.clone();
            let action = gio::SimpleAction::new("prev-tab", None);
            action.connect_activate(move |_, _| {
                let n = notebook.n_pages();
                if n > 0 {
                    let current = notebook.current_page().unwrap_or(0);
                    let prev = if current == 0 { n - 1 } else { current - 1 };
                    notebook.set_current_page(Some(prev));
                    sync_tab_bar_active(&tab_bar, &tabs, &notebook);
                }
            });
            window.add_action(&action);
        }

        {
            let notebook = notebook.clone();
            let tabs = Rc::clone(&tabs);
            let tab_bar = tab_bar.clone();
            let action = gio::SimpleAction::new("next-tab", None);
            action.connect_activate(move |_, _| {
                let n = notebook.n_pages();
                if n > 0 {
                    let current = notebook.current_page().unwrap_or(0);
                    notebook.set_current_page(Some((current + 1) % n));
                    sync_tab_bar_active(&tab_bar, &tabs, &notebook);
                }
            });
            window.add_action(&action);
        }

        {
            let notebook = notebook.clone();
            let tabs = Rc::clone(&tabs);
            let tab_bar = tab_bar.clone();
            let action = gio::SimpleAction::new("next-alerted-tab", None);
            action.connect_activate(move |_, _| {
                let n = notebook.n_pages();
                if n > 0 {
                    let current = notebook.current_page().unwrap_or(0) as usize;
                    let tabs_ref = tabs.borrow();
                    for offset in 1..tabs_ref.len() {
                        let idx = (current + offset) % tabs_ref.len();
                        if let Some(entry) = tabs_ref.get(idx) {
                            if tab_bar.has_bell(entry.id) {
                                drop(tabs_ref);
                                notebook.set_current_page(Some(idx as u32));
                                sync_tab_bar_active(&tab_bar, &tabs, &notebook);
                                return;
                            }
                        }
                    }
                }
            });
            window.add_action(&action);
        }

        {
            let notebook = notebook.clone();
            let tabs = Rc::clone(&tabs);
            let tab_bar = tab_bar.clone();
            let action =
                gio::SimpleAction::new("switch-tab", Some(&glib::VariantType::new("s").unwrap()));
            action.connect_activate(move |_, param| {
                if let Some(id_str) = param.and_then(|p| p.get::<String>()) {
                    if let Ok(id) = id_str.parse::<u64>() {
                        let tabs_ref = tabs.borrow();
                        if let Some(idx) = tabs_ref.iter().position(|t| t.id == id) {
                            notebook.set_current_page(Some(idx as u32));
                            drop(tabs_ref);
                            sync_tab_bar_active(&tab_bar, &tabs, &notebook);
                        }
                    }
                }
            });
            window.add_action(&action);
        }

        // Tools menu actions
        {
            let notebook = notebook.clone();
            let tabs = Rc::clone(&tabs);
            let window_clone = window.clone();
            let action = gio::SimpleAction::new(
                "run-tool-shortcut",
                Some(&glib::VariantType::new("s").unwrap()),
            );
            action.connect_activate(move |_, param| {
                if let Some(idx_str) = param.and_then(|p| p.get::<String>()) {
                    if let Ok(idx) = idx_str.parse::<usize>() {
                        if let Ok(shortcuts) = cterm_app::config::load_tool_shortcuts() {
                            if let Some(shortcut) = shortcuts.get(idx) {
                                // Get CWD from active terminal
                                #[cfg(unix)]
                                let cwd = {
                                    let tabs_borrow = tabs.borrow();
                                    if let Some(page_idx) = notebook.current_page() {
                                        tabs_borrow
                                            .get(page_idx as usize)
                                            .and_then(|entry| entry.terminal.foreground_cwd())
                                    } else {
                                        None
                                    }
                                };
                                #[cfg(not(unix))]
                                let cwd: Option<String> = None;

                                let cwd = cwd.unwrap_or_else(|| {
                                    std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
                                });

                                if let Err(e) = shortcut.execute(std::path::Path::new(&cwd)) {
                                    let dialog = gtk4::MessageDialog::new(
                                        Some(&window_clone),
                                        gtk4::DialogFlags::MODAL,
                                        gtk4::MessageType::Error,
                                        gtk4::ButtonsType::Ok,
                                        format!(
                                            "Failed to launch \"{}\"\n\nCommand '{}' failed: {}",
                                            shortcut.name, shortcut.command, e
                                        ),
                                    );
                                    dialog.connect_response(|d, _| d.close());
                                    dialog.present();
                                }
                            }
                        }
                    }
                }
            });
            window.add_action(&action);
        }

        // Help menu actions
        {
            let window_clone = window.clone();
            let config = Rc::clone(&config);
            let menu_bar_clone = menu_bar.clone();
            let action = gio::SimpleAction::new("preferences", None);
            action.connect_activate(move |_, _| {
                let cfg = config.borrow().clone();
                let config_for_save = Rc::clone(&config);
                let menu_bar = menu_bar_clone.clone();
                dialogs::show_preferences_dialog(&window_clone, &cfg, move |new_config| {
                    log::info!("Preferences saved");
                    // Save to disk
                    if let Err(e) = cterm_app::config::save_config(&new_config) {
                        log::error!("Failed to save config: {}", e);
                    } else {
                        log::info!("Configuration saved to disk");
                    }
                    // Rebuild menu bar to reflect debug menu preference
                    menu::rebuild_menu_bar(&menu_bar, new_config.general.show_debug_menu);
                    // Update internal config state
                    *config_for_save.borrow_mut() = new_config;
                });
            });
            window.add_action(&action);
        }

        // Check for updates action
        {
            let window_clone = window.clone();
            let action = gio::SimpleAction::new("check-updates", None);
            action.connect_activate(move |_, _| {
                crate::update_dialog::show_update_dialog(&window_clone);
            });
            window.add_action(&action);
        }

        // Execute upgrade action (called from update dialog)
        {
            let tabs = Rc::clone(&tabs);
            let window_clone = window.clone();
            let notebook_upgrade = notebook.clone();
            let action = gio::SimpleAction::new(
                "execute-upgrade",
                Some(&glib::VariantType::new("s").unwrap()),
            );
            action.connect_activate(move |_, param| {
                if let Some(binary_path) = param.and_then(|p| p.get::<String>()) {
                    log::info!("Executing seamless upgrade with binary: {}", binary_path);

                    // Collect upgrade state from current window
                    let tabs_borrowed = tabs.borrow();

                    // Build upgrade state
                    let mut upgrade_state = cterm_app::upgrade::UpgradeState::new();

                    // Collect window state
                    let mut window_state = cterm_app::upgrade::WindowUpgradeState::new();
                    window_state.width = window_clone.width();
                    window_state.height = window_clone.height();
                    window_state.maximized = window_clone.is_maximized();
                    window_state.fullscreen = window_clone.is_fullscreen();

                    for tab in tabs_borrowed.iter() {
                        let mut tab_state = cterm_app::upgrade::TabUpgradeState::new(tab.id);
                        tab_state.title = tab.title.clone();
                        if tab.title_locked {
                            tab_state.custom_title = Some(tab.title.clone());
                        }
                        tab_state.color = tab.color.clone();
                        tab_state.session_id = tab.session_id.clone();

                        // Get working directory from terminal
                        #[cfg(unix)]
                        {
                            let term = tab.terminal.terminal().lock();
                            tab_state.cwd = term
                                .foreground_cwd()
                                .map(|p| p.to_string_lossy().into_owned());
                        }

                        window_state.tabs.push(tab_state);
                    }

                    // Set active tab
                    window_state.active_tab = notebook_upgrade.current_page().unwrap_or(0) as usize;

                    upgrade_state.windows.push(window_state);

                    drop(tabs_borrowed);

                    log::info!(
                        "Collected upgrade state: {} windows, {} tabs",
                        upgrade_state.windows.len(),
                        upgrade_state
                            .windows
                            .iter()
                            .map(|w| w.tabs.len())
                            .sum::<usize>()
                    );

                    // Execute the upgrade
                    let binary = std::path::Path::new(&binary_path);
                    match cterm_app::upgrade::execute_upgrade(binary, &upgrade_state) {
                        Ok(()) => {
                            log::info!("Upgrade successful, exiting");
                            std::process::exit(0);
                        }
                        Err(e) => {
                            log::error!("Upgrade failed: {}", e);
                            show_upgrade_error_dialog(&window_clone, &e);
                        }
                    }
                }
            });
            window.add_action(&action);
        }

        {
            let window_clone = window.clone();
            let action = gio::SimpleAction::new("about", None);
            action.connect_activate(move |_, _| {
                dialogs::show_about_dialog(&window_clone);
            });
            window.add_action(&action);
        }

        // Tab Templates action
        {
            let window_clone = window.clone();
            let notebook = notebook.clone();
            let tabs = Rc::clone(&tabs);
            let next_tab_id = Rc::clone(&next_tab_id);
            let config = Rc::clone(&config);
            let theme = theme.clone();
            let tab_bar = tab_bar.clone();
            let has_bell = Rc::clone(&has_bell);
            let file_manager = Rc::clone(&self.file_manager);
            let notification_bar = self.notification_bar.clone();
            let remote_manager = self.remote_manager.clone();
            let action = gio::SimpleAction::new("tab-templates", None);
            action.connect_activate(move |_, _| {
                let notebook = notebook.clone();
                let tabs = Rc::clone(&tabs);
                let next_tab_id = Rc::clone(&next_tab_id);
                let config = Rc::clone(&config);
                let theme = theme.clone();
                let tab_bar = tab_bar.clone();
                let window_for_tab = window_clone.clone();
                let has_bell = Rc::clone(&has_bell);
                let file_manager = Rc::clone(&file_manager);
                let notification_bar = notification_bar.clone();
                let remote_manager = remote_manager.clone();
                crate::tab_templates_dialog::show_tab_templates_dialog_with_open(
                    &window_clone,
                    || {
                        log::info!("Tab templates saved");
                    },
                    move |template| {
                        create_tab_from_template(
                            &notebook,
                            &tabs,
                            &next_tab_id,
                            &config,
                            &theme,
                            &tab_bar,
                            &window_for_tab,
                            &has_bell,
                            &file_manager,
                            &notification_bar,
                            &template,
                            &remote_manager,
                        );
                    },
                );
            });
            window.add_action(&action);
        }

        // View Logs action (debug menu)
        {
            let window_clone = window.clone();
            let action = gio::SimpleAction::new("view-logs", None);
            action.connect_activate(move |_, _| {
                crate::log_viewer::show_log_viewer(&window_clone);
            });
            window.add_action(&action);
        }

        // Debug menu actions (hidden unless Shift is held when opening Help menu)
        {
            // Re-launch cterm - triggers seamless upgrade to the same binary (for testing)
            let tabs = Rc::clone(&tabs);
            let window_clone = window.clone();
            let action = gio::SimpleAction::new("debug-relaunch", None);
            action.connect_activate(move |_, _| {
                log::info!("Debug: Re-launching cterm for seamless upgrade test");

                // Use the executable path captured at startup (immune to binary replacement)
                let current_exe = crate::get_exe_path();
                log::info!("Re-launching from: {:?}", current_exe);

                // Get the current tabs for state collection
                let tabs_borrowed = tabs.borrow();
                let tab_count = tabs_borrowed.len();

                log::info!(
                    "Re-launch would preserve {} tabs (not yet fully implemented)",
                    tab_count
                );

                // Trigger upgrade to same binary via the execute-upgrade action
                let path_str = current_exe.to_string_lossy().to_string();
                if let Err(e) = gtk4::prelude::WidgetExt::activate_action(
                    &window_clone,
                    "win.execute-upgrade",
                    Some(&path_str.to_variant()),
                ) {
                    log::error!("Failed to activate execute-upgrade action: {}", e);
                }
            });
            window.add_action(&action);
        }

        {
            // Re-launch ctermd - trigger daemon exec-in-place relaunch
            let action = gio::SimpleAction::new("debug-relaunch-daemon", None);
            action.connect_activate(move |_, _| {
                log::info!("Debug: Requesting ctermd relaunch");
                std::thread::spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("Failed to create tokio runtime");
                    rt.block_on(async {
                        let socket_path = cterm_client::default_socket_path();
                        match cterm_client::DaemonConnection::connect_unix(&socket_path, false)
                            .await
                        {
                            Ok(conn) => match conn.relaunch_daemon("").await {
                                Ok(resp) => {
                                    if resp.success {
                                        log::info!("ctermd relaunch succeeded");
                                    } else {
                                        log::error!("ctermd relaunch failed: {}", resp.reason);
                                    }
                                }
                                Err(e) => {
                                    log::info!(
                                        "ctermd relaunch in progress (connection dropped: {})",
                                        e
                                    );
                                }
                            },
                            Err(e) => {
                                log::error!("Failed to connect to ctermd for relaunch: {}", e);
                            }
                        }
                    });
                });
            });
            window.add_action(&action);
        }

        {
            // Kill Local ctermd - force shutdown the local daemon
            let action = gio::SimpleAction::new("debug-kill-daemon", None);
            action.connect_activate(move |_, _| {
                log::info!("Debug: Requesting ctermd force shutdown");
                std::thread::spawn(|| {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("Failed to create tokio runtime");
                    rt.block_on(async {
                        let socket_path = cterm_client::default_socket_path();
                        match cterm_client::DaemonConnection::connect_unix(&socket_path, false)
                            .await
                        {
                            Ok(conn) => match conn.shutdown(true).await {
                                Ok(resp) => {
                                    if resp.success {
                                        log::info!("ctermd shutdown succeeded");
                                    } else {
                                        log::error!("ctermd shutdown failed: {}", resp.reason);
                                    }
                                }
                                Err(e) => {
                                    log::info!(
                                        "ctermd shutdown in progress (connection dropped: {})",
                                        e
                                    );
                                }
                            },
                            Err(e) => {
                                log::error!("Failed to connect to ctermd for shutdown: {}", e);
                            }
                        }
                    });
                });
            });
            window.add_action(&action);
        }

        {
            // Dump State - dump current terminal state for debugging
            let tabs = Rc::clone(&tabs);
            let action = gio::SimpleAction::new("debug-dump-state", None);
            action.connect_activate(move |_, _| {
                log::info!("Debug: Dumping terminal state");
                let tabs = tabs.borrow();
                log::info!("Number of tabs: {}", tabs.len());
                for (i, tab) in tabs.iter().enumerate() {
                    log::info!("Tab {}: id={}, title=\"{}\"", i, tab.id, tab.title);
                }
            });
            window.add_action(&action);
        }
    }

    /// Present the window and focus the terminal
    pub fn present(&self) {
        self.window.present();

        // Focus the current terminal after the window is presented
        let notebook = self.notebook.clone();
        let tabs = Rc::clone(&self.tabs);
        glib::idle_add_local_once(move || {
            if let Some(page_idx) = notebook.current_page() {
                let tabs_ref = tabs.borrow();
                if let Some(tab) = tabs_ref.get(page_idx as usize) {
                    tab.terminal.widget().grab_focus();
                }
            }
        });
    }

    /// Set up keyboard event handler
    fn setup_key_handler(&self) {
        let key_controller = EventControllerKey::new();
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        // Disable IM on the window shortcut controller so IBus doesn't
        // swallow Ctrl+Shift+letter events before key-pressed fires.
        key_controller.set_im_context(None::<&gtk4::IMContext>);

        let shortcuts = self.shortcuts.clone();
        let notebook = self.notebook.clone();
        let tabs = Rc::clone(&self.tabs);
        let next_tab_id = Rc::clone(&self.next_tab_id);
        let window = self.window.clone();
        let config = self.config.clone();
        let theme = self.theme.clone();
        let tab_bar = self.tab_bar.clone();
        let has_bell = Rc::clone(&self.has_bell);
        let file_manager = Rc::clone(&self.file_manager);
        let notification_bar = self.notification_bar.clone();

        key_controller.connect_key_pressed(move |_, keyval, _keycode, state| {
            // Convert GTK modifiers to our modifiers
            let mut modifiers = gtk_modifiers_to_modifiers(state);

            // GTK4 on X11 consumes Shift to produce uppercase keyvals,
            // removing SHIFT_MASK from the state. Detect Shift from the keyval.
            if !modifiers.contains(Modifiers::SHIFT) {
                if let Some(c) = keyval.to_unicode() {
                    if c.is_uppercase() {
                        modifiers.insert(Modifiers::SHIFT);
                    }
                }
            }

            // Convert keyval to our key code
            if let Some(key) = keyval_to_keycode(keyval) {
                // Check for shortcut match
                if let Some(action) = shortcuts.match_event(key, modifiers) {
                    match action {
                        Action::NewTab => {
                            let (cwd, daemon_socket) = {
                                let tabs_borrow = tabs.borrow();
                                if let Some(page_idx) = notebook.current_page() {
                                    let entry = tabs_borrow.get(page_idx as usize);
                                    #[cfg(unix)]
                                    let cwd = entry.and_then(|e| e.terminal.foreground_cwd());
                                    #[cfg(not(unix))]
                                    let cwd: Option<String> = None;
                                    let socket = entry.and_then(|e| e.daemon_socket.clone());
                                    (cwd, socket)
                                } else {
                                    (None, None)
                                }
                            };

                            create_new_tab(
                                &notebook,
                                &tabs,
                                &next_tab_id,
                                &config,
                                &theme,
                                &tab_bar,
                                &window,
                                &has_bell,
                                &file_manager,
                                &notification_bar,
                                cwd,
                                daemon_socket,
                            );
                            return glib::Propagation::Stop;
                        }
                        Action::CloseTab => {
                            close_current_tab(&notebook, &tabs, &tab_bar, &window, &config);
                            return glib::Propagation::Stop;
                        }
                        Action::NextTab => {
                            let n = notebook.n_pages();
                            if n > 0 {
                                let current = notebook.current_page().unwrap_or(0);
                                notebook.set_current_page(Some((current + 1) % n));
                                sync_tab_bar_active(&tab_bar, &tabs, &notebook);
                                focus_current_terminal(&notebook);
                            }
                            return glib::Propagation::Stop;
                        }
                        Action::PrevTab => {
                            let n = notebook.n_pages();
                            if n > 0 {
                                let current = notebook.current_page().unwrap_or(0);
                                let prev = if current == 0 { n - 1 } else { current - 1 };
                                notebook.set_current_page(Some(prev));
                                sync_tab_bar_active(&tab_bar, &tabs, &notebook);
                                focus_current_terminal(&notebook);
                            }
                            return glib::Propagation::Stop;
                        }
                        Action::NextAlertedTab => {
                            let n = notebook.n_pages();
                            if n > 0 {
                                let current = notebook.current_page().unwrap_or(0) as usize;
                                let tabs_ref = tabs.borrow();
                                for offset in 1..tabs_ref.len() {
                                    let idx = (current + offset) % tabs_ref.len();
                                    if let Some(entry) = tabs_ref.get(idx) {
                                        if tab_bar.has_bell(entry.id) {
                                            drop(tabs_ref);
                                            notebook.set_current_page(Some(idx as u32));
                                            sync_tab_bar_active(&tab_bar, &tabs, &notebook);
                                            focus_current_terminal(&notebook);
                                            break;
                                        }
                                    }
                                }
                            }
                            return glib::Propagation::Stop;
                        }
                        Action::Tab(n) => {
                            let idx = (*n as u32).saturating_sub(1);
                            if idx < notebook.n_pages() {
                                notebook.set_current_page(Some(idx));
                                sync_tab_bar_active(&tab_bar, &tabs, &notebook);
                                focus_current_terminal(&notebook);
                            }
                            return glib::Propagation::Stop;
                        }
                        Action::Copy => {
                            // Copy selection to clipboard
                            if let Some(page_idx) = notebook.current_page() {
                                let tabs_ref = tabs.borrow();
                                if let Some(tab) = tabs_ref.get(page_idx as usize) {
                                    tab.terminal.copy_selection();
                                }
                            }
                            return glib::Propagation::Stop;
                        }
                        Action::Paste => {
                            // Get clipboard and paste to current terminal
                            if let Some(display) = gdk::Display::default() {
                                let clipboard = display.clipboard();
                                let tabs_paste = Rc::clone(&tabs);
                                let notebook_paste = notebook.clone();
                                clipboard.read_text_async(
                                    None::<&gio::Cancellable>,
                                    move |result| {
                                        if let Ok(Some(text)) = result {
                                            // Find current terminal and write
                                            if let Some(page_idx) = notebook_paste.current_page() {
                                                let tabs = tabs_paste.borrow();
                                                if let Some(tab) = tabs.get(page_idx as usize) {
                                                    tab.terminal.write_str(&text);
                                                }
                                            }
                                        }
                                    },
                                );
                            }
                            return glib::Propagation::Stop;
                        }
                        Action::ZoomIn => {
                            if let Some(page_idx) = notebook.current_page() {
                                let tabs_ref = tabs.borrow();
                                if let Some(tab) = tabs_ref.get(page_idx as usize) {
                                    tab.terminal.zoom_in();
                                }
                            }
                            return glib::Propagation::Stop;
                        }
                        Action::ZoomOut => {
                            if let Some(page_idx) = notebook.current_page() {
                                let tabs_ref = tabs.borrow();
                                if let Some(tab) = tabs_ref.get(page_idx as usize) {
                                    tab.terminal.zoom_out();
                                }
                            }
                            return glib::Propagation::Stop;
                        }
                        Action::ZoomReset => {
                            if let Some(page_idx) = notebook.current_page() {
                                let tabs_ref = tabs.borrow();
                                if let Some(tab) = tabs_ref.get(page_idx as usize) {
                                    tab.terminal.zoom_reset();
                                }
                            }
                            return glib::Propagation::Stop;
                        }
                        Action::NewWindow => {
                            gtk4::prelude::ActionGroupExt::activate_action(
                                &window,
                                "new-window",
                                None,
                            );
                            return glib::Propagation::Stop;
                        }
                        Action::CloseWindow => {
                            window.close();
                            return glib::Propagation::Stop;
                        }
                        Action::QuickOpenTemplate => {
                            // Activate the quick-open action
                            gtk4::prelude::ActionGroupExt::activate_action(
                                &window,
                                "quick-open",
                                None,
                            );
                            return glib::Propagation::Stop;
                        }
                        _ => {}
                    }
                }
            }

            // Pass to terminal
            glib::Propagation::Proceed
        });

        self.window.add_controller(key_controller);
    }

    /// Set up window focus handler to clear bell when window becomes active
    /// and send focus events to the terminal (DECSET 1004)
    fn setup_focus_handler(&self) {
        let has_bell = Rc::clone(&self.has_bell);
        let window = self.window.clone();
        let tab_bar = self.tab_bar.clone();
        let tabs = Rc::clone(&self.tabs);
        let notebook = self.notebook.clone();

        self.window.connect_is_active_notify(move |win| {
            let is_active = win.is_active();

            // Send focus event to the active terminal (DECSET 1004)
            if let Some(page_idx) = notebook.current_page() {
                let tabs_borrowed = tabs.borrow();
                if let Some(tab) = tabs_borrowed.get(page_idx as usize) {
                    tab.terminal.send_focus_event(is_active);
                }
            }

            if is_active {
                // Window became active, clear bell indicator
                let mut bell = has_bell.borrow_mut();
                if *bell {
                    *bell = false;
                    window.set_title(Some("cterm"));

                    // Clear bell on the currently active tab
                    if let Some(page_idx) = notebook.current_page() {
                        let tabs = tabs.borrow();
                        if let Some(tab) = tabs.get(page_idx as usize) {
                            tab_bar.clear_bell(tab.id);
                            tab.terminal.clear_alert();
                        }
                    }
                }
            }
        });
    }

    /// Set up terminal focus restoration
    ///
    /// When keys are pressed and focus is not on the terminal (e.g., after
    /// closing a menu), automatically restore focus to the terminal and
    /// forward the key to the terminal so it's not lost.
    fn setup_terminal_focus_restore(&self) {
        let focus_controller = EventControllerKey::new();
        focus_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        // Disable IM on the focus-restore controller too, so IBus doesn't
        // consume key events before our handler runs.
        focus_controller.set_im_context(None::<&gtk4::IMContext>);

        let notebook = self.notebook.clone();
        let tabs = Rc::clone(&self.tabs);

        focus_controller.connect_key_pressed(move |_controller, keyval, _keycode, state| {
            // Skip modifier keys and menu activation keys
            let is_modifier = matches!(
                keyval,
                gdk::Key::Shift_L
                    | gdk::Key::Shift_R
                    | gdk::Key::Control_L
                    | gdk::Key::Control_R
                    | gdk::Key::Alt_L
                    | gdk::Key::Alt_R
                    | gdk::Key::Super_L
                    | gdk::Key::Super_R
                    | gdk::Key::Meta_L
                    | gdk::Key::Meta_R
                    | gdk::Key::F10
            );

            if is_modifier {
                return glib::Propagation::Proceed;
            }

            // Check if the terminal widget itself has focus.
            // (focus_child() only returns the direct child, not the deeply
            // nested DrawingArea, so we check has_focus() on the actual widget.)
            let terminal_has_focus = notebook
                .current_page()
                .and_then(|idx| {
                    let tabs_ref = tabs.borrow();
                    tabs_ref
                        .get(idx as usize)
                        .map(|tab| tab.terminal.widget().has_focus())
                })
                .unwrap_or(false);

            if !terminal_has_focus {
                // Focus is not on terminal - restore it and forward the key
                if let Some(page_idx) = notebook.current_page() {
                    let tabs_ref = tabs.borrow();
                    if let Some(tab) = tabs_ref.get(page_idx as usize) {
                        // Grab focus
                        tab.terminal.widget().grab_focus();

                        // Forward the key to the terminal
                        let has_ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
                        let has_alt = state.contains(gdk::ModifierType::ALT_MASK);
                        let has_shift = state.contains(gdk::ModifierType::SHIFT_MASK)
                            || keyval.to_unicode().is_some_and(|c| c.is_uppercase());

                        // Don't forward Ctrl+Shift combinations - those are
                        // shortcuts handled by the key_controller.
                        if has_ctrl && has_shift {
                            return glib::Propagation::Proceed;
                        }

                        if let Some(c) = keyval.to_unicode() {
                            if has_ctrl && !has_alt {
                                // Ctrl+key - convert to control character
                                let ctrl_char = match c.to_ascii_lowercase() {
                                    'a'..='z' => {
                                        Some((c.to_ascii_lowercase() as u8 - b'a' + 1) as char)
                                    }
                                    '[' | '3' => Some('\x1b'), // Escape
                                    '\\' | '4' => Some('\x1c'),
                                    ']' | '5' => Some('\x1d'),
                                    '^' | '6' => Some('\x1e'),
                                    '_' | '7' => Some('\x1f'),
                                    '@' | '2' => Some('\x00'),
                                    _ => None,
                                };
                                if let Some(ctrl) = ctrl_char {
                                    tab.terminal.write_str(&ctrl.to_string());
                                    tab.terminal.widget().queue_draw();
                                    return glib::Propagation::Stop;
                                }
                            } else if !has_ctrl && !has_alt {
                                // Simple character - write directly
                                let mut s = [0u8; 4];
                                let s = c.encode_utf8(&mut s);
                                tab.terminal.write_str(s);
                                tab.terminal.widget().queue_draw();
                                return glib::Propagation::Stop;
                            }
                        }

                        // For special keys or Alt combinations, let the terminal's
                        // key handler process it. Focus is now on the terminal.
                    }
                }
            }

            glib::Propagation::Proceed
        });

        self.window.add_controller(focus_controller);
    }

    /// Set up notification bar callbacks for file transfers
    fn setup_notification_bar(&self) {
        let file_manager = Rc::clone(&self.file_manager);
        let notification_bar = self.notification_bar.clone();
        let window = self.window.clone();

        // Save button - save to default location (Downloads or last saved dir)
        let file_manager_save = Rc::clone(&file_manager);
        let notification_bar_save = notification_bar.clone();
        notification_bar.set_on_save(move |id| {
            let mut manager = file_manager_save.borrow_mut();
            if let Some(path) = manager.default_save_path() {
                match manager.save_to_path(id, &path) {
                    Ok(size) => {
                        log::info!("Saved file to {:?} ({} bytes)", path, size);
                    }
                    Err(e) => {
                        log::error!("Failed to save file: {}", e);
                    }
                }
            }
            drop(manager);
            notification_bar_save.hide();
        });

        // Save As button - show file chooser dialog
        let file_manager_save_as = Rc::clone(&file_manager);
        let notification_bar_save_as = notification_bar.clone();
        notification_bar.set_on_save_as(move |id| {
            let manager = file_manager_save_as.borrow();
            let suggested_name = manager.suggested_filename().map(|s| s.to_string());
            let initial_dir = manager.last_save_dir().cloned();
            drop(manager);

            let file_chooser = gtk4::FileChooserDialog::new(
                Some("Save File As"),
                Some(&window),
                gtk4::FileChooserAction::Save,
                &[
                    ("Cancel", gtk4::ResponseType::Cancel),
                    ("Save", gtk4::ResponseType::Accept),
                ],
            );

            // Set suggested filename
            if let Some(name) = suggested_name {
                file_chooser.set_current_name(&name);
            }

            // Set initial folder
            if let Some(dir) = initial_dir {
                let file = gio::File::for_path(&dir);
                file_chooser.set_current_folder(Some(&file)).ok();
            } else if let Some(downloads) = cterm_app::file_transfer::dirs::download_dir() {
                let file = gio::File::for_path(&downloads);
                file_chooser.set_current_folder(Some(&file)).ok();
            }

            let file_manager_dialog = Rc::clone(&file_manager_save_as);
            let notification_bar_dialog = notification_bar_save_as.clone();

            file_chooser.connect_response(move |dialog, response| {
                if response == gtk4::ResponseType::Accept {
                    if let Some(file) = dialog.file() {
                        if let Some(path) = file.path() {
                            let mut manager = file_manager_dialog.borrow_mut();
                            match manager.save_to_path(id, &path) {
                                Ok(size) => {
                                    log::info!("Saved file to {:?} ({} bytes)", path, size);
                                }
                                Err(e) => {
                                    log::error!("Failed to save file: {}", e);
                                }
                            }
                        }
                    }
                }
                notification_bar_dialog.hide();
                dialog.close();
            });

            file_chooser.present();
        });

        // Discard button - discard the pending file
        let file_manager_discard = Rc::clone(&file_manager);
        let notification_bar_discard = notification_bar.clone();
        notification_bar.set_on_discard(move |id| {
            file_manager_discard.borrow_mut().discard(id);
            notification_bar_discard.hide();
            log::debug!("Discarded pending file {}", id);
        });
    }

    /// Set up Quick Open overlay callback
    /// Poll the Unix Shells DeviceService for changes and rebuild the menu.
    fn setup_unixshells_menu_poller(&self) {
        let ds = std::sync::Arc::clone(&self.unixshells_service);
        let menu = self.unixshells_menu.clone();
        let last_version = Rc::new(RefCell::new(0u64));

        glib::timeout_add_local(std::time::Duration::from_secs(2), move || {
            let current = ds.version.load(std::sync::atomic::Ordering::Relaxed);
            if current == *last_version.borrow() {
                return glib::ControlFlow::Continue;
            }
            *last_version.borrow_mut() = current;

            // Rebuild the Unix Shells submenu
            menu.remove_all();

            match ds.login_state() {
                cterm_app::unixshells::LoginState::LoggedOut => {
                    menu.append(Some("Sign In..."), Some("win.unixshells-signin"));
                }
                cterm_app::unixshells::LoginState::PendingApproval { username, .. } => {
                    let item =
                        gio::MenuItem::new(Some(&format!("{} (pending...)", username)), None);
                    item.set_attribute_value("action", None::<&glib::Variant>);
                    menu.append_item(&item);
                }
                cterm_app::unixshells::LoginState::LoggedIn { ref username } => {
                    // Username label (non-clickable)
                    let item = gio::MenuItem::new(Some(username), None);
                    menu.append_item(&item);

                    // Devices
                    let devices = ds.devices();
                    if !devices.is_empty() {
                        let devices_section = gio::Menu::new();
                        for device in &devices {
                            let has_sessions = device.sessions.iter().any(|s| s.status == "alive");
                            let label = if has_sessions || device.online {
                                format!("{} (online)", device.device)
                            } else {
                                format!("{} (offline)", device.device)
                            };

                            if has_sessions || device.online {
                                let action = format!("win.unixshells-connect::{}", device.device);
                                devices_section.append(Some(&label), Some(&action));
                            } else {
                                // Offline device — no action
                                let item = gio::MenuItem::new(Some(&label), None);
                                devices_section.append_item(&item);
                            }
                        }
                        menu.append_section(None, &devices_section);
                    }

                    // Sign out
                    let signout_section = gio::Menu::new();
                    signout_section.append(Some("Sign Out"), Some("win.unixshells-signout"));
                    menu.append_section(None, &signout_section);
                }
            }

            glib::ControlFlow::Continue
        });
    }

    fn setup_quick_open(&self) {
        let notebook = self.notebook.clone();
        let tabs = Rc::clone(&self.tabs);
        let next_tab_id = Rc::clone(&self.next_tab_id);
        let config = Rc::clone(&self.config);
        let theme = self.theme.clone();
        let tab_bar = self.tab_bar.clone();
        let window = self.window.clone();
        let has_bell = Rc::clone(&self.has_bell);
        let file_manager = Rc::clone(&self.file_manager);
        let notification_bar = self.notification_bar.clone();
        let remote_manager = self.remote_manager.clone();

        self.quick_open.set_on_select(move |template| {
            create_tab_from_template(
                &notebook,
                &tabs,
                &next_tab_id,
                &config,
                &theme,
                &tab_bar,
                &window,
                &has_bell,
                &file_manager,
                &notification_bar,
                &template,
                &remote_manager,
            );
            log::info!("Opened template tab from Quick Open: {}", template.name);
        });
    }

    /// Set up tab bar callbacks
    fn setup_tab_bar_callbacks(&self) {
        let notebook = self.notebook.clone();
        let tabs = Rc::clone(&self.tabs);
        let next_tab_id = Rc::clone(&self.next_tab_id);
        let config = self.config.clone();
        let theme = self.theme.clone();
        let tab_bar = self.tab_bar.clone();
        let window = self.window.clone();
        let has_bell = Rc::clone(&self.has_bell);
        let file_manager = Rc::clone(&self.file_manager);
        let notification_bar = self.notification_bar.clone();

        // New tab button
        self.tab_bar.set_on_new_tab(move || {
            // Get info from the active terminal
            let (cwd, daemon_socket) = {
                let tabs_borrow = tabs.borrow();
                if let Some(page_idx) = notebook.current_page() {
                    let entry = tabs_borrow.get(page_idx as usize);
                    #[cfg(unix)]
                    let cwd = entry.and_then(|e| e.terminal.foreground_cwd());
                    #[cfg(not(unix))]
                    let cwd: Option<String> = None;
                    let socket = entry.and_then(|e| e.daemon_socket.clone());
                    (cwd, socket)
                } else {
                    (None, None)
                }
            };

            create_new_tab(
                &notebook,
                &tabs,
                &next_tab_id,
                &config,
                &theme,
                &tab_bar,
                &window,
                &has_bell,
                &file_manager,
                &notification_bar,
                cwd,
                daemon_socket,
            );
        });

        // Rename tab (right-click context menu)
        {
            let tabs = Rc::clone(&self.tabs);
            let tab_bar = self.tab_bar.clone();
            let window = self.window.clone();
            self.tab_bar.set_on_rename(move |tab_id| {
                show_rename_tab_dialog(&window, &tabs, &tab_bar, tab_id);
            });
        }

        // Set tab color (right-click context menu)
        {
            let tabs = Rc::clone(&self.tabs);
            let tab_bar = self.tab_bar.clone();
            let window = self.window.clone();
            self.tab_bar.set_on_set_color(move |tab_id| {
                let tab_bar_clone = tab_bar.clone();
                let tabs_clone = Rc::clone(&tabs);
                dialogs::show_set_color_dialog(&window, move |color| {
                    let mut tabs = tabs_clone.borrow_mut();
                    if let Some(tab) = tabs.iter_mut().find(|t| t.id == tab_id) {
                        tab_bar_clone.set_color(tab_id, color.as_deref());
                        tab.terminal
                            .set_tab_color_on_daemon(color.as_deref().unwrap_or(""));
                        tab.color = color;
                    }
                });
            });
        }
    }

    /// Set up close request handler to confirm when closing with running processes
    #[cfg(unix)]
    fn setup_close_request_handler(&self) {
        let tabs = Rc::clone(&self.tabs);
        let config = Rc::clone(&self.config);
        let window = self.window.clone();

        self.window.connect_close_request(move |win| {
            let confirm_close = config.borrow().general.confirm_close_with_running;
            if !confirm_close {
                return glib::Propagation::Proceed;
            }

            // Collect session info for daemon queries
            let tab_infos: Vec<(String, Option<std::path::PathBuf>, String)> = {
                let tabs = tabs.borrow();
                tabs.iter()
                    .filter_map(|tab| {
                        let sid = tab.session_id.clone()?;
                        if sid.is_empty() {
                            return None;
                        }
                        Some((sid, tab.daemon_socket.clone(), tab.title.clone()))
                    })
                    .collect()
            };

            if tab_infos.is_empty() {
                return glib::Propagation::Proceed;
            }

            // Query the daemon asynchronously for each session's foreground process
            let window_clone = window.clone();
            let win_ref = win.clone();
            let (result_tx, result_rx) = std::sync::mpsc::channel();

            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let results = rt.block_on(async {
                    let mut running = Vec::new();
                    for (session_id, daemon_socket, title) in &tab_infos {
                        let conn = match if let Some(ref path) = daemon_socket {
                            cterm_client::DaemonConnection::connect_unix(path, false).await
                        } else {
                            cterm_client::DaemonConnection::connect_local().await
                        } {
                            Ok(c) => c,
                            Err(_) => continue,
                        };
                        if let Ok(info) = conn.get_session(session_id).await {
                            if info.has_foreground_process {
                                let name = if info.foreground_process_name.is_empty() {
                                    "a process".to_string()
                                } else {
                                    info.foreground_process_name
                                };
                                running.push((title.clone(), name));
                            }
                        }
                    }
                    running
                });
                let _ = result_tx.send(results);
            });

            glib::idle_add_local_once(move || {
                let running_processes =
                    match result_rx.recv_timeout(std::time::Duration::from_secs(2)) {
                        Ok(r) => r,
                        Err(_) => {
                            window_clone.destroy();
                            return;
                        }
                    };

                if running_processes.is_empty() {
                    window_clone.destroy();
                } else {
                    let wc = window_clone.clone();
                    dialogs::show_close_confirmation_dialog(
                        &win_ref,
                        running_processes,
                        move |confirmed| {
                            if confirmed {
                                wc.destroy();
                            }
                        },
                    );
                }
            });

            glib::Propagation::Stop
        });
    }

    /// Set up close request handler (non-Unix fallback - no process detection)
    #[cfg(not(unix))]
    fn setup_close_request_handler(&self) {
        // No process detection on non-Unix platforms
    }

    /// Create a new tab
    pub fn new_tab(&self) {
        create_new_tab(
            &self.notebook,
            &self.tabs,
            &self.next_tab_id,
            &self.config,
            &self.theme,
            &self.tab_bar,
            &self.window,
            &self.has_bell,
            &self.file_manager,
            &self.notification_bar,
            None,
            None,
        );
    }

    /// Add a tab for a reconnected daemon session (with screen snapshot).
    ///
    /// Used during startup reconnection to create tabs for existing daemon sessions.
    pub fn add_reconnected_tab(
        &self,
        recon: cterm_app::daemon_reconnect::ReconnectedSession,
        tab_color: Option<String>,
    ) {
        let sid = recon.handle.session_id().to_string();
        let daemon_socket = recon.handle.socket_path().map(|p| p.to_owned());
        // Prefer custom_title (user-set), then daemon title, then fallback
        let (title, title_locked) = if !recon.custom_title.is_empty() {
            (recon.custom_title.clone(), true)
        } else if !recon.title.is_empty() {
            (recon.title.clone(), false)
        } else {
            ("Terminal".to_string(), false)
        };

        // Use explicit tab_color if provided (upgrade path), otherwise use daemon's
        let effective_color = tab_color.or_else(|| {
            if recon.tab_color.is_empty() {
                None
            } else {
                Some(recon.tab_color.clone())
            }
        });

        let cfg = self.config.borrow();
        let terminal = TerminalWidget::from_daemon_with_screen(recon, &cfg, &self.theme);
        drop(cfg);

        let tab_id = generate_tab_id(&self.next_tab_id);
        let page_num = self
            .notebook
            .append_page(terminal.widget(), None::<&gtk4::Widget>);
        self.tab_bar.add_tab(tab_id, &title);

        setup_tab_callbacks(
            &self.notebook,
            &self.tabs,
            &self.config,
            &self.tab_bar,
            &self.window,
            &self.has_bell,
            &self.file_manager,
            &self.notification_bar,
            &terminal,
            tab_id,
            false,
        );

        finalize_new_tab(
            &self.notebook,
            &self.tabs,
            &self.tab_bar,
            tab_id,
            page_num,
            title,
            terminal,
            title_locked,
            Some(sid),
            daemon_socket,
        );

        // Restore tab color if available
        if let Some(ref color) = effective_color {
            self.tab_bar.set_color(tab_id, Some(color));
            if let Some(tab) = self.tabs.borrow_mut().iter_mut().find(|t| t.id == tab_id) {
                tab.color = effective_color;
            }
        }
    }

    /// Update window title when switching tabs
    fn setup_tab_switch_handler(&self) {
        let tabs = Rc::clone(&self.tabs);
        let window = self.window.clone();
        let tab_bar = self.tab_bar.clone();
        let has_bell = Rc::clone(&self.has_bell);
        self.notebook.connect_switch_page(move |_, _, page_num| {
            let tabs = tabs.borrow();
            if let Some(tab) = tabs.get(page_num as usize) {
                window.set_title(Some(&tab.title));
                tab_bar.set_active(tab.id);
                tab_bar.clear_bell(tab.id);
                tab.terminal.clear_alert();
                *has_bell.borrow_mut() = false;
            }
        });
    }
}

/// Show the rename dialog for a tab and persist the new title.
/// Used by both the menu bar "Set Title" action and the right-click context menu.
fn show_rename_tab_dialog(
    window: &ApplicationWindow,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    tab_bar: &TabBar,
    tab_id: u64,
) {
    let current_title = {
        let tabs = tabs.borrow();
        tabs.iter()
            .find(|t| t.id == tab_id)
            .map(|t| t.title.clone())
            .unwrap_or_default()
    };
    let tabs_clone = Rc::clone(tabs);
    let tab_bar_clone = tab_bar.clone();
    let window_clone = window.clone();
    dialogs::show_set_title_dialog(window, &current_title, move |new_title| {
        let mut tabs = tabs_clone.borrow_mut();
        if let Some(tab) = tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.title = new_title.clone();
            tab.title_locked = true;
            tab_bar_clone.set_title(tab_id, &new_title);
            window_clone.set_title(Some(&new_title));
            // Persist custom title to daemon
            tab.terminal.set_custom_title(&new_title);
        }
    });
}

/// Generate a unique tab ID from the shared counter
fn generate_tab_id(next_tab_id: &Rc<RefCell<u64>>) -> u64 {
    let mut id = next_tab_id.borrow_mut();
    let current = *id;
    *id += 1;
    current
}

/// Set up all standard callbacks for a tab (close, click, exit, bell, title, file transfer)
#[allow(clippy::too_many_arguments)]
fn setup_tab_callbacks(
    notebook: &Notebook,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    config: &Rc<RefCell<Config>>,
    tab_bar: &TabBar,
    window: &ApplicationWindow,
    has_bell: &Rc<RefCell<bool>>,
    file_manager: &Rc<RefCell<PendingFileManager>>,
    notification_bar: &NotificationBar,
    terminal: &TerminalWidget,
    tab_id: u64,
    keep_open: bool,
) {
    // Close callback (with confirmation for running processes)
    let notebook_close = notebook.clone();
    let tabs_close = Rc::clone(tabs);
    let tab_bar_close = tab_bar.clone();
    let window_close = window.clone();
    let config_close = Rc::clone(config);
    tab_bar.set_on_close(tab_id, move || {
        request_close_tab_by_id(
            &notebook_close,
            &tabs_close,
            &tab_bar_close,
            &window_close,
            &config_close,
            tab_id,
        );
    });

    // Click callback
    let notebook_click = notebook.clone();
    let tabs_click = Rc::clone(tabs);
    let tab_bar_click = tab_bar.clone();
    tab_bar.set_on_click(tab_id, move || {
        let tabs = tabs_click.borrow();
        if let Some(idx) = tabs.iter().position(|t| t.id == tab_id) {
            notebook_click.set_current_page(Some(idx as u32));
            tab_bar_click.set_active(tab_id);
            tab_bar_click.clear_bell(tab_id);
            tabs[idx].terminal.clear_alert();
            if let Some(widget) = notebook_click.nth_page(Some(idx as u32)) {
                widget.grab_focus();
            }
        }
    });

    // Exit callback
    let notebook_exit = notebook.clone();
    let tabs_exit = Rc::clone(tabs);
    let tab_bar_exit = tab_bar.clone();
    let window_exit = window.clone();
    terminal.set_on_exit(move || {
        if !keep_open {
            close_tab_by_id(
                &notebook_exit,
                &tabs_exit,
                &tab_bar_exit,
                &window_exit,
                tab_id,
            );
        }
    });

    // Bell callback
    let tab_bar_bell = tab_bar.clone();
    let notebook_bell = notebook.clone();
    let tabs_bell = Rc::clone(tabs);
    let window_bell = window.clone();
    let has_bell_bell = Rc::clone(has_bell);
    terminal.set_on_bell(move || {
        let is_window_active = window_bell.is_active();
        let is_current_tab = if let Some(current_page) = notebook_bell.current_page() {
            let tabs = tabs_bell.borrow();
            tabs.get(current_page as usize)
                .map(|t| t.id == tab_id)
                .unwrap_or(false)
        } else {
            false
        };

        if !is_current_tab || !is_window_active {
            tab_bar_bell.set_bell(tab_id, true);
        }

        if !is_window_active {
            *has_bell_bell.borrow_mut() = true;
            window_bell.set_title(Some("🔔 cterm"));
        }
    });

    // Title change callback
    let tab_bar_title = tab_bar.clone();
    let tabs_title = Rc::clone(tabs);
    let window_title = window.clone();
    let notebook_title = notebook.clone();
    let has_bell_title = Rc::clone(has_bell);
    terminal.set_on_title_change(move |title| {
        // Check if title is locked (user-set or template)
        {
            let tabs = tabs_title.borrow();
            if let Some(entry) = tabs.iter().find(|t| t.id == tab_id) {
                if entry.title_locked {
                    return;
                }
            }
        }

        // Update tab bar
        tab_bar_title.set_title(tab_id, title);

        // Update stored title in tabs
        {
            let mut tabs = tabs_title.borrow_mut();
            if let Some(entry) = tabs.iter_mut().find(|t| t.id == tab_id) {
                entry.title = title.to_string();
            }
        }

        // Update window title if this is the active tab
        if let Some(current_page) = notebook_title.current_page() {
            let tabs = tabs_title.borrow();
            if tabs
                .get(current_page as usize)
                .map(|t| t.id == tab_id)
                .unwrap_or(false)
            {
                *has_bell_title.borrow_mut() = false;
                window_title.set_title(Some(title));
            }
        }
    });

    // File transfer callback
    let file_manager_transfer = Rc::clone(file_manager);
    let notification_bar_transfer = notification_bar.clone();
    terminal.set_on_file_transfer(move |transfer| {
        use cterm_core::FileTransferOperation;

        match transfer {
            FileTransferOperation::FileReceived { id, name, data } => {
                log::info!(
                    "File received: id={}, name={:?}, size={}",
                    id,
                    name,
                    data.len()
                );
                let size = data.len();
                let mut manager = file_manager_transfer.borrow_mut();
                manager.set_pending(id, name.clone(), data);
                drop(manager);
                notification_bar_transfer.show_file(id, name.as_deref(), size);
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
                let mut manager = file_manager_transfer.borrow_mut();
                manager.set_pending_streaming(id, name.clone(), result.data);
                drop(manager);
                notification_bar_transfer.show_file(id, name.as_deref(), size);
            }
        }
    });
}

/// Finalize a new tab: store entry, update visibility, switch to it, and focus
#[allow(clippy::too_many_arguments)]
fn finalize_new_tab(
    notebook: &Notebook,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    tab_bar: &TabBar,
    tab_id: u64,
    page_num: u32,
    title: String,
    terminal: TerminalWidget,
    title_locked: bool,
    session_id: Option<String>,
    daemon_socket: Option<std::path::PathBuf>,
) {
    tabs.borrow_mut().push(TabEntry {
        id: tab_id,
        title,
        terminal,
        title_locked,
        color: None,
        session_id,
        daemon_socket,
    });

    tab_bar.update_visibility();
    notebook.set_current_page(Some(page_num));
    tab_bar.set_active(tab_id);

    if let Some(widget) = notebook.nth_page(Some(page_num)) {
        widget.grab_focus();
    }
}

/// Create a new terminal tab (daemon-backed via ctermd)
///
/// If `daemon_socket` is Some, creates the session on that specific daemon
/// (e.g. an SSH-tunneled remote). Otherwise uses the local daemon.
#[allow(clippy::too_many_arguments)]
fn create_new_tab(
    notebook: &Notebook,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    next_tab_id: &Rc<RefCell<u64>>,
    config: &Rc<RefCell<Config>>,
    theme: &Theme,
    tab_bar: &TabBar,
    window: &ApplicationWindow,
    has_bell: &Rc<RefCell<bool>>,
    file_manager: &Rc<RefCell<PendingFileManager>>,
    notification_bar: &NotificationBar,
    cwd: Option<String>,
    daemon_socket: Option<std::path::PathBuf>,
) {
    let cfg = config.borrow();

    // Get shell basename for initial title
    let shell = cfg
        .general
        .default_shell
        .clone()
        .unwrap_or_else(|| std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string()));
    let initial_title = std::path::Path::new(&shell)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Terminal")
        .to_string();

    // Build daemon session options — for remote daemons, don't pass local shell/args
    let opts = if daemon_socket.is_some() {
        cterm_client::CreateSessionOpts {
            cols: 80,
            rows: 24,
            ..Default::default()
        }
    } else {
        cterm_client::CreateSessionOpts {
            cols: 80,
            rows: 24,
            shell: cfg.general.default_shell.clone(),
            args: cfg.general.shell_args.clone(),
            cwd,
            ..Default::default()
        }
    };
    drop(cfg);

    spawn_daemon_tab(
        notebook,
        tabs,
        next_tab_id,
        config,
        theme,
        tab_bar,
        window,
        has_bell,
        file_manager,
        notification_bar,
        opts,
        initial_title,
        None,
        None,
        false,
        None,
        daemon_socket,
    );
}

/// Create a new Docker terminal tab (daemon-backed via ctermd)
#[allow(clippy::too_many_arguments)]
fn create_docker_tab(
    notebook: &Notebook,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    next_tab_id: &Rc<RefCell<u64>>,
    config: &Rc<RefCell<Config>>,
    theme: &Theme,
    tab_bar: &TabBar,
    window: &ApplicationWindow,
    has_bell: &Rc<RefCell<bool>>,
    file_manager: &Rc<RefCell<PendingFileManager>>,
    notification_bar: &NotificationBar,
    command: &str,
    args: &[String],
    title: &str,
) {
    let opts = cterm_client::CreateSessionOpts {
        cols: 80,
        rows: 24,
        shell: Some(command.to_string()),
        args: args.to_vec(),
        ..Default::default()
    };

    spawn_daemon_tab(
        notebook,
        tabs,
        next_tab_id,
        config,
        theme,
        tab_bar,
        window,
        has_bell,
        file_manager,
        notification_bar,
        opts,
        title.to_string(),
        Some("#0db7ed".to_string()),
        None,
        false,
        None,
        None,
    );
}

/// Spawn a new daemon-backed tab: connects to ctermd, creates session, and wires up the tab
///
/// Connection priority:
/// 1. `daemon_socket` — connect to a specific socket (e.g. SSH-tunneled remote)
/// 2. `remote` — connect via RemoteManager (template-based remotes)
/// 3. Neither — connect to local daemon
#[allow(clippy::too_many_arguments)]
fn spawn_daemon_tab(
    notebook: &Notebook,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    next_tab_id: &Rc<RefCell<u64>>,
    config: &Rc<RefCell<Config>>,
    theme: &Theme,
    tab_bar: &TabBar,
    window: &ApplicationWindow,
    has_bell: &Rc<RefCell<bool>>,
    file_manager: &Rc<RefCell<PendingFileManager>>,
    notification_bar: &NotificationBar,
    opts: cterm_client::CreateSessionOpts,
    title: String,
    color: Option<String>,
    background_color: Option<String>,
    keep_open: bool,
    remote: Option<(cterm_client::RemoteManager, String, String, bool)>,
    daemon_socket: Option<std::path::PathBuf>,
) {
    let notebook = notebook.clone();
    let tabs = Rc::clone(tabs);
    let next_tab_id = Rc::clone(next_tab_id);
    let config = Rc::clone(config);
    let theme = theme.clone();
    let tab_bar = tab_bar.clone();
    let window = window.clone();
    let has_bell = Rc::clone(has_bell);
    let file_manager = Rc::clone(file_manager);
    let notification_bar = notification_bar.clone();

    let (tx, rx) = std::sync::mpsc::channel::<DaemonAttachResult>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();

        let result = match rt {
            Ok(rt) => rt.block_on(async {
                let conn = if let Some(ref path) = daemon_socket {
                    cterm_client::DaemonConnection::connect_unix(path, false).await?
                } else if let Some((ref mgr, ref name, ref host, compress)) = remote {
                    mgr.get_or_connect(name, host, compress).await?
                } else {
                    cterm_client::DaemonConnection::connect_local().await?
                };
                let session = conn.create_session(opts).await?;
                Ok(session)
            }),
            Err(e) => Err(cterm_client::ClientError::Connection(e.to_string())),
        };

        let _ = tx.send(result);
    });

    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        match rx.try_recv() {
            Ok(result) => {
                match result {
                    Ok(session) => {
                        let sid = Some(session.session_id().to_string());
                        let daemon_socket = session.socket_path().map(|p| p.to_owned());
                        let cfg = config.borrow();
                        let terminal = TerminalWidget::from_daemon(session, &cfg, &theme);
                        drop(cfg);

                        // Apply background color override from template
                        if let Some(ref bg) = background_color {
                            terminal.set_background_override(Some(bg));
                        }

                        let tab_id = generate_tab_id(&next_tab_id);
                        let page_num =
                            notebook.append_page(terminal.widget(), None::<&gtk4::Widget>);
                        tab_bar.add_tab(tab_id, &title);

                        if let Some(ref c) = color {
                            tab_bar.set_color(tab_id, Some(c));
                        }

                        setup_tab_callbacks(
                            &notebook,
                            &tabs,
                            &config,
                            &tab_bar,
                            &window,
                            &has_bell,
                            &file_manager,
                            &notification_bar,
                            &terminal,
                            tab_id,
                            keep_open,
                        );

                        finalize_new_tab(
                            &notebook,
                            &tabs,
                            &tab_bar,
                            tab_id,
                            page_num,
                            title.clone(),
                            terminal,
                            false,
                            sid,
                            daemon_socket,
                        );

                        // Store color in tab entry and send metadata to daemon
                        if color.is_some() {
                            if let Some(tab) = tabs.borrow_mut().iter_mut().find(|t| t.id == tab_id)
                            {
                                tab.color = color.clone();
                            }
                        }
                        // Persist tab metadata to daemon
                        if let Some(tab) = tabs.borrow().iter().find(|t| t.id == tab_id) {
                            if let Some(ref c) = color {
                                tab.terminal.set_tab_color_on_daemon(c);
                            }
                            if !title.is_empty() {
                                tab.terminal.set_template_name_on_daemon(&title);
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to create daemon session: {}", e);
                    }
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

/// Spawn a mosh-backed tab.
///
/// Launches mosh-server in a background thread, then creates the terminal
/// widget on the main thread when the connection is established.
#[allow(clippy::too_many_arguments)]
fn spawn_mosh_tab(
    notebook: &Notebook,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    next_tab_id: &Rc<RefCell<u64>>,
    config: &Rc<RefCell<Config>>,
    theme: &Theme,
    tab_bar: &TabBar,
    window: &ApplicationWindow,
    has_bell: &Rc<RefCell<bool>>,
    file_manager: &Rc<RefCell<PendingFileManager>>,
    notification_bar: &NotificationBar,
    mosh_config: cterm_mosh::MoshConfig,
    title: String,
    color: Option<String>,
    background_color: Option<String>,
    keep_open: bool,
) {
    let notebook = notebook.clone();
    let tabs = Rc::clone(tabs);
    let next_tab_id = Rc::clone(next_tab_id);
    let config = Rc::clone(config);
    let theme = theme.clone();
    let tab_bar = tab_bar.clone();
    let window = window.clone();
    let has_bell = Rc::clone(has_bell);
    let file_manager = Rc::clone(file_manager);
    let notification_bar = notification_bar.clone();

    let (tx, rx) =
        std::sync::mpsc::channel::<Result<cterm_mosh::MoshSession, cterm_mosh::MoshError>>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();

        let result = match rt {
            Ok(rt) => rt.block_on(async { cterm_mosh::MoshSession::connect(mosh_config).await }),
            Err(e) => Err(cterm_mosh::MoshError::SshFailed(e.to_string())),
        };

        let _ = tx.send(result);
    });

    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        match rx.try_recv() {
            Ok(result) => {
                match result {
                    Ok(session) => {
                        let cfg = config.borrow();
                        let terminal = TerminalWidget::from_mosh(session, &cfg, &theme);
                        drop(cfg);

                        if let Some(ref bg) = background_color {
                            terminal.set_background_override(Some(bg));
                        }

                        let tab_id = generate_tab_id(&next_tab_id);
                        let page_num =
                            notebook.append_page(terminal.widget(), None::<&gtk4::Widget>);
                        tab_bar.add_tab(tab_id, &title);

                        if let Some(ref c) = color {
                            tab_bar.set_color(tab_id, Some(c));
                        }

                        setup_tab_callbacks(
                            &notebook,
                            &tabs,
                            &config,
                            &tab_bar,
                            &window,
                            &has_bell,
                            &file_manager,
                            &notification_bar,
                            &terminal,
                            tab_id,
                            keep_open,
                        );

                        finalize_new_tab(
                            &notebook,
                            &tabs,
                            &tab_bar,
                            tab_id,
                            page_num,
                            title.clone(),
                            terminal,
                            false,
                            None, // no session_id for mosh
                            None, // no daemon_socket for mosh
                        );
                    }
                    Err(e) => {
                        log::error!("Failed to create mosh session: {}", e);
                    }
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

/// Create a new daemon-backed tab by attaching to a session
#[allow(clippy::too_many_arguments)]
fn create_daemon_tab(
    notebook: &Notebook,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    next_tab_id: &Rc<RefCell<u64>>,
    config: &Rc<RefCell<Config>>,
    theme: &Theme,
    tab_bar: &TabBar,
    window: &ApplicationWindow,
    has_bell: &Rc<RefCell<bool>>,
    file_manager: &Rc<RefCell<PendingFileManager>>,
    notification_bar: &NotificationBar,
    session_id: &str,
) {
    let cfg = config.borrow();
    let session_id = session_id.to_string();

    let notebook = notebook.clone();
    let tabs = Rc::clone(tabs);
    let next_tab_id = Rc::clone(next_tab_id);
    let config = Rc::clone(config);
    let theme = theme.clone();
    let tab_bar = tab_bar.clone();
    let window = window.clone();
    let has_bell = Rc::clone(has_bell);
    let file_manager = Rc::clone(file_manager);
    let notification_bar = notification_bar.clone();

    // Calculate terminal dimensions from current allocation
    let cell_dims = calculate_initial_cell_dimensions(&cfg);
    let alloc = notebook.allocation();
    let cols = ((alloc.width() as f64) / cell_dims.width).floor().max(80.0) as u32;
    let rows = ((alloc.height() as f64) / cell_dims.height)
        .floor()
        .max(24.0) as u32;
    drop(cfg);

    // Connect and attach in background thread, then create tab on main thread
    let (tx, rx) = std::sync::mpsc::channel::<DaemonAttachResult>();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();

        let result = match rt {
            Ok(rt) => rt.block_on(async {
                let conn = cterm_client::DaemonConnection::connect_local().await?;
                let (session, _initial_screen) =
                    conn.attach_session(&session_id, cols, rows).await?;
                Ok(session)
            }),
            Err(e) => Err(cterm_client::ClientError::Connection(e.to_string())),
        };

        let _ = tx.send(result);
    });

    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        match rx.try_recv() {
            Ok(result) => {
                match result {
                    Ok(session) => {
                        let sid = session.session_id().to_string();
                        let title = "Terminal".to_string();
                        let cfg = config.borrow();
                        let terminal = TerminalWidget::from_daemon(session, &cfg, &theme);

                        let tab_id = generate_tab_id(&next_tab_id);
                        let page_num =
                            notebook.append_page(terminal.widget(), None::<&gtk4::Widget>);
                        tab_bar.add_tab(tab_id, &title);

                        setup_tab_callbacks(
                            &notebook,
                            &tabs,
                            &config,
                            &tab_bar,
                            &window,
                            &has_bell,
                            &file_manager,
                            &notification_bar,
                            &terminal,
                            tab_id,
                            false,
                        );

                        finalize_new_tab(
                            &notebook,
                            &tabs,
                            &tab_bar,
                            tab_id,
                            page_num,
                            title,
                            terminal,
                            false,
                            Some(sid),
                            None,
                        );
                    }
                    Err(e) => {
                        log::error!("Failed to attach to daemon session: {}", e);
                    }
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        }
    });
}

type DaemonAttachResult =
    std::result::Result<cterm_client::SessionHandle, cterm_client::ClientError>;

/// Create a new terminal tab from a template
#[allow(clippy::too_many_arguments)]
fn create_tab_from_template(
    notebook: &Notebook,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    next_tab_id: &Rc<RefCell<u64>>,
    config: &Rc<RefCell<Config>>,
    theme: &Theme,
    tab_bar: &TabBar,
    window: &ApplicationWindow,
    has_bell: &Rc<RefCell<bool>>,
    file_manager: &Rc<RefCell<PendingFileManager>>,
    notification_bar: &NotificationBar,
    template: &cterm_app::config::StickyTabConfig,
    remote_manager: &cterm_client::RemoteManager,
) {
    // Prepare working directory (clone from git if needed)
    if let Some(ref working_dir) = template.working_directory {
        if let Err(e) =
            cterm_app::prepare_working_directory(working_dir, template.git_remote.as_deref())
        {
            log::error!("Failed to prepare working directory: {}", e);
        }
    }

    let cfg = config.borrow();

    // Build daemon session options from template
    let opts = cterm_client::CreateSessionOpts {
        cols: 80,
        rows: 24,
        shell: template
            .command
            .clone()
            .or_else(|| cfg.general.default_shell.clone()),
        args: if template.args.is_empty() && template.command.is_none() {
            cfg.general.shell_args.clone()
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

    // Resolve remote from template
    let remote_cfg = template
        .remote
        .as_ref()
        .and_then(|name| cfg.remotes.iter().find(|r| r.name == *name).cloned());

    let is_mosh = remote_cfg
        .as_ref()
        .is_some_and(|r| r.method == cterm_app::config::ConnectionMethod::Mosh);

    if is_mosh {
        let remote = remote_cfg.unwrap();
        let mosh_config = mosh_config_from_remote(&remote, 80, 24);
        drop(cfg);

        spawn_mosh_tab(
            notebook,
            tabs,
            next_tab_id,
            config,
            theme,
            tab_bar,
            window,
            has_bell,
            file_manager,
            notification_bar,
            mosh_config,
            template.name.clone(),
            template.color.clone(),
            template.background_color.clone(),
            template.keep_open,
        );
    } else {
        let remote = remote_cfg.map(|r| {
            (
                remote_manager.clone(),
                r.name.clone(),
                r.host.clone(),
                r.ssh_compression,
            )
        });
        drop(cfg);

        spawn_daemon_tab(
            notebook,
            tabs,
            next_tab_id,
            config,
            theme,
            tab_bar,
            window,
            has_bell,
            file_manager,
            notification_bar,
            opts,
            template.name.clone(),
            template.color.clone(),
            template.background_color.clone(),
            template.keep_open,
            remote,
            None,
        );
    }
}

/// Close current tab (with confirmation if process is running)
fn close_current_tab(
    notebook: &Notebook,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    tab_bar: &TabBar,
    window: &ApplicationWindow,
    config: &Rc<RefCell<Config>>,
) {
    if let Some(page_idx) = notebook.current_page() {
        let tab_id = {
            let tabs = tabs.borrow();
            tabs.get(page_idx as usize).map(|t| t.id)
        };
        if let Some(id) = tab_id {
            request_close_tab_by_id(notebook, tabs, tab_bar, window, config, id);
        }
    }
}

/// Close tab by ID (unconditionally - used when process has already exited)
fn close_tab_by_id(
    notebook: &Notebook,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    tab_bar: &TabBar,
    window: &ApplicationWindow,
    id: u64,
) {
    // Find index of this tab
    let index = {
        let tabs = tabs.borrow();
        tabs.iter().position(|t| t.id == id)
    };

    let Some(index) = index else { return };

    // Destroy the daemon session (kill the PTY process)
    {
        let tabs = tabs.borrow();
        tabs[index].terminal.destroy_session();
    }

    // Remove from notebook
    notebook.remove_page(Some(index as u32));

    // Remove from tabs list
    tabs.borrow_mut().remove(index);

    // Remove from tab bar
    tab_bar.remove_tab(id);

    // Update tab bar visibility (hide if only one tab)
    tab_bar.update_visibility();

    // Close window if no tabs left
    if tabs.borrow().is_empty() {
        window.close();
        return;
    }

    // Update active tab in tab bar
    sync_tab_bar_active(tab_bar, tabs, notebook);

    // Focus the current terminal
    if let Some(page) = notebook.current_page() {
        if let Some(widget) = notebook.nth_page(Some(page)) {
            widget.grab_focus();
        }
    }
}

/// Request to close tab by ID - queries daemon for running processes and confirms with user
#[cfg(unix)]
fn request_close_tab_by_id(
    notebook: &Notebook,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    tab_bar: &TabBar,
    window: &ApplicationWindow,
    config: &Rc<RefCell<Config>>,
    id: u64,
) {
    let confirm_close = config.borrow().general.confirm_close_with_running;
    if !confirm_close {
        close_tab_by_id(notebook, tabs, tab_bar, window, id);
        return;
    }

    // Get session_id, daemon_socket, and title for the daemon query
    let query_info: Option<(String, Option<std::path::PathBuf>, String)> = {
        let tabs = tabs.borrow();
        tabs.iter().find(|t| t.id == id).map(|tab| {
            (
                tab.session_id.clone().unwrap_or_default(),
                tab.daemon_socket.clone(),
                tab.title.clone(),
            )
        })
    };

    let Some((session_id, daemon_socket, tab_title)) = query_info else {
        return;
    };

    if session_id.is_empty() {
        close_tab_by_id(notebook, tabs, tab_bar, window, id);
        return;
    }

    // Query the daemon asynchronously for foreground process info
    let notebook = notebook.clone();
    let tabs = Rc::clone(tabs);
    let tab_bar = tab_bar.clone();
    let window = window.clone();
    let (result_tx, result_rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = rt.block_on(async {
            let conn = match if let Some(ref path) = daemon_socket {
                cterm_client::DaemonConnection::connect_unix(path, false).await
            } else {
                cterm_client::DaemonConnection::connect_local().await
            } {
                Ok(c) => c,
                Err(_) => return None,
            };
            let session = match conn.get_session(&session_id).await {
                Ok(s) => s,
                Err(_) => return None,
            };
            if session.has_foreground_process {
                let name = if session.foreground_process_name.is_empty() {
                    "a process".to_string()
                } else {
                    session.foreground_process_name
                };
                Some(name)
            } else {
                None
            }
        });
        let _ = result_tx.send(result);
    });

    // Check result on the main thread via idle callback
    glib::idle_add_local_once(move || {
        // The thread should complete very quickly (local socket query)
        let process_name = match result_rx.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(Some(name)) => name,
            _ => {
                // No foreground process or query failed — close directly
                close_tab_by_id(&notebook, &tabs, &tab_bar, &window, id);
                return;
            }
        };

        let window_for_closure = window.clone();
        let notebook_c = notebook.clone();
        let tabs_c = Rc::clone(&tabs);
        let tab_bar_c = tab_bar.clone();

        dialogs::show_close_confirmation_dialog(
            &window,
            vec![(tab_title, process_name)],
            move |confirmed| {
                if confirmed {
                    close_tab_by_id(&notebook_c, &tabs_c, &tab_bar_c, &window_for_closure, id);
                }
            },
        );
    });
}

/// Request to close tab by ID - non-Unix fallback (no process detection)
#[cfg(not(unix))]
fn request_close_tab_by_id(
    notebook: &Notebook,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    tab_bar: &TabBar,
    window: &ApplicationWindow,
    _config: &Rc<RefCell<Config>>,
    id: u64,
) {
    close_tab_by_id(notebook, tabs, tab_bar, window, id);
}

/// Close all tabs except the current one
fn close_other_tabs(
    notebook: &Notebook,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    tab_bar: &TabBar,
    _window: &ApplicationWindow,
) {
    let current_id = {
        if let Some(page_idx) = notebook.current_page() {
            let tabs = tabs.borrow();
            tabs.get(page_idx as usize).map(|t| t.id)
        } else {
            None
        }
    };

    let Some(current_id) = current_id else { return };

    // Collect IDs of tabs to close (all except current)
    let ids_to_close: Vec<u64> = {
        let tabs = tabs.borrow();
        tabs.iter()
            .filter(|t| t.id != current_id)
            .map(|t| t.id)
            .collect()
    };

    // Close each tab by removing from notebook, tabs list, and tab bar
    for id in ids_to_close {
        // Find index of this tab
        let index = {
            let tabs = tabs.borrow();
            tabs.iter().position(|t| t.id == id)
        };

        if let Some(index) = index {
            notebook.remove_page(Some(index as u32));
            tabs.borrow_mut().remove(index);
            tab_bar.remove_tab(id);
        }
    }

    // Update tab bar visibility (hide if only one tab)
    tab_bar.update_visibility();

    // Update active tab in tab bar
    sync_tab_bar_active(tab_bar, tabs, notebook);
}

/// Sync tab bar active state with notebook
/// Focus the terminal widget in the currently visible notebook page
fn focus_current_terminal(notebook: &Notebook) {
    if let Some(page) = notebook.current_page() {
        if let Some(widget) = notebook.nth_page(Some(page)) {
            widget.grab_focus();
        }
    }
}

fn sync_tab_bar_active(tab_bar: &TabBar, tabs: &Rc<RefCell<Vec<TabEntry>>>, notebook: &Notebook) {
    if let Some(page_idx) = notebook.current_page() {
        let tabs = tabs.borrow();
        if let Some(tab) = tabs.get(page_idx as usize) {
            tab_bar.set_active(tab.id);
            // Clear bell when tab becomes active
            tab_bar.clear_bell(tab.id);
            tab.terminal.clear_alert();
        }
    }
}

/// Convert GTK modifier state to our Modifiers
fn gtk_modifiers_to_modifiers(state: gdk::ModifierType) -> Modifiers {
    let mut modifiers = Modifiers::empty();

    if state.contains(gdk::ModifierType::CONTROL_MASK) {
        modifiers.insert(Modifiers::CTRL);
    }
    if state.contains(gdk::ModifierType::SHIFT_MASK) {
        modifiers.insert(Modifiers::SHIFT);
    }
    if state.contains(gdk::ModifierType::ALT_MASK) {
        modifiers.insert(Modifiers::ALT);
    }
    if state.contains(gdk::ModifierType::SUPER_MASK) {
        modifiers.insert(Modifiers::SUPER);
    }

    modifiers
}

/// Convert GDK keyval to our KeyCode
fn keyval_to_keycode(keyval: gdk::Key) -> Option<KeyCode> {
    use gdk::Key;

    Some(match keyval {
        Key::a | Key::A => KeyCode::A,
        Key::b | Key::B => KeyCode::B,
        Key::c | Key::C => KeyCode::C,
        Key::d | Key::D => KeyCode::D,
        Key::e | Key::E => KeyCode::E,
        Key::f | Key::F => KeyCode::F,
        Key::g | Key::G => KeyCode::G,
        Key::h | Key::H => KeyCode::H,
        Key::i | Key::I => KeyCode::I,
        Key::j | Key::J => KeyCode::J,
        Key::k | Key::K => KeyCode::K,
        Key::l | Key::L => KeyCode::L,
        Key::m | Key::M => KeyCode::M,
        Key::n | Key::N => KeyCode::N,
        Key::o | Key::O => KeyCode::O,
        Key::p | Key::P => KeyCode::P,
        Key::q | Key::Q => KeyCode::Q,
        Key::r | Key::R => KeyCode::R,
        Key::s | Key::S => KeyCode::S,
        Key::t | Key::T => KeyCode::T,
        Key::u | Key::U => KeyCode::U,
        Key::v | Key::V => KeyCode::V,
        Key::w | Key::W => KeyCode::W,
        Key::x | Key::X => KeyCode::X,
        Key::y | Key::Y => KeyCode::Y,
        Key::z | Key::Z => KeyCode::Z,
        Key::_0 => KeyCode::Key0,
        Key::_1 => KeyCode::Key1,
        Key::_2 => KeyCode::Key2,
        Key::_3 => KeyCode::Key3,
        Key::_4 => KeyCode::Key4,
        Key::_5 => KeyCode::Key5,
        Key::_6 => KeyCode::Key6,
        Key::_7 => KeyCode::Key7,
        Key::_8 => KeyCode::Key8,
        Key::_9 => KeyCode::Key9,
        Key::F1 => KeyCode::F1,
        Key::F2 => KeyCode::F2,
        Key::F3 => KeyCode::F3,
        Key::F4 => KeyCode::F4,
        Key::F5 => KeyCode::F5,
        Key::F6 => KeyCode::F6,
        Key::F7 => KeyCode::F7,
        Key::F8 => KeyCode::F8,
        Key::F9 => KeyCode::F9,
        Key::F10 => KeyCode::F10,
        Key::F11 => KeyCode::F11,
        Key::F12 => KeyCode::F12,
        Key::Up => KeyCode::Up,
        Key::Down => KeyCode::Down,
        Key::Left => KeyCode::Left,
        Key::Right => KeyCode::Right,
        Key::Home => KeyCode::Home,
        Key::End => KeyCode::End,
        Key::Page_Up | Key::KP_Page_Up => KeyCode::PageUp,
        Key::Page_Down | Key::KP_Page_Down => KeyCode::PageDown,
        Key::Insert => KeyCode::Insert,
        Key::Delete => KeyCode::Delete,
        Key::BackSpace => KeyCode::Backspace,
        Key::Return | Key::KP_Enter => KeyCode::Enter,
        Key::Tab | Key::ISO_Left_Tab => KeyCode::Tab,
        Key::Escape => KeyCode::Escape,
        Key::space => KeyCode::Space,
        Key::minus => KeyCode::Minus,
        Key::equal => KeyCode::Equals,
        Key::comma => KeyCode::Comma,
        Key::period => KeyCode::Period,
        Key::slash => KeyCode::Slash,
        Key::backslash => KeyCode::Backslash,
        Key::semicolon => KeyCode::Semicolon,
        Key::apostrophe => KeyCode::Quote,
        Key::bracketleft => KeyCode::LeftBracket,
        Key::bracketright => KeyCode::RightBracket,
        Key::grave => KeyCode::Backquote,
        _ => return None,
    })
}

/// Calculate initial cell dimensions for window sizing
/// Uses Pango font metrics to get accurate measurements
fn calculate_initial_cell_dimensions(config: &Config) -> CellDimensions {
    use gtk4::pango;

    let font_family = &config.appearance.font.family;
    let font_size = config.appearance.font.size;

    // Get the default font map and create a context
    let font_map = pangocairo::FontMap::default();
    let context = font_map.create_context();

    // Try the requested font first, then fall back to generic monospace
    let fonts_to_try = [font_family.to_string(), "monospace".to_string()];

    for font_name in &fonts_to_try {
        let font_desc =
            pango::FontDescription::from_string(&format!("{} {}", font_name, font_size));

        if let Some(font) = font_map.load_font(&context, &font_desc) {
            let metrics = font.metrics(None);
            let char_width = metrics.approximate_char_width() as f64 / pango::SCALE as f64;
            let ascent = metrics.ascent() as f64 / pango::SCALE as f64;
            let descent = metrics.descent() as f64 / pango::SCALE as f64;
            let height = ascent + descent;

            if char_width > 0.0 && height > 0.0 {
                return CellDimensions {
                    width: char_width,
                    height: height * 1.1,
                };
            }
        }
    }

    // Last resort: use a Pango layout to measure a character directly
    let layout = pango::Layout::new(&context);
    let font_desc = pango::FontDescription::from_string(&format!("monospace {}", font_size));
    layout.set_font_description(Some(&font_desc));
    layout.set_text("M");

    let (width, height) = layout.pixel_size();
    if width > 0 && height > 0 {
        return CellDimensions {
            width: width as f64,
            height: height as f64 * 1.1,
        };
    }

    panic!(
        "Failed to load any font or measure text. \
         Please ensure fonts are installed (e.g., fonts-dejavu or similar)."
    );
}

/// Build a MoshConfig from a RemoteConfig.
fn mosh_config_from_remote(remote: &RemoteConfig, cols: u16, rows: u16) -> cterm_mosh::MoshConfig {
    let connection_type = match remote.connection_type {
        ConnectionType::Relay => {
            let relay_host = remote
                .proxy_jump
                .as_deref()
                .unwrap_or("unixshells.com")
                .to_string();
            let jump_host = format!("relay.{}", relay_host);
            cterm_mosh::MoshConnectionType::Relay {
                relay_host: relay_host.clone(),
                jump_host,
                relay_username: remote
                    .relay_username
                    .clone()
                    .unwrap_or_else(|| "default".to_string()),
                relay_device: remote
                    .relay_device
                    .clone()
                    .unwrap_or_else(|| "default".to_string()),
                session_name: remote
                    .session_name
                    .clone()
                    .unwrap_or_else(|| "default".to_string()),
            }
        }
        ConnectionType::Direct => cterm_mosh::MoshConnectionType::Direct,
    };
    cterm_mosh::MoshConfig {
        host: remote.host.clone(),
        connection_type,
        proxy_jump: if remote.connection_type == ConnectionType::Direct {
            remote.proxy_jump.clone()
        } else {
            None
        },
        cols,
        rows,
        locale: Some("en_US.UTF-8".to_string()),
        term: Some("xterm-256color".to_string()),
        ssh_args: Vec::new(),
    }
}
