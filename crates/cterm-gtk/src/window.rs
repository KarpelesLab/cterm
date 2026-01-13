//! Main window implementation

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{
    Application, ApplicationWindow, Box as GtkBox, EventControllerKey, Label,
    Notebook, Orientation, Stack, Widget, gdk, gio, glib,
};
use parking_lot::Mutex;

use cterm_app::config::Config;
use cterm_app::session::{TabState, WindowState};
use cterm_app::shortcuts::ShortcutManager;
use cterm_core::pty::PtyError;
use cterm_ui::events::{Action, KeyCode, Modifiers};
use cterm_ui::theme::Theme;

use crate::tab_bar::TabBar;
use crate::terminal_widget::TerminalWidget;

/// Main window container
pub struct CtermWindow {
    pub window: ApplicationWindow,
    pub notebook: Notebook,
    pub tab_bar: TabBar,
    pub config: Config,
    pub theme: Theme,
    pub shortcuts: ShortcutManager,
    pub terminals: Rc<RefCell<Vec<TerminalWidget>>>,
}

impl CtermWindow {
    /// Create a new window
    pub fn new(app: &Application, config: &Config, theme: &Theme) -> Self {
        // Create the main window
        let window = ApplicationWindow::builder()
            .application(app)
            .title("cterm")
            .default_width(800)
            .default_height(600)
            .build();

        // Create the main container
        let main_box = GtkBox::new(Orientation::Vertical, 0);

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

        let cterm_window = Self {
            window: window.clone(),
            notebook: notebook.clone(),
            tab_bar,
            config: config.clone(),
            theme: theme.clone(),
            shortcuts,
            terminals: Rc::new(RefCell::new(Vec::new())),
        };

        // Set up key event handling
        cterm_window.setup_key_handler();

        // Create initial tab
        cterm_window.new_tab();

        // Set up tab bar callbacks
        cterm_window.setup_tab_bar_callbacks();

        cterm_window
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
        let terminals = Rc::clone(&self.terminals);
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
                            create_new_tab(&notebook, &terminals, &config, &theme, &tab_bar);
                            return glib::Propagation::Stop;
                        }
                        Action::CloseTab => {
                            close_current_tab(&notebook, &terminals, &tab_bar);
                            return glib::Propagation::Stop;
                        }
                        Action::NextTab => {
                            let n = notebook.n_pages();
                            if n > 0 {
                                let current = notebook.current_page().unwrap_or(0);
                                notebook.set_current_page(Some((current + 1) % n));
                                update_tab_bar_active(&tab_bar, &notebook);
                            }
                            return glib::Propagation::Stop;
                        }
                        Action::PrevTab => {
                            let n = notebook.n_pages();
                            if n > 0 {
                                let current = notebook.current_page().unwrap_or(0);
                                let prev = if current == 0 { n - 1 } else { current - 1 };
                                notebook.set_current_page(Some(prev));
                                update_tab_bar_active(&tab_bar, &notebook);
                            }
                            return glib::Propagation::Stop;
                        }
                        Action::Tab(n) => {
                            let idx = (*n as u32).saturating_sub(1);
                            if idx < notebook.n_pages() {
                                notebook.set_current_page(Some(idx));
                                update_tab_bar_active(&tab_bar, &notebook);
                            }
                            return glib::Propagation::Stop;
                        }
                        Action::Copy => {
                            // TODO: Copy selection
                            return glib::Propagation::Stop;
                        }
                        Action::Paste => {
                            // TODO: Paste from clipboard
                            return glib::Propagation::Stop;
                        }
                        Action::ZoomIn => {
                            // TODO: Increase font size
                            return glib::Propagation::Stop;
                        }
                        Action::ZoomOut => {
                            // TODO: Decrease font size
                            return glib::Propagation::Stop;
                        }
                        Action::ZoomReset => {
                            // TODO: Reset font size
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

    /// Set up tab bar callbacks
    fn setup_tab_bar_callbacks(&self) {
        let notebook = self.notebook.clone();
        let terminals = Rc::clone(&self.terminals);
        let config = self.config.clone();
        let theme = self.theme.clone();
        let tab_bar = self.tab_bar.clone();

        // New tab button
        self.tab_bar.set_on_new_tab(move || {
            create_new_tab(&notebook, &terminals, &config, &theme, &tab_bar);
        });
    }

    /// Create a new tab
    pub fn new_tab(&self) {
        create_new_tab(
            &self.notebook,
            &self.terminals,
            &self.config,
            &self.theme,
            &self.tab_bar,
        );
    }
}

/// Create a new terminal tab
fn create_new_tab(
    notebook: &Notebook,
    terminals: &Rc<RefCell<Vec<TerminalWidget>>>,
    config: &Config,
    theme: &Theme,
    tab_bar: &TabBar,
) {
    // Create terminal widget
    let terminal = match TerminalWidget::new(config, theme) {
        Ok(t) => t,
        Err(e) => {
            log::error!("Failed to create terminal: {}", e);
            return;
        }
    };

    // Add to notebook
    let page_num = notebook.append_page(terminal.widget(), None::<&Widget>);

    // Add to tab bar
    let tab_id = terminals.borrow().len() as u64;
    tab_bar.add_tab(tab_id, "Terminal");

    // Set up close callback
    let notebook_close = notebook.clone();
    let terminals_close = Rc::clone(terminals);
    let tab_bar_close = tab_bar.clone();
    tab_bar.set_on_close(tab_id, move || {
        close_tab_by_id(&notebook_close, &terminals_close, &tab_bar_close, tab_id);
    });

    // Set up click callback
    let notebook_click = notebook.clone();
    let terminals_click = Rc::clone(terminals);
    let tab_bar_click = tab_bar.clone();
    tab_bar.set_on_click(tab_id, move || {
        // Find the page index for this tab
        let terminals = terminals_click.borrow();
        if let Some(idx) = terminals.iter().position(|_| true) {
            notebook_click.set_current_page(Some(idx as u32));
            update_tab_bar_active(&tab_bar_click, &notebook_click);
        }
    });

    // Store terminal
    terminals.borrow_mut().push(terminal);

    // Switch to new tab
    notebook.set_current_page(Some(page_num));
    update_tab_bar_active(tab_bar, notebook);
}

/// Close current tab
fn close_current_tab(
    notebook: &Notebook,
    terminals: &Rc<RefCell<Vec<TerminalWidget>>>,
    tab_bar: &TabBar,
) {
    if let Some(page) = notebook.current_page() {
        close_tab(notebook, terminals, tab_bar, page as usize);
    }
}

/// Close tab by index
fn close_tab(
    notebook: &Notebook,
    terminals: &Rc<RefCell<Vec<TerminalWidget>>>,
    tab_bar: &TabBar,
    index: usize,
) {
    let n_pages = notebook.n_pages() as usize;
    if n_pages <= 1 || index >= n_pages {
        return;
    }

    // Remove from notebook
    notebook.remove_page(Some(index as u32));

    // Remove from terminals list
    terminals.borrow_mut().remove(index);

    // Remove from tab bar
    tab_bar.remove_tab(index as u64);

    update_tab_bar_active(tab_bar, notebook);
}

/// Close tab by ID
fn close_tab_by_id(
    notebook: &Notebook,
    terminals: &Rc<RefCell<Vec<TerminalWidget>>>,
    tab_bar: &TabBar,
    _id: u64,
) {
    // For now, find by position (in a real impl we'd track IDs properly)
    if let Some(page) = notebook.current_page() {
        close_tab(notebook, terminals, tab_bar, page as usize);
    }
}

/// Update tab bar active state
fn update_tab_bar_active(tab_bar: &TabBar, notebook: &Notebook) {
    if let Some(page) = notebook.current_page() {
        tab_bar.set_active(page as u64);
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
