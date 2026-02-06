//! Terminal rendering widget using Cairo

use std::cell::RefCell;
use std::io::Read;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gtk4::prelude::*;
use gtk4::{
    gdk, gio, glib, pango, DrawingArea, EventControllerKey, EventControllerScroll, GestureClick,
};
use parking_lot::Mutex;

use cterm_app::config::{Config, StickyTabConfig};
use cterm_app::upgrade::TerminalUpgradeState;
use cterm_core::cell::CellAttrs;
use cterm_core::color::{Color, Rgb};
use cterm_core::pty::{PtyConfig, PtyError};
use cterm_core::screen::{ClipboardOperation, CursorStyle, ScreenConfig};
use cterm_core::term::{Key, Modifiers, Terminal, TerminalEvent};
use cterm_ui::theme::Theme;

/// Cell dimensions calculated from font metrics
#[derive(Debug, Clone, Copy)]
pub struct CellDimensions {
    pub width: f64,
    pub height: f64,
}

/// Callback type for terminal events
type EventCallback = Rc<RefCell<Option<Box<dyn Fn()>>>>;
/// Callback type for title change events
type TitleCallback = Rc<RefCell<Option<Box<dyn Fn(&str)>>>>;
/// Callback type for file transfer events
type FileTransferCallback = Rc<RefCell<Option<Box<dyn Fn(cterm_core::FileTransferOperation)>>>>;

/// Terminal widget wrapping GTK drawing area
pub struct TerminalWidget {
    drawing_area: DrawingArea,
    terminal: Arc<Mutex<Terminal>>,
    theme: Theme,
    font_family: String,
    font_size: Rc<RefCell<f64>>,
    default_font_size: f64,
    cell_dims: Rc<RefCell<CellDimensions>>,
    /// Optional background color override (from template)
    background_override: Rc<RefCell<Option<cterm_core::color::Rgb>>>,
    on_exit: EventCallback,
    on_bell: EventCallback,
    on_title_change: TitleCallback,
    on_file_transfer: FileTransferCallback,
}

impl TerminalWidget {
    /// Create a new terminal widget
    pub fn new(config: &Config, theme: &Theme) -> Result<Self, PtyError> {
        // Get font settings
        let font_family = config.appearance.font.family.clone();
        let font_size = config.appearance.font.size;

        // Calculate cell dimensions using Pango font metrics
        let cell_dims = calculate_cell_dimensions(&font_family, font_size);

        // Create drawing area with proper sizing
        let drawing_area = DrawingArea::new();
        drawing_area.set_can_focus(true);
        drawing_area.set_focusable(true);
        drawing_area.add_css_class("terminal");
        drawing_area.set_vexpand(true);
        drawing_area.set_hexpand(true);

        // Set minimum size for 80x24 characters
        let min_width = (cell_dims.width * 80.0).ceil() as i32;
        let min_height = (cell_dims.height * 24.0).ceil() as i32;
        drawing_area.set_size_request(min_width, min_height);

        // Calculate initial terminal size (80x24 minimum)
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
            term: config.general.term.clone(),
            ..Default::default()
        };

        let terminal = Terminal::with_shell(cols, rows, screen_config, &pty_config)?;
        let terminal = Arc::new(Mutex::new(terminal));

        let cell_dims = Rc::new(RefCell::new(cell_dims));

        let widget = Self {
            drawing_area: drawing_area.clone(),
            terminal: Arc::clone(&terminal),
            theme: theme.clone(),
            font_family,
            font_size: Rc::new(RefCell::new(font_size)),
            default_font_size: font_size,
            cell_dims,
            background_override: Rc::new(RefCell::new(None)),
            on_exit: Rc::new(RefCell::new(None)),
            on_bell: Rc::new(RefCell::new(None)),
            on_title_change: Rc::new(RefCell::new(None)),
            on_file_transfer: Rc::new(RefCell::new(None)),
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

    /// Create a new terminal widget with a specified working directory
    pub fn new_with_cwd(
        config: &Config,
        theme: &Theme,
        cwd: Option<String>,
    ) -> Result<Self, PtyError> {
        // Get font settings
        let font_family = config.appearance.font.family.clone();
        let font_size = config.appearance.font.size;

        // Calculate cell dimensions using Pango font metrics
        let cell_dims = calculate_cell_dimensions(&font_family, font_size);

        // Create drawing area with proper sizing
        let drawing_area = DrawingArea::new();
        drawing_area.set_can_focus(true);
        drawing_area.set_focusable(true);
        drawing_area.add_css_class("terminal");
        drawing_area.set_vexpand(true);
        drawing_area.set_hexpand(true);

        // Set minimum size for 80x24 characters
        let min_width = (cell_dims.width * 80.0).ceil() as i32;
        let min_height = (cell_dims.height * 24.0).ceil() as i32;
        drawing_area.set_size_request(min_width, min_height);

        // Calculate initial terminal size (80x24 minimum)
        let cols = 80;
        let rows = 24;

        // Create terminal
        let screen_config = ScreenConfig {
            scrollback_lines: config.general.scrollback_lines,
        };

        // Use provided cwd or fall back to config setting
        let effective_cwd = cwd
            .map(std::path::PathBuf::from)
            .or_else(|| config.general.working_directory.clone());

        let pty_config = PtyConfig {
            shell: config.general.default_shell.clone(),
            args: config.general.shell_args.clone(),
            cwd: effective_cwd,
            env: config
                .general
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            term: config.general.term.clone(),
            ..Default::default()
        };

        let terminal = Terminal::with_shell(cols, rows, screen_config, &pty_config)?;
        let terminal = Arc::new(Mutex::new(terminal));

        let cell_dims = Rc::new(RefCell::new(cell_dims));

        let widget = Self {
            drawing_area: drawing_area.clone(),
            terminal: Arc::clone(&terminal),
            theme: theme.clone(),
            font_family,
            font_size: Rc::new(RefCell::new(font_size)),
            default_font_size: font_size,
            cell_dims,
            background_override: Rc::new(RefCell::new(None)),
            on_exit: Rc::new(RefCell::new(None)),
            on_bell: Rc::new(RefCell::new(None)),
            on_title_change: Rc::new(RefCell::new(None)),
            on_file_transfer: Rc::new(RefCell::new(None)),
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

    /// Create a terminal widget from a sticky tab template
    pub fn from_template(
        config: &Config,
        theme: &Theme,
        template: &StickyTabConfig,
    ) -> Result<Self, PtyError> {
        // Get font settings
        let font_family = config.appearance.font.family.clone();
        let font_size = config.appearance.font.size;

        // Calculate cell dimensions using Pango font metrics
        let cell_dims = calculate_cell_dimensions(&font_family, font_size);

        // Create drawing area with proper sizing
        let drawing_area = DrawingArea::new();
        drawing_area.set_can_focus(true);
        drawing_area.set_focusable(true);
        drawing_area.add_css_class("terminal");
        drawing_area.set_vexpand(true);
        drawing_area.set_hexpand(true);

        // Set minimum size for 80x24 characters
        let min_width = (cell_dims.width * 80.0).ceil() as i32;
        let min_height = (cell_dims.height * 24.0).ceil() as i32;
        drawing_area.set_size_request(min_width, min_height);

        // Calculate initial terminal size (80x24 minimum)
        let cols = 80;
        let rows = 24;

        // Create terminal with template settings
        let screen_config = ScreenConfig {
            scrollback_lines: config.general.scrollback_lines,
        };

        // Get command and args from template
        let (command, args) = template.get_command_args();

        let pty_config = PtyConfig {
            shell: command,
            args,
            cwd: template.working_directory.clone(),
            env: template
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            term: config.general.term.clone(),
            ..Default::default()
        };

        let terminal = Terminal::with_shell(cols, rows, screen_config, &pty_config)?;
        let terminal = Arc::new(Mutex::new(terminal));

        let cell_dims = Rc::new(RefCell::new(cell_dims));

        let widget = Self {
            drawing_area: drawing_area.clone(),
            terminal: Arc::clone(&terminal),
            theme: theme.clone(),
            font_family,
            font_size: Rc::new(RefCell::new(font_size)),
            default_font_size: font_size,
            cell_dims,
            background_override: Rc::new(RefCell::new(None)),
            on_exit: Rc::new(RefCell::new(None)),
            on_bell: Rc::new(RefCell::new(None)),
            on_title_change: Rc::new(RefCell::new(None)),
            on_file_transfer: Rc::new(RefCell::new(None)),
        };

        // Set up drawing
        widget.setup_drawing();

        // Set up input handling
        widget.setup_input();

        // Set up PTY reading
        widget.setup_pty_reader();

        // Set up resize handling
        widget.setup_resize();

        // Apply background color override from template
        if let Some(ref color) = template.background_color {
            widget.set_background_override(Some(color));
        }

        Ok(widget)
    }

    /// Create a terminal widget from restored state (for seamless upgrades)
    ///
    /// This takes a pre-existing terminal with restored screen state and PTY,
    /// and sets up the GTK widget around it.
    #[cfg(unix)]
    pub fn from_restored(terminal: Terminal, config: &Config, theme: &Theme) -> Self {
        // Get font settings
        let font_family = config.appearance.font.family.clone();
        let font_size = config.appearance.font.size;

        // Calculate cell dimensions using Pango font metrics
        let cell_dims = calculate_cell_dimensions(&font_family, font_size);

        // Create drawing area with proper sizing
        let drawing_area = DrawingArea::new();
        drawing_area.set_can_focus(true);
        drawing_area.set_focusable(true);
        drawing_area.add_css_class("terminal");
        drawing_area.set_vexpand(true);
        drawing_area.set_hexpand(true);

        // Set minimum size for 80x24 characters
        let min_width = (cell_dims.width * 80.0).ceil() as i32;
        let min_height = (cell_dims.height * 24.0).ceil() as i32;
        drawing_area.set_size_request(min_width, min_height);

        let terminal = Arc::new(Mutex::new(terminal));
        let cell_dims = Rc::new(RefCell::new(cell_dims));

        let widget = Self {
            drawing_area: drawing_area.clone(),
            terminal: Arc::clone(&terminal),
            theme: theme.clone(),
            font_family,
            font_size: Rc::new(RefCell::new(font_size)),
            default_font_size: font_size,
            cell_dims,
            background_override: Rc::new(RefCell::new(None)),
            on_exit: Rc::new(RefCell::new(None)),
            on_bell: Rc::new(RefCell::new(None)),
            on_title_change: Rc::new(RefCell::new(None)),
            on_file_transfer: Rc::new(RefCell::new(None)),
        };

        // Set up drawing
        widget.setup_drawing();

        // Set up input handling
        widget.setup_input();

        // Set up PTY reading (the restored PTY should already be running)
        widget.setup_pty_reader();

        // Set up resize handling
        widget.setup_resize();

        widget
    }

    /// Export terminal state for seamless upgrade
    #[cfg(unix)]
    pub fn export_state(&self) -> TerminalUpgradeState {
        let term = self.terminal.lock();
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

    /// Get the widget for adding to containers
    pub fn widget(&self) -> &DrawingArea {
        &self.drawing_area
    }

    /// Get the current cell dimensions
    #[allow(dead_code)]
    pub fn cell_dimensions(&self) -> CellDimensions {
        *self.cell_dims.borrow()
    }

    /// Set callback for when the terminal process exits
    pub fn set_on_exit<F: Fn() + 'static>(&self, callback: F) {
        *self.on_exit.borrow_mut() = Some(Box::new(callback));
    }

    /// Set callback for when the terminal rings the bell
    pub fn set_on_bell<F: Fn() + 'static>(&self, callback: F) {
        *self.on_bell.borrow_mut() = Some(Box::new(callback));
    }

    /// Set callback for when the terminal title changes
    pub fn set_on_title_change<F: Fn(&str) + 'static>(&self, callback: F) {
        *self.on_title_change.borrow_mut() = Some(Box::new(callback));
    }

    /// Set callback for when a file is received
    pub fn set_on_file_transfer<F: Fn(cterm_core::FileTransferOperation) + 'static>(
        &self,
        callback: F,
    ) {
        *self.on_file_transfer.borrow_mut() = Some(Box::new(callback));
    }

    /// Get the terminal for file transfer operations
    pub fn terminal(&self) -> &Arc<Mutex<Terminal>> {
        &self.terminal
    }

    /// Get the current working directory of the foreground process (if any)
    #[cfg(unix)]
    pub fn foreground_cwd(&self) -> Option<String> {
        self.terminal
            .lock()
            .foreground_cwd()
            .map(|p| p.to_string_lossy().into_owned())
    }

    /// Check if there's a foreground process running (other than the shell)
    #[cfg(unix)]
    pub fn has_foreground_process(&self) -> bool {
        self.terminal.lock().has_foreground_process()
    }

    /// Get the name of the foreground process (if any)
    #[cfg(unix)]
    pub fn foreground_process_name(&self) -> Option<String> {
        self.terminal.lock().foreground_process_name()
    }

    /// Write a string to the terminal (for paste operations)
    pub fn write_str(&self, s: &str) {
        let mut term = self.terminal.lock();
        if let Err(e) = term.write_str(s) {
            log::error!("Failed to write to terminal: {}", e);
        }
    }

    /// Set an optional background color override (hex string like "#1a1b26")
    pub fn set_background_override(&self, color: Option<&str>) {
        let rgb = color.and_then(|hex| {
            let hex = hex.trim_start_matches('#');
            if hex.len() == 6 {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(cterm_core::color::Rgb::new(r, g, b))
            } else {
                None
            }
        });
        *self.background_override.borrow_mut() = rgb;
        // Trigger redraw to apply new background
        self.drawing_area.queue_draw();
    }

    /// Increase font size (zoom in)
    pub fn zoom_in(&self) {
        let mut font_size = self.font_size.borrow_mut();
        *font_size = (*font_size + 1.0).min(72.0);
        let new_size = *font_size;
        drop(font_size);
        self.update_cell_dimensions(new_size);
        self.trigger_resize();
    }

    /// Decrease font size (zoom out)
    pub fn zoom_out(&self) {
        let mut font_size = self.font_size.borrow_mut();
        *font_size = (*font_size - 1.0).max(6.0);
        let new_size = *font_size;
        drop(font_size);
        self.update_cell_dimensions(new_size);
        self.trigger_resize();
    }

    /// Reset font size to default
    pub fn zoom_reset(&self) {
        *self.font_size.borrow_mut() = self.default_font_size;
        self.update_cell_dimensions(self.default_font_size);
        self.trigger_resize();
    }

    /// Update cell dimensions after font size change
    fn update_cell_dimensions(&self, font_size: f64) {
        let new_dims = calculate_cell_dimensions(&self.font_family, font_size);
        *self.cell_dims.borrow_mut() = new_dims;
    }

    /// Reset the terminal (soft reset - keeps scrollback)
    pub fn reset(&self) {
        let mut term = self.terminal.lock();
        let screen = term.screen_mut();
        // Soft reset: reset modes and cursor but keep scrollback
        screen.cursor = cterm_core::screen::Cursor::default();
        screen.style = cterm_core::cell::CellStyle::default();
        screen.modes = cterm_core::screen::TerminalModes {
            auto_wrap: true,
            show_cursor: true,
            ..Default::default()
        };
        screen.reset_scroll_region();
        screen.dirty = true;
        drop(term);
        self.drawing_area.queue_draw();
    }

    /// Clear scrollback buffer and fully reset the terminal
    pub fn clear_scrollback_and_reset(&self) {
        let mut term = self.terminal.lock();
        term.screen_mut().reset();
        drop(term);
        self.drawing_area.queue_draw();
    }

    /// Send a signal to the terminal process
    pub fn send_signal(&self, signal: i32) {
        let term = self.terminal.lock();
        if let Err(e) = term.send_signal(signal) {
            log::error!("Failed to send signal {}: {}", signal, e);
        }
    }

    /// Send focus event to terminal if focus events mode is enabled (DECSET 1004)
    /// `focused`: true for focus in (\x1b[I), false for focus out (\x1b[O)
    pub fn send_focus_event(&self, focused: bool) {
        let mut term = self.terminal.lock();
        if term.screen().modes.focus_events {
            let sequence = if focused { b"\x1b[I" } else { b"\x1b[O" };
            if let Err(e) = term.write(sequence) {
                log::error!("Failed to send focus event: {}", e);
            }
        }
    }

    /// Search for text in terminal buffer (scrollback + visible)
    ///
    /// Returns the number of matches found. If matches are found, scrolls to the first match.
    pub fn find(&self, pattern: &str, case_sensitive: bool, regex: bool) -> usize {
        let term = self.terminal.lock();
        let results = term.find(pattern, case_sensitive, regex);
        let count = results.len();

        if let Some(first) = results.first() {
            // Need to release the lock before we can take mutable lock
            let line_idx = first.line;
            drop(term);

            let mut term = self.terminal.lock();
            term.scroll_to_line(line_idx);
            self.drawing_area.queue_draw();
        }

        count
    }

    /// Search and return all matches (for iteration/highlighting)
    #[allow(dead_code)]
    pub fn find_all(
        &self,
        pattern: &str,
        case_sensitive: bool,
        regex: bool,
    ) -> Vec<cterm_core::SearchResult> {
        let term = self.terminal.lock();
        term.find(pattern, case_sensitive, regex)
    }

    /// Scroll to a specific search result
    #[allow(dead_code)]
    pub fn scroll_to_result(&self, result: &cterm_core::SearchResult) {
        let mut term = self.terminal.lock();
        term.scroll_to_line(result.line);
        drop(term);
        self.drawing_area.queue_draw();
    }

    /// Convert pixel coordinates to cell (row, col) coordinates
    ///
    /// Returns (visible_row, col) where visible_row is the row on screen (0 = top)
    #[allow(dead_code)]
    pub fn pixel_to_cell(&self, x: f64, y: f64) -> (usize, usize) {
        let dims = self.cell_dims.borrow();
        let col = (x / dims.width).floor() as usize;
        let row = (y / dims.height).floor() as usize;
        (row, col)
    }

    /// Convert pixel coordinates to absolute line index
    ///
    /// Returns (absolute_line, col) where absolute_line accounts for scrollback
    #[allow(dead_code)]
    pub fn pixel_to_absolute(&self, x: f64, y: f64) -> (usize, usize) {
        let (visible_row, col) = self.pixel_to_cell(x, y);
        let term = self.terminal.lock();
        let absolute_line = term.screen().visible_row_to_absolute_line(visible_row);
        (absolute_line, col)
    }

    /// Start a new selection at the given pixel coordinates
    #[allow(dead_code)]
    pub fn start_selection(&self, x: f64, y: f64) {
        let (line, col) = self.pixel_to_absolute(x, y);
        let mut term = self.terminal.lock();
        term.screen_mut()
            .start_selection(line, col, cterm_core::SelectionMode::Char);
        drop(term);
        self.drawing_area.queue_draw();
    }

    /// Extend the current selection to the given pixel coordinates
    #[allow(dead_code)]
    pub fn extend_selection(&self, x: f64, y: f64) {
        let (line, col) = self.pixel_to_absolute(x, y);
        let mut term = self.terminal.lock();
        term.screen_mut().extend_selection(line, col);
        drop(term);
        self.drawing_area.queue_draw();
    }

    /// Clear the current selection
    #[allow(dead_code)]
    pub fn clear_selection(&self) {
        let mut term = self.terminal.lock();
        term.screen_mut().clear_selection();
        drop(term);
        self.drawing_area.queue_draw();
    }

    /// Get the selected text (if any)
    pub fn get_selected_text(&self) -> Option<String> {
        let term = self.terminal.lock();
        term.screen().get_selected_text()
    }

    /// Copy the current selection to clipboard
    pub fn copy_selection(&self) {
        if let Some(text) = self.get_selected_text() {
            if let Some(display) = gdk::Display::default() {
                let clipboard = display.clipboard();
                clipboard.set_text(&text);
            }
        }
    }

    /// Get the selected text as HTML (if any)
    pub fn get_selected_html(&self) -> Option<String> {
        let term = self.terminal.lock();
        term.screen().get_selected_html(&self.theme.colors)
    }

    /// Copy the current selection to clipboard as HTML
    pub fn copy_selection_html(&self) {
        let term = self.terminal.lock();
        let html = term.screen().get_selected_html(&self.theme.colors);
        let text = term.screen().get_selected_text();
        drop(term);

        if let (Some(html), Some(_text)) = (html, text) {
            if let Some(display) = gdk::Display::default() {
                let clipboard = display.clipboard();
                // GTK4 clipboard can hold multiple formats via ContentProvider
                // For simplicity, we set HTML as text - most apps will interpret it
                // A full implementation would use ContentProvider with multiple MIME types
                clipboard.set_text(&html);
                log::debug!("Copied {} chars as HTML to clipboard", html.len());
            }
        }
    }

    /// Select all text in the terminal
    pub fn select_all(&self) {
        let mut term = self.terminal.lock();
        let total_lines = term.screen().total_lines();
        let width = term.screen().width();

        // Select from the first line to the last line
        term.screen_mut()
            .start_selection(0, 0, cterm_core::screen::SelectionMode::Char);
        term.screen_mut()
            .extend_selection(total_lines.saturating_sub(1), width.saturating_sub(1));
        drop(term);

        self.drawing_area.queue_draw();
    }

    /// Copy the current selection to primary selection (Unix only)
    #[cfg(unix)]
    #[allow(dead_code)]
    pub fn copy_selection_to_primary(&self) {
        if let Some(text) = self.get_selected_text() {
            if let Some(display) = gdk::Display::default() {
                let primary = display.primary_clipboard();
                primary.set_text(&text);
            }
        }
    }

    /// Paste from primary selection (Unix middle-click paste)
    #[cfg(unix)]
    #[allow(dead_code)]
    pub fn paste_primary(&self) {
        let Some(display) = gdk::Display::default() else {
            return;
        };
        let primary = display.primary_clipboard();
        let terminal = Arc::clone(&self.terminal);
        let drawing_area = self.drawing_area.clone();

        primary.read_text_async(None::<&gio::Cancellable>, move |result| {
            if let Ok(Some(text)) = result {
                let mut term = terminal.lock();
                // Use bracketed paste if enabled
                let paste_text = if term.screen().modes.bracketed_paste {
                    format!("\x1b[200~{}\x1b[201~", text)
                } else {
                    text.to_string()
                };
                let _ = term.write_str(&paste_text);
                drawing_area.queue_draw();
            }
        });
    }

    /// Trigger a resize to recalculate terminal dimensions
    fn trigger_resize(&self) {
        // Force a resize by getting current size
        let width = self.drawing_area.width();
        let height = self.drawing_area.height();

        let dims = self.cell_dims.borrow();
        let cols = ((width as f64) / dims.width).floor() as usize;
        let rows = ((height as f64) / dims.height).floor() as usize;
        drop(dims);

        if cols > 0 && rows > 0 {
            let mut term = self.terminal.lock();
            term.resize(cols, rows);
        }

        self.drawing_area.queue_draw();
    }

    /// Set up the draw function
    fn setup_drawing(&self) {
        let terminal = Arc::clone(&self.terminal);
        let theme = self.theme.clone();
        let font_family = self.font_family.clone();
        let font_size = Rc::clone(&self.font_size);
        let cell_dims = Rc::clone(&self.cell_dims);
        let background_override = Rc::clone(&self.background_override);

        self.drawing_area
            .set_draw_func(move |_area, cr, _width, _height| {
                let font_size = *font_size.borrow();
                let dims = *cell_dims.borrow();
                let bg_override = *background_override.borrow();
                draw_terminal(
                    cr,
                    &terminal,
                    &theme,
                    &font_family,
                    font_size,
                    dims,
                    bg_override,
                );
            });
    }

    /// Set up input handling
    fn setup_input(&self) {
        let terminal = Arc::clone(&self.terminal);
        let cell_dims = Rc::clone(&self.cell_dims);

        // Keyboard input
        let key_controller = EventControllerKey::new();
        let terminal_key = Arc::clone(&terminal);

        key_controller.connect_key_pressed(move |_, keyval, _keycode, state| {
            let modifiers = gtk_state_to_modifiers(state);
            let has_ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let has_alt = state.contains(gdk::ModifierType::ALT_MASK);
            let has_shift = state.contains(gdk::ModifierType::SHIFT_MASK);

            // Let Ctrl+Shift combinations pass through to window shortcuts
            if has_ctrl && has_shift {
                return glib::Propagation::Proceed;
            }

            // Handle special keys (arrows, function keys, etc.)
            if let Some(key) = keyval_to_key(keyval) {
                let mut term = terminal_key.lock();
                if let Some(bytes) = term.handle_key(key, modifiers) {
                    if let Err(e) = term.write(&bytes) {
                        log::error!("Failed to write to PTY: {}", e);
                    }
                }
                return glib::Propagation::Stop;
            }

            // Get the character for this key
            if let Some(c) = keyval.to_unicode() {
                // Handle Ctrl+letter -> control character
                if has_ctrl && !has_alt {
                    let mut term = terminal_key.lock();
                    let ctrl_char = match c.to_ascii_lowercase() {
                        'a'..='z' => Some(c.to_ascii_lowercase() as u8 - b'a' + 1),
                        '[' | '3' => Some(0x1b), // Escape
                        '\\' | '4' => Some(0x1c),
                        ']' | '5' => Some(0x1d),
                        '^' | '6' => Some(0x1e),
                        '_' | '7' | '/' => Some(0x1f),
                        ' ' | '2' | '@' => Some(0x00), // Ctrl-Space/Ctrl-@
                        '?' | '8' => Some(0x7f),       // DEL
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
                    let mut term = terminal_key.lock();
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
                    let mut term = terminal_key.lock();
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

        // Selection state: tracks whether we're in a drag operation
        let selecting = Rc::new(RefCell::new(false));

        // Mouse click for selection
        let click_controller = GestureClick::new();
        click_controller.set_button(gdk::BUTTON_PRIMARY);

        let terminal_click = Arc::clone(&terminal);
        let cell_dims_click = Rc::clone(&cell_dims);
        let drawing_area_click = self.drawing_area.clone();
        let selecting_pressed = Rc::clone(&selecting);

        click_controller.connect_pressed(move |_, n_press, x, y| {
            drawing_area_click.grab_focus();

            // Determine selection mode based on click count
            let mode = match n_press {
                2 => cterm_core::SelectionMode::Word,
                3 => cterm_core::SelectionMode::Line,
                _ => cterm_core::SelectionMode::Char,
            };

            // Start selection
            let dims = cell_dims_click.borrow();
            let col = (x / dims.width).floor() as usize;
            let row = (y / dims.height).floor() as usize;
            drop(dims);

            let mut term = terminal_click.lock();
            let line = term.screen().visible_row_to_absolute_line(row);
            term.screen_mut().start_selection(line, col, mode);
            drop(term);

            *selecting_pressed.borrow_mut() = true;
            drawing_area_click.queue_draw();
        });

        let terminal_released = Arc::clone(&terminal);
        let drawing_area_released = self.drawing_area.clone();
        let selecting_released = Rc::clone(&selecting);

        click_controller.connect_released(move |_, _n_press, _x, _y| {
            *selecting_released.borrow_mut() = false;

            // Check if selection is empty (same start and end) and clear it
            let term = terminal_released.lock();
            if let Some(selection) = &term.screen().selection {
                if selection.anchor == selection.end {
                    drop(term);
                    let mut term = terminal_released.lock();
                    term.screen_mut().clear_selection();
                    drawing_area_released.queue_draw();
                } else {
                    // Copy selection to primary clipboard (Unix behavior)
                    #[cfg(unix)]
                    if let Some(text) = term.screen().get_selected_text() {
                        if let Some(display) = gdk::Display::default() {
                            let primary = display.primary_clipboard();
                            primary.set_text(&text);
                        }
                    }
                }
            }
        });

        self.drawing_area.add_controller(click_controller);

        // Middle-click paste from primary selection (Unix only)
        #[cfg(unix)]
        {
            let middle_click_controller = GestureClick::new();
            middle_click_controller.set_button(gdk::BUTTON_MIDDLE);

            let terminal_middle = Arc::clone(&terminal);
            let drawing_area_middle = self.drawing_area.clone();

            middle_click_controller.connect_pressed(move |_, _n_press, _x, _y| {
                let Some(display) = gdk::Display::default() else {
                    return;
                };
                let primary = display.primary_clipboard();
                let terminal = Arc::clone(&terminal_middle);
                let drawing_area = drawing_area_middle.clone();

                primary.read_text_async(None::<&gio::Cancellable>, move |result| {
                    if let Ok(Some(text)) = result {
                        let mut term = terminal.lock();
                        // Use bracketed paste if enabled
                        let paste_text = if term.screen().modes.bracketed_paste {
                            format!("\x1b[200~{}\x1b[201~", text)
                        } else {
                            text.to_string()
                        };
                        let _ = term.write_str(&paste_text);
                        drawing_area.queue_draw();
                    }
                });
            });

            self.drawing_area.add_controller(middle_click_controller);
        }

        // Mouse motion for drag selection
        let motion_controller = gtk4::EventControllerMotion::new();

        let terminal_motion = Arc::clone(&terminal);
        let cell_dims_motion = Rc::clone(&cell_dims);
        let drawing_area_motion = self.drawing_area.clone();
        let selecting_motion = Rc::clone(&selecting);

        motion_controller.connect_motion(move |_, x, y| {
            if !*selecting_motion.borrow() {
                return;
            }

            let dims = cell_dims_motion.borrow();
            let col = (x / dims.width).floor() as usize;
            let row = (y / dims.height).floor() as usize;
            drop(dims);

            let mut term = terminal_motion.lock();
            let line = term.screen().visible_row_to_absolute_line(row);
            term.screen_mut().extend_selection(line, col);
            drop(term);

            drawing_area_motion.queue_draw();
        });

        self.drawing_area.add_controller(motion_controller);

        // Scroll handling
        let scroll_controller =
            EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
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

        // Spawn a thread to read from PTY
        let (tx, rx) = std::sync::mpsc::channel::<PtyMessage>();

        std::thread::spawn(move || {
            let mut buf = vec![0u8; 4096];

            // Get a reader for the PTY
            let reader = {
                let term = terminal.lock();
                term.pty_reader()
            };

            let Some(mut reader) = reader else {
                log::warn!("No PTY reader available");
                return;
            };

            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
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
            }
        });

        // Handle messages on main thread using glib timeout
        let terminal_main = Arc::clone(&self.terminal);
        let on_exit = Rc::clone(&self.on_exit);
        let on_bell = Rc::clone(&self.on_bell);
        let on_title_change = Rc::clone(&self.on_title_change);
        let on_file_transfer = Rc::clone(&self.on_file_transfer);
        glib::timeout_add_local(Duration::from_millis(10), move || {
            // Process all pending messages
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    PtyMessage::Data(data) => {
                        let mut term = terminal_main.lock();
                        let events = term.process(&data);

                        // Handle terminal events
                        for event in events {
                            match event {
                                TerminalEvent::ClipboardRequest(op) => {
                                    // Get clipboard from display
                                    if let Some(display) = gdk::Display::default() {
                                        let clipboard = display.clipboard();
                                        match op {
                                            ClipboardOperation::Set { selection: _, data } => {
                                                // Set clipboard content
                                                if let Ok(text) = String::from_utf8(data) {
                                                    clipboard.set_text(&text);
                                                    log::debug!(
                                                        "Set clipboard via OSC 52: {} bytes",
                                                        text.len()
                                                    );
                                                }
                                            }
                                            ClipboardOperation::Query { selection } => {
                                                // Query clipboard and send response asynchronously
                                                log::debug!("Clipboard query via OSC 52");
                                                let terminal_clip = Arc::clone(&terminal_main);
                                                let sel = selection;
                                                clipboard.read_text_async(
                                                    None::<&gio::Cancellable>,
                                                    move |result| {
                                                        let text = result
                                                            .ok()
                                                            .flatten()
                                                            .map(|s| s.to_string())
                                                            .unwrap_or_default();
                                                        let mut term = terminal_clip.lock();
                                                        let _ = term.send_clipboard_response(
                                                            sel,
                                                            text.as_bytes(),
                                                        );
                                                        log::debug!(
                                                            "Sent clipboard response: {} bytes",
                                                            text.len()
                                                        );
                                                    },
                                                );
                                            }
                                        }
                                    }
                                }
                                TerminalEvent::Bell => {
                                    if let Some(ref callback) = *on_bell.borrow() {
                                        callback();
                                    }
                                }
                                TerminalEvent::TitleChanged(ref title) => {
                                    if let Some(ref callback) = *on_title_change.borrow() {
                                        callback(title);
                                    }
                                }
                                TerminalEvent::ContentChanged => {
                                    // We always redraw below
                                }
                                TerminalEvent::ProcessExited(_) => {
                                    // Handled by PtyMessage::Exited
                                }
                            }
                        }

                        // Check for bell (legacy, might be redundant now)
                        if term.screen().bell {
                            term.screen_mut().bell = false;
                            if let Some(ref callback) = *on_bell.borrow() {
                                callback();
                            }
                        }

                        // Check for file transfers
                        let transfers = term.screen_mut().take_file_transfers();
                        drop(term); // Release lock before callbacks

                        for transfer in transfers {
                            if let Some(ref callback) = *on_file_transfer.borrow() {
                                callback(transfer);
                            }
                        }

                        // Re-acquire lock to clear dirty flag
                        terminal_main.lock().screen_mut().dirty = false;
                        drawing_area.queue_draw();
                    }
                    PtyMessage::Exited => {
                        log::info!("Terminal process exited");
                        // Call exit callback to close tab
                        if let Some(ref callback) = *on_exit.borrow() {
                            callback();
                        }
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
        let cell_dims = Rc::clone(&self.cell_dims);

        self.drawing_area
            .connect_resize(move |_area, width, height| {
                let dims = cell_dims.borrow();
                let cols = ((width as f64) / dims.width).floor() as usize;
                let rows = ((height as f64) / dims.height).floor() as usize;
                drop(dims);

                if cols > 0 && rows > 0 {
                    let mut term = terminal.lock();
                    term.resize(cols, rows);
                }
            });
    }
}

/// Calculate cell dimensions using Pango font metrics
fn calculate_cell_dimensions(font_family: &str, font_size: f64) -> CellDimensions {
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
            // Use the approximate char width for monospace fonts
            let char_width = metrics.approximate_char_width() as f64 / pango::SCALE as f64;
            // Height is ascent + descent with some line spacing
            let ascent = metrics.ascent() as f64 / pango::SCALE as f64;
            let descent = metrics.descent() as f64 / pango::SCALE as f64;
            let height = ascent + descent;

            // Validate that we got sensible metrics
            if char_width > 0.0 && height > 0.0 {
                log::debug!(
                    "Using font '{}' at {}pt: cell={}x{}",
                    font_name,
                    font_size,
                    char_width,
                    height * 1.1
                );
                return CellDimensions {
                    width: char_width,
                    height: height * 1.1, // Small line spacing factor
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
        log::warn!(
            "Font metrics unavailable, using layout measurement: {}x{}",
            width,
            height
        );
        return CellDimensions {
            width: width as f64,
            height: height as f64 * 1.1,
        };
    }

    // This should never happen on a functioning system with fonts installed
    panic!(
        "Failed to load any font or measure text. \
         Please ensure fonts are installed (e.g., fonts-dejavu or similar)."
    );
}

/// Messages from PTY reader thread
enum PtyMessage {
    Data(Vec<u8>),
    Exited,
}

/// Draw the terminal contents
fn draw_terminal(
    cr: &cairo::Context,
    terminal: &Arc<Mutex<Terminal>>,
    theme: &Theme,
    font_family: &str,
    font_size: f64,
    cell_dims: CellDimensions,
    background_override: Option<cterm_core::color::Rgb>,
) {
    let term = terminal.lock();
    let screen = term.screen();
    let palette = &theme.colors;

    // Draw background (use override if set, otherwise use theme)
    let bg = background_override.as_ref().unwrap_or(&palette.background);
    let (r, g, b) = bg.to_f64();
    cr.set_source_rgb(r, g, b);
    cr.paint().ok();

    // Create Pango layout for text rendering
    let pango_context = pangocairo::functions::create_context(cr);
    let layout = pango::Layout::new(&pango_context);

    // Set font
    let font_desc = pango::FontDescription::from_string(&format!("{} {}", font_family, font_size));
    layout.set_font_description(Some(&font_desc));

    // Use pre-calculated cell dimensions
    let cell_width = cell_dims.width;
    let cell_height = cell_dims.height;

    // Draw cells
    let grid = screen.grid();
    let scroll_offset = screen.scroll_offset;
    let scrollback_len = screen.scrollback().len();

    for row_idx in 0..grid.height() {
        if let Some(row) = grid.row(row_idx) {
            let y = row_idx as f64 * cell_height;

            // Calculate absolute line for selection checking
            let absolute_line = scrollback_len.saturating_sub(scroll_offset) + row_idx;

            for col_idx in 0..grid.width() {
                let cell = &row[col_idx];
                let x = col_idx as f64 * cell_width;

                // Skip wide char spacers
                if cell.attrs.contains(CellAttrs::WIDE_SPACER) {
                    continue;
                }

                // Check if this cell is selected
                let is_selected = screen.is_selected(absolute_line, col_idx);

                // Determine if cell has INVERSE attribute (XOR with selection)
                let is_inverted = cell.attrs.contains(CellAttrs::INVERSE) != is_selected;

                // Draw background (always draw for selected cells to show highlight)
                let needs_bg = cell.bg != Color::Default || is_inverted || is_selected;

                if needs_bg {
                    let bg_color = if is_inverted {
                        // Inverted: use foreground color as background
                        if cell.fg == Color::Default {
                            palette.foreground
                        } else {
                            cell.fg.to_rgb(palette)
                        }
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
                    let fg_color = if is_inverted {
                        // Inverted: use background color as foreground
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
                    let attrs = pango::AttrList::new();

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
