//! Terminal rendering widget using Cairo

use std::cell::RefCell;
use std::io::Read;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{
    DrawingArea, EventControllerKey, EventControllerScroll,
    GestureClick, ScrolledWindow, Widget, gdk, glib, pango,
};
use parking_lot::Mutex;

use cterm_app::config::Config;
use cterm_core::color::{Color, ColorPalette, Rgb};
use cterm_core::cell::CellAttrs;
use cterm_core::parser::Parser;
use cterm_core::pty::{Pty, PtyConfig, PtyError};
use cterm_core::screen::{CursorStyle, Screen, ScreenConfig};
use cterm_core::term::{Key, Modifiers, Terminal};
use cterm_ui::theme::Theme;

/// Terminal widget wrapping GTK drawing area
pub struct TerminalWidget {
    container: ScrolledWindow,
    drawing_area: DrawingArea,
    terminal: Arc<Mutex<Terminal>>,
    theme: Theme,
    font_family: String,
    font_size: f64,
    cell_width: f64,
    cell_height: f64,
}

impl TerminalWidget {
    /// Create a new terminal widget
    pub fn new(config: &Config, theme: &Theme) -> Result<Self, PtyError> {
        // Create drawing area
        let drawing_area = DrawingArea::new();
        drawing_area.set_can_focus(true);
        drawing_area.set_focusable(true);
        drawing_area.add_css_class("terminal");

        // Create scrolled window
        let container = ScrolledWindow::builder()
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .hexpand(true)
            .child(&drawing_area)
            .build();

        // Get font settings
        let font_family = config.appearance.font.family.clone();
        let font_size = config.appearance.font.size;

        // Calculate cell dimensions (will be updated on first draw)
        let cell_width = font_size * 0.6;
        let cell_height = font_size * 1.2;

        // Calculate initial terminal size
        let cols = 80;
        let rows = 24;

        // Create terminal
        let screen_config = ScreenConfig {
            scrollback_lines: config.general.scrollback_lines,
        };

        let pty_config = PtyConfig {
            shell: config.general.default_shell.clone(),
            args: config.general.shell_args.clone(),
            cwd: config.general.working_directory.clone(),
            env: config
                .general
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            ..Default::default()
        };

        let terminal = Terminal::with_shell(cols, rows, screen_config, &pty_config)?;
        let terminal = Arc::new(Mutex::new(terminal));

        let widget = Self {
            container,
            drawing_area: drawing_area.clone(),
            terminal: Arc::clone(&terminal),
            theme: theme.clone(),
            font_family,
            font_size,
            cell_width,
            cell_height,
        };

        // Set up drawing
        widget.setup_drawing();

        // Set up input handling
        widget.setup_input();

        // Set up PTY reading
        widget.setup_pty_reader();

        // Set up resize handling
        widget.setup_resize();

        Ok(widget)
    }

    /// Get the widget for adding to containers
    pub fn widget(&self) -> &ScrolledWindow {
        &self.container
    }

    /// Set up the draw function
    fn setup_drawing(&self) {
        let terminal = Arc::clone(&self.terminal);
        let theme = self.theme.clone();
        let font_family = self.font_family.clone();
        let font_size = self.font_size;

        self.drawing_area.set_draw_func(move |area, cr, width, height| {
            draw_terminal(
                cr,
                width as f64,
                height as f64,
                &terminal,
                &theme,
                &font_family,
                font_size,
            );
        });
    }

    /// Set up input handling
    fn setup_input(&self) {
        let terminal = Arc::clone(&self.terminal);
        let drawing_area = self.drawing_area.clone();

        // Keyboard input
        let key_controller = EventControllerKey::new();
        let terminal_key = Arc::clone(&terminal);

        key_controller.connect_key_pressed(move |_, keyval, _keycode, state| {
            let modifiers = gtk_state_to_modifiers(state);
            let has_ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let has_alt = state.contains(gdk::ModifierType::ALT_MASK);
            let _has_shift = state.contains(gdk::ModifierType::SHIFT_MASK);

            // Handle special keys (arrows, function keys, etc.)
            if let Some(key) = keyval_to_key(keyval) {
                let term = terminal_key.lock();
                if let Some(bytes) = term.handle_key(key, modifiers) {
                    if let Err(e) = term.write(&bytes) {
                        log::error!("Failed to write to PTY: {}", e);
                    }
                }
                return glib::Propagation::Stop;
            }

            // Get the character for this key
            if let Some(c) = keyval.to_unicode() {
                let term = terminal_key.lock();

                // Handle Ctrl+letter -> control character
                if has_ctrl && !has_alt {
                    let ctrl_char = match c.to_ascii_lowercase() {
                        'a'..='z' => Some((c.to_ascii_lowercase() as u8 - b'a' + 1) as u8),
                        '[' | '3' => Some(0x1b), // Escape
                        '\\' | '4' => Some(0x1c),
                        ']' | '5' => Some(0x1d),
                        '^' | '6' => Some(0x1e),
                        '_' | '7' | '/' => Some(0x1f),
                        ' ' | '2' | '@' => Some(0x00), // Ctrl-Space/Ctrl-@
                        '?' | '8' => Some(0x7f), // DEL
                        _ => None,
                    };

                    if let Some(byte) = ctrl_char {
                        if let Err(e) = term.write(&[byte]) {
                            log::error!("Failed to write to PTY: {}", e);
                        }
                        return glib::Propagation::Stop;
                    }
                }

                // Handle Alt+key -> ESC + key
                if has_alt && !has_ctrl {
                    let mut buf = vec![0x1b]; // ESC
                    let mut char_buf = [0u8; 4];
                    let s = c.encode_utf8(&mut char_buf);
                    buf.extend_from_slice(s.as_bytes());
                    if let Err(e) = term.write(&buf) {
                        log::error!("Failed to write to PTY: {}", e);
                    }
                    return glib::Propagation::Stop;
                }

                // Regular character (no Ctrl/Alt)
                if !has_ctrl && !has_alt {
                    let mut buf = [0u8; 4];
                    let s = c.encode_utf8(&mut buf);
                    if let Err(e) = term.write(s.as_bytes()) {
                        log::error!("Failed to write to PTY: {}", e);
                    }
                    return glib::Propagation::Stop;
                }
            }

            glib::Propagation::Proceed
        });

        self.drawing_area.add_controller(key_controller);

        // Mouse click for focus
        let click_controller = GestureClick::new();
        click_controller.connect_pressed(move |_, _, _, _| {
            drawing_area.grab_focus();
        });
        self.drawing_area.add_controller(click_controller);

        // Scroll handling
        let scroll_controller = EventControllerScroll::new(
            gtk4::EventControllerScrollFlags::VERTICAL,
        );
        let terminal_scroll = Arc::clone(&terminal);
        let drawing_area_scroll = self.drawing_area.clone();

        scroll_controller.connect_scroll(move |_, _dx, dy| {
            let mut term = terminal_scroll.lock();
            if dy < 0.0 {
                term.scroll_viewport_up(3);
            } else {
                term.scroll_viewport_down(3);
            }
            drawing_area_scroll.queue_draw();
            glib::Propagation::Stop
        });

        self.drawing_area.add_controller(scroll_controller);
    }

    /// Set up PTY reader
    fn setup_pty_reader(&self) {
        let terminal = Arc::clone(&self.terminal);
        let drawing_area = self.drawing_area.clone();

        // Spawn a thread to read from PTY using glib's spawn_future_local
        let (tx, rx) = std::sync::mpsc::channel::<PtyMessage>();

        std::thread::spawn(move || {
            let mut buf = vec![0u8; 4096];
            loop {
                let term = terminal.lock();
                if let Some(pty) = term.pty() {
                    drop(term); // Release lock before blocking read

                    let terminal_clone = Arc::clone(&terminal);
                    let reader = {
                        let term = terminal_clone.lock();
                        term.pty().map(|p| p.clone_reader())
                    };

                    if let Some(reader) = reader {
                        let mut reader = reader.lock();
                        match reader.read(&mut buf) {
                            Ok(0) => {
                                // EOF - process exited
                                let _ = tx.send(PtyMessage::Exited);
                                break;
                            }
                            Ok(n) => {
                                let data = buf[..n].to_vec();
                                if tx.send(PtyMessage::Data(data)).is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                log::error!("PTY read error: {}", e);
                                let _ = tx.send(PtyMessage::Exited);
                                break;
                            }
                        }
                    } else {
                        break;
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });

        // Handle messages on main thread using glib timeout
        let terminal_main = Arc::clone(&self.terminal);
        glib::timeout_add_local(Duration::from_millis(10), move || {
            // Process all pending messages
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    PtyMessage::Data(data) => {
                        let mut term = terminal_main.lock();
                        term.process(&data);
                        term.screen_mut().dirty = false;
                        drawing_area.queue_draw();
                    }
                    PtyMessage::Exited => {
                        log::info!("Terminal process exited");
                        // TODO: Handle process exit (close tab or show message)
                        return glib::ControlFlow::Break;
                    }
                }
            }
            glib::ControlFlow::Continue
        });
    }

    /// Set up resize handling
    fn setup_resize(&self) {
        let terminal = Arc::clone(&self.terminal);
        let font_size = self.font_size;

        self.drawing_area.connect_resize(move |area, width, height| {
            // Calculate cell dimensions using Pango
            let cell_width = font_size * 0.6;
            let cell_height = font_size * 1.4;

            let cols = ((width as f64) / cell_width).floor() as usize;
            let rows = ((height as f64) / cell_height).floor() as usize;

            if cols > 0 && rows > 0 {
                let mut term = terminal.lock();
                term.resize(cols, rows);
            }
        });
    }
}

/// Messages from PTY reader thread
enum PtyMessage {
    Data(Vec<u8>),
    Exited,
}

/// Draw the terminal contents
fn draw_terminal(
    cr: &cairo::Context,
    width: f64,
    height: f64,
    terminal: &Arc<Mutex<Terminal>>,
    theme: &Theme,
    font_family: &str,
    font_size: f64,
) {
    let term = terminal.lock();
    let screen = term.screen();
    let palette = &theme.colors;

    // Draw background
    let (r, g, b) = palette.background.to_f64();
    cr.set_source_rgb(r, g, b);
    cr.paint().ok();

    // Create Pango layout for text measurement and rendering
    let pango_context = pangocairo::functions::create_context(cr);
    let layout = pango::Layout::new(&pango_context);

    // Set font
    let font_desc = pango::FontDescription::from_string(&format!("{} {}", font_family, font_size));
    layout.set_font_description(Some(&font_desc));

    // Measure cell size
    layout.set_text("M");
    let (char_width, char_height) = layout.pixel_size();
    let cell_width = char_width as f64;
    let cell_height = char_height as f64 * 1.2;

    // Draw cells
    let grid = screen.grid();
    let scroll_offset = screen.scroll_offset;

    for row_idx in 0..grid.height() {
        if let Some(row) = grid.row(row_idx) {
            let y = row_idx as f64 * cell_height;

            for col_idx in 0..grid.width() {
                let cell = &row[col_idx];
                let x = col_idx as f64 * cell_width;

                // Skip wide char spacers
                if cell.attrs.contains(CellAttrs::WIDE_SPACER) {
                    continue;
                }

                // Draw background if not default
                if cell.bg != Color::Default || cell.attrs.contains(CellAttrs::INVERSE) {
                    let bg_color = if cell.attrs.contains(CellAttrs::INVERSE) {
                        cell.fg.to_rgb(palette)
                    } else {
                        cell.bg.to_rgb(palette)
                    };

                    let (r, g, b) = bg_color.to_f64();
                    cr.set_source_rgb(r, g, b);

                    let char_width = if cell.attrs.contains(CellAttrs::WIDE) {
                        cell_width * 2.0
                    } else {
                        cell_width
                    };

                    cr.rectangle(x, y, char_width, cell_height);
                    cr.fill().ok();
                }

                // Draw character
                if cell.c != ' ' {
                    let fg_color = if cell.attrs.contains(CellAttrs::INVERSE) {
                        cell.bg.to_rgb(palette)
                    } else if cell.fg == Color::Default {
                        palette.foreground
                    } else {
                        cell.fg.to_rgb(palette)
                    };

                    // Apply dim
                    let fg_color = if cell.attrs.contains(CellAttrs::DIM) {
                        Rgb::new(
                            (fg_color.r as f64 * 0.5) as u8,
                            (fg_color.g as f64 * 0.5) as u8,
                            (fg_color.b as f64 * 0.5) as u8,
                        )
                    } else {
                        fg_color
                    };

                    let (r, g, b) = fg_color.to_f64();
                    cr.set_source_rgb(r, g, b);

                    // Apply text attributes to font
                    let mut attrs = pango::AttrList::new();

                    if cell.attrs.contains(CellAttrs::BOLD) {
                        let attr = pango::AttrInt::new_weight(pango::Weight::Bold);
                        attrs.insert(attr);
                    }

                    if cell.attrs.contains(CellAttrs::ITALIC) {
                        let attr = pango::AttrInt::new_style(pango::Style::Italic);
                        attrs.insert(attr);
                    }

                    if cell.attrs.contains(CellAttrs::UNDERLINE) {
                        let attr = pango::AttrInt::new_underline(pango::Underline::Single);
                        attrs.insert(attr);
                    }

                    if cell.attrs.contains(CellAttrs::STRIKETHROUGH) {
                        let attr = pango::AttrInt::new_strikethrough(true);
                        attrs.insert(attr);
                    }

                    layout.set_attributes(Some(&attrs));
                    layout.set_text(&cell.c.to_string());

                    cr.move_to(x, y);
                    pangocairo::functions::show_layout(cr, &layout);

                    // Reset attributes
                    layout.set_attributes(None::<&pango::AttrList>);
                }
            }
        }
    }

    // Draw cursor
    if screen.modes.show_cursor && scroll_offset == 0 {
        let cursor = &screen.cursor;
        let x = cursor.col as f64 * cell_width;
        let y = cursor.row as f64 * cell_height;

        let (r, g, b) = theme.cursor.color.to_f64();
        cr.set_source_rgb(r, g, b);

        match cursor.style {
            CursorStyle::Block => {
                cr.rectangle(x, y, cell_width, cell_height);
                cr.fill().ok();

                // Draw character under cursor with inverted color
                if let Some(cell) = screen.get_cell(cursor.row, cursor.col) {
                    if cell.c != ' ' {
                        let (r, g, b) = theme.cursor.text_color.to_f64();
                        cr.set_source_rgb(r, g, b);
                        layout.set_text(&cell.c.to_string());
                        cr.move_to(x, y);
                        pangocairo::functions::show_layout(cr, &layout);
                    }
                }
            }
            CursorStyle::Underline => {
                cr.rectangle(x, y + cell_height - 2.0, cell_width, 2.0);
                cr.fill().ok();
            }
            CursorStyle::Bar => {
                cr.rectangle(x, y, 2.0, cell_height);
                cr.fill().ok();
            }
        }
    }
}

/// Convert GTK modifier state to our Modifiers
fn gtk_state_to_modifiers(state: gdk::ModifierType) -> Modifiers {
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

/// Convert GDK keyval to terminal Key
fn keyval_to_key(keyval: gdk::Key) -> Option<Key> {
    use gdk::Key as GK;

    Some(match keyval {
        GK::Up => Key::Up,
        GK::Down => Key::Down,
        GK::Left => Key::Left,
        GK::Right => Key::Right,
        GK::Home => Key::Home,
        GK::End => Key::End,
        GK::Page_Up => Key::PageUp,
        GK::Page_Down => Key::PageDown,
        GK::Insert => Key::Insert,
        GK::Delete => Key::Delete,
        GK::BackSpace => Key::Backspace,
        GK::Return | GK::KP_Enter => Key::Enter,
        GK::Tab | GK::ISO_Left_Tab => Key::Tab,
        GK::Escape => Key::Escape,
        GK::F1 => Key::F(1),
        GK::F2 => Key::F(2),
        GK::F3 => Key::F(3),
        GK::F4 => Key::F(4),
        GK::F5 => Key::F(5),
        GK::F6 => Key::F(6),
        GK::F7 => Key::F(7),
        GK::F8 => Key::F(8),
        GK::F9 => Key::F(9),
        GK::F10 => Key::F(10),
        GK::F11 => Key::F(11),
        GK::F12 => Key::F(12),
        _ => return None,
    })
}
