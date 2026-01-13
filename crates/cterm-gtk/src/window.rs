//! Main window implementation

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    gdk, gio, glib, Application, ApplicationWindow, Box as GtkBox, EventControllerKey, Notebook,
    Orientation, PopoverMenuBar,
};

use cterm_app::config::Config;
use cterm_app::shortcuts::ShortcutManager;
use cterm_ui::events::{Action, KeyCode, Modifiers};
use cterm_ui::theme::Theme;

use crate::dialogs;
use crate::menu;
use crate::tab_bar::TabBar;
use crate::terminal_widget::{CellDimensions, TerminalWidget};

/// Tab entry tracking terminal and its ID
struct TabEntry {
    id: u64,
    title: String,
    terminal: TerminalWidget,
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
    debug_menu_shown: Rc<RefCell<bool>>,
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
        let menu_model = menu::create_menu_model();
        let menu_bar = PopoverMenuBar::from_model(Some(&menu_model));
        main_box.append(&menu_bar);

        // Create tab bar
        let tab_bar = TabBar::new();
        main_box.append(tab_bar.widget());

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

        let debug_menu_shown = Rc::new(RefCell::new(false));

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
            debug_menu_shown,
        };

        // Set up window actions
        cterm_window.setup_actions();

        // Set up key event handling
        cterm_window.setup_key_handler();

        // Set up Shift key tracking for debug menu
        cterm_window.setup_debug_menu_handler();

        // Create initial tab
        cterm_window.new_tab();

        // Set up tab bar callbacks
        cterm_window.setup_tab_bar_callbacks();

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

        // File menu actions
        {
            let notebook = notebook.clone();
            let tabs = Rc::clone(&tabs);
            let next_tab_id = Rc::clone(&next_tab_id);
            let config = Rc::clone(&config);
            let theme = theme.clone();
            let tab_bar = tab_bar.clone();
            let window_clone = window.clone();
            let action = gio::SimpleAction::new("new-tab", None);
            action.connect_activate(move |_, _| {
                create_new_tab(
                    &notebook,
                    &tabs,
                    &next_tab_id,
                    &config,
                    &theme,
                    &tab_bar,
                    &window_clone,
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
            let action = gio::SimpleAction::new("close-tab", None);
            action.connect_activate(move |_, _| {
                close_current_tab(&notebook, &tabs, &tab_bar, &window_clone);
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

        // Edit menu actions
        {
            // Copy - placeholder until selection is implemented
            let action = gio::SimpleAction::new("copy", None);
            action.connect_activate(|_, _| {
                log::info!("Copy action triggered - selection not yet implemented");
            });
            window.add_action(&action);
        }

        {
            // Copy as HTML - placeholder
            let action = gio::SimpleAction::new("copy-html", None);
            action.connect_activate(|_, _| {
                log::info!("Copy as HTML action triggered - not yet implemented");
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
            // Select All - placeholder
            let action = gio::SimpleAction::new("select-all", None);
            action.connect_activate(|_, _| {
                log::info!("Select All action triggered - selection not yet implemented");
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
                let current_title = {
                    if let Some(page_idx) = notebook.current_page() {
                        let tabs = tabs.borrow();
                        tabs.get(page_idx as usize)
                            .map(|t| t.title.clone())
                            .unwrap_or_default()
                    } else {
                        String::new()
                    }
                };
                let tabs_clone = Rc::clone(&tabs);
                let notebook_clone = notebook.clone();
                let tab_bar_clone = tab_bar.clone();
                dialogs::show_set_title_dialog(&window_clone, &current_title, move |new_title| {
                    if let Some(page_idx) = notebook_clone.current_page() {
                        let mut tabs = tabs_clone.borrow_mut();
                        if let Some(tab) = tabs.get_mut(page_idx as usize) {
                            tab.title = new_title.clone();
                            tab_bar_clone.set_title(tab.id, &new_title);
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
            let tab_bar = tab_bar.clone();
            let action = gio::SimpleAction::new("set-color", None);
            action.connect_activate(move |_, _| {
                let tabs_clone = Rc::clone(&tabs);
                let notebook_clone = notebook.clone();
                let tab_bar_clone = tab_bar.clone();
                dialogs::show_set_color_dialog(&window_clone, move |color| {
                    if let Some(page_idx) = notebook_clone.current_page() {
                        let tabs = tabs_clone.borrow();
                        if let Some(tab) = tabs.get(page_idx as usize) {
                            tab_bar_clone.set_color(tab.id, color.as_deref());
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
                    log::info!("Set encoding: {}", encoding);
                    // TODO: Implement encoding change
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

        // Help menu actions
        {
            let window_clone = window.clone();
            let config = Rc::clone(&config);
            let action = gio::SimpleAction::new("preferences", None);
            action.connect_activate(move |_, _| {
                let cfg = config.borrow().clone();
                let config_for_save = Rc::clone(&config);
                dialogs::show_preferences_dialog(&window_clone, &cfg, move |new_config| {
                    log::info!("Preferences saved");
                    // Save to disk
                    if let Err(e) = cterm_app::config::save_config(&new_config) {
                        log::error!("Failed to save config: {}", e);
                    } else {
                        log::info!("Configuration saved to disk");
                    }
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
            let action = gio::SimpleAction::new(
                "execute-upgrade",
                Some(&glib::VariantType::new("s").unwrap()),
            );
            action.connect_activate(move |_, param| {
                if let Some(binary_path) = param.and_then(|p| p.get::<String>()) {
                    log::info!("Executing seamless upgrade with binary: {}", binary_path);

                    // Get the current tabs for state collection
                    let tabs_borrowed = tabs.borrow();
                    let tab_count = tabs_borrowed.len();

                    log::info!(
                        "Upgrade would preserve {} tabs (not yet fully implemented)",
                        tab_count
                    );

                    // TODO: Full implementation requires:
                    // 1. Stop all PTY readers
                    // 2. Collect upgrade state from all windows/tabs
                    // 3. Create Unix socket
                    // 4. Spawn new process with --upgrade-receiver
                    // 5. Send state and FDs via SCM_RIGHTS
                    // 6. Wait for ack and exit

                    // For now, show a dialog explaining the limitation
                    let dialog = gtk4::MessageDialog::new(
                        None::<&gtk4::Window>,
                        gtk4::DialogFlags::MODAL,
                        gtk4::MessageType::Info,
                        gtk4::ButtonsType::Ok,
                        format!(
                            "Seamless upgrade is ready to be implemented.\n\n\
                            Binary: {}\n\
                            Tabs to preserve: {}\n\n\
                            Full implementation pending.",
                            binary_path, tab_count
                        ),
                    );
                    dialog.connect_response(|d, _| d.close());
                    dialog.present();
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

        // Debug menu actions (hidden unless Shift is held when opening Help menu)
        {
            // Re-launch cterm - triggers seamless upgrade to the same binary (for testing)
            let tabs = Rc::clone(&tabs);
            let window_clone = window.clone();
            let action = gio::SimpleAction::new("debug-relaunch", None);
            action.connect_activate(move |_, _| {
                log::info!("Debug: Re-launching cterm for seamless upgrade test");

                // Get current executable path
                let current_exe = match std::env::current_exe() {
                    Ok(path) => path,
                    Err(e) => {
                        log::error!("Failed to get current executable path: {}", e);
                        return;
                    }
                };

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

    /// Present the window
    pub fn present(&self) {
        self.window.present();
    }

    /// Set up keyboard event handler
    fn setup_key_handler(&self) {
        let key_controller = EventControllerKey::new();

        let shortcuts = self.shortcuts.clone();
        let notebook = self.notebook.clone();
        let tabs = Rc::clone(&self.tabs);
        let next_tab_id = Rc::clone(&self.next_tab_id);
        let window = self.window.clone();
        let config = self.config.clone();
        let theme = self.theme.clone();
        let tab_bar = self.tab_bar.clone();

        key_controller.connect_key_pressed(move |_, keyval, _keycode, state| {
            // Convert GTK modifiers to our modifiers
            let modifiers = gtk_modifiers_to_modifiers(state);

            // Convert keyval to our key code
            if let Some(key) = keyval_to_keycode(keyval) {
                // Check for shortcut match
                if let Some(action) = shortcuts.match_event(key, modifiers) {
                    match action {
                        Action::NewTab => {
                            create_new_tab(
                                &notebook,
                                &tabs,
                                &next_tab_id,
                                &config,
                                &theme,
                                &tab_bar,
                                &window,
                            );
                            return glib::Propagation::Stop;
                        }
                        Action::CloseTab => {
                            close_current_tab(&notebook, &tabs, &tab_bar, &window);
                            return glib::Propagation::Stop;
                        }
                        Action::NextTab => {
                            let n = notebook.n_pages();
                            if n > 0 {
                                let current = notebook.current_page().unwrap_or(0);
                                notebook.set_current_page(Some((current + 1) % n));
                                sync_tab_bar_active(&tab_bar, &tabs, &notebook);
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
                            }
                            return glib::Propagation::Stop;
                        }
                        Action::Tab(n) => {
                            let idx = (*n as u32).saturating_sub(1);
                            if idx < notebook.n_pages() {
                                notebook.set_current_page(Some(idx));
                                sync_tab_bar_active(&tab_bar, &tabs, &notebook);
                            }
                            return glib::Propagation::Stop;
                        }
                        Action::Copy => {
                            // TODO: Copy selection (requires text selection implementation)
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
                        Action::CloseWindow => {
                            window.close();
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

    /// Set up Shift key tracking for debug menu visibility
    fn setup_debug_menu_handler(&self) {
        let debug_controller = EventControllerKey::new();

        let menu_bar = self.menu_bar.clone();
        let debug_menu_shown = Rc::clone(&self.debug_menu_shown);

        // Track Shift key for debug menu
        debug_controller.connect_key_pressed(move |_, keyval, _keycode, _state| {
            // Check if Shift was pressed
            if keyval == gdk::Key::Shift_L || keyval == gdk::Key::Shift_R {
                let mut shown = debug_menu_shown.borrow_mut();
                if !*shown {
                    *shown = true;
                    let new_menu = menu::create_menu_model_with_options(true);
                    menu_bar.set_menu_model(Some(&new_menu));
                }
            }
            glib::Propagation::Proceed
        });

        let menu_bar = self.menu_bar.clone();
        let debug_menu_shown = Rc::clone(&self.debug_menu_shown);

        debug_controller.connect_key_released(move |_, keyval, _keycode, _state| {
            // Check if Shift was released
            if keyval == gdk::Key::Shift_L || keyval == gdk::Key::Shift_R {
                let mut shown = debug_menu_shown.borrow_mut();
                if *shown {
                    *shown = false;
                    let new_menu = menu::create_menu_model_with_options(false);
                    menu_bar.set_menu_model(Some(&new_menu));
                }
            }
        });

        self.window.add_controller(debug_controller);
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

        // New tab button
        self.tab_bar.set_on_new_tab(move || {
            create_new_tab(
                &notebook,
                &tabs,
                &next_tab_id,
                &config,
                &theme,
                &tab_bar,
                &window,
            );
        });
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
        );
    }
}

/// Create a new terminal tab
fn create_new_tab(
    notebook: &Notebook,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    next_tab_id: &Rc<RefCell<u64>>,
    config: &Rc<RefCell<Config>>,
    theme: &Theme,
    tab_bar: &TabBar,
    window: &ApplicationWindow,
) {
    // Create terminal widget
    let cfg = config.borrow();
    let terminal = match TerminalWidget::new(&cfg, theme) {
        Ok(t) => t,
        Err(e) => {
            log::error!("Failed to create terminal: {}", e);
            return;
        }
    };

    // Generate unique tab ID
    let tab_id = {
        let mut id = next_tab_id.borrow_mut();
        let current = *id;
        *id += 1;
        current
    };

    // Add to notebook
    let page_num = notebook.append_page(terminal.widget(), None::<&gtk4::Widget>);

    // Add to tab bar
    tab_bar.add_tab(tab_id, "Terminal");

    // Set up close callback
    let notebook_close = notebook.clone();
    let tabs_close = Rc::clone(tabs);
    let tab_bar_close = tab_bar.clone();
    let window_close = window.clone();
    tab_bar.set_on_close(tab_id, move || {
        close_tab_by_id(
            &notebook_close,
            &tabs_close,
            &tab_bar_close,
            &window_close,
            tab_id,
        );
    });

    // Set up click callback
    let notebook_click = notebook.clone();
    let tabs_click = Rc::clone(tabs);
    let tab_bar_click = tab_bar.clone();
    tab_bar.set_on_click(tab_id, move || {
        // Find the page index for this tab ID
        let tabs = tabs_click.borrow();
        if let Some(idx) = tabs.iter().position(|t| t.id == tab_id) {
            notebook_click.set_current_page(Some(idx as u32));
            tab_bar_click.set_active(tab_id);
            // Clear bell when tab becomes active
            tab_bar_click.clear_bell(tab_id);
        }
    });

    // Set up terminal exit callback to close tab when process exits
    let notebook_exit = notebook.clone();
    let tabs_exit = Rc::clone(tabs);
    let tab_bar_exit = tab_bar.clone();
    let window_exit = window.clone();
    terminal.set_on_exit(move || {
        close_tab_by_id(
            &notebook_exit,
            &tabs_exit,
            &tab_bar_exit,
            &window_exit,
            tab_id,
        );
    });

    // Set up bell callback to show bell icon on inactive tabs
    let tab_bar_bell = tab_bar.clone();
    let notebook_bell = notebook.clone();
    let tabs_bell = Rc::clone(tabs);
    terminal.set_on_bell(move || {
        // Only show bell if this tab is not active
        if let Some(current_page) = notebook_bell.current_page() {
            let tabs = tabs_bell.borrow();
            if let Some(current_tab) = tabs.get(current_page as usize) {
                if current_tab.id != tab_id {
                    // This tab is not active, show bell
                    tab_bar_bell.set_bell(tab_id, true);
                }
            }
        }
    });

    // Store terminal with its ID
    tabs.borrow_mut().push(TabEntry {
        id: tab_id,
        title: "Terminal".to_string(),
        terminal,
    });

    // Switch to new tab and focus terminal
    notebook.set_current_page(Some(page_num));
    tab_bar.set_active(tab_id);

    // Focus the terminal widget
    if let Some(widget) = notebook.nth_page(Some(page_num)) {
        widget.grab_focus();
    }
}

/// Close current tab
fn close_current_tab(
    notebook: &Notebook,
    tabs: &Rc<RefCell<Vec<TabEntry>>>,
    tab_bar: &TabBar,
    window: &ApplicationWindow,
) {
    if let Some(page_idx) = notebook.current_page() {
        let tab_id = {
            let tabs = tabs.borrow();
            tabs.get(page_idx as usize).map(|t| t.id)
        };
        if let Some(id) = tab_id {
            close_tab_by_id(notebook, tabs, tab_bar, window, id);
        }
    }
}

/// Close tab by ID
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

    // Remove from notebook
    notebook.remove_page(Some(index as u32));

    // Remove from tabs list
    tabs.borrow_mut().remove(index);

    // Remove from tab bar
    tab_bar.remove_tab(id);

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

    // Update active tab in tab bar
    sync_tab_bar_active(tab_bar, tabs, notebook);
}

/// Sync tab bar active state with notebook
fn sync_tab_bar_active(tab_bar: &TabBar, tabs: &Rc<RefCell<Vec<TabEntry>>>, notebook: &Notebook) {
    if let Some(page_idx) = notebook.current_page() {
        let tabs = tabs.borrow();
        if let Some(tab) = tabs.get(page_idx as usize) {
            tab_bar.set_active(tab.id);
            // Clear bell when tab becomes active
            tab_bar.clear_bell(tab.id);
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
        Key::Page_Up => KeyCode::PageUp,
        Key::Page_Down => KeyCode::PageDown,
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
