//! ANSI/VT sequence parser
//!
//! Uses the `vte` crate for parsing escape sequences and generates
//! actions that can be applied to the terminal screen.

use std::sync::Arc;
use vte::Params;

use crate::cell::{CellAttrs, Hyperlink};
use crate::color::{AnsiColor, Color, Rgb};
use crate::screen::{
    ClearMode, ClipboardOperation, ClipboardSelection, CursorStyle, LineClearMode, MouseMode,
    Screen,
};

/// Parser wraps the vte parser and applies actions to a Screen
pub struct Parser {
    state_machine: vte::Parser,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state_machine: vte::Parser::new(),
        }
    }

    /// Parse input bytes and apply actions to the screen
    pub fn parse(&mut self, screen: &mut Screen, bytes: &[u8]) {
        let mut performer = ScreenPerformer { screen };
        for byte in bytes {
            self.state_machine.advance(&mut performer, *byte);
        }
    }
}

/// Performer that applies VTE actions to a Screen
struct ScreenPerformer<'a> {
    screen: &'a mut Screen,
}

impl vte::Perform for ScreenPerformer<'_> {
    fn print(&mut self, c: char) {
        self.screen.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            // Bell (BEL)
            0x07 => {
                self.screen.bell = true;
                log::debug!("Bell");
            }
            // Backspace (BS)
            0x08 => {
                if self.screen.cursor.col > 0 {
                    self.screen.cursor.col -= 1;
                }
            }
            // Horizontal Tab (HT)
            0x09 => {
                self.screen.tab_forward(1);
            }
            // Line Feed (LF), Vertical Tab (VT), Form Feed (FF)
            0x0a..=0x0c => {
                self.screen.line_feed();
                if self.screen.modes.line_feed_mode {
                    self.screen.carriage_return();
                }
            }
            // Carriage Return (CR)
            0x0d => {
                self.screen.carriage_return();
            }
            // Shift Out (SO) - switch to G1 charset
            0x0e => {
                self.screen.modes.charset_g1_active = true;
                log::trace!("Shift Out: activated G1 charset");
            }
            // Shift In (SI) - switch to G0 charset
            0x0f => {
                self.screen.modes.charset_g1_active = false;
                log::trace!("Shift In: activated G0 charset");
            }
            _ => {
                log::trace!("Unhandled execute byte: 0x{:02x}", byte);
            }
        }
    }

    fn hook(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        log::trace!(
            "DCS hook: params={:?}, intermediates={:?}, action={:?}",
            params_to_vec(params),
            intermediates,
            action
        );
    }

    fn put(&mut self, _byte: u8) {
        // DCS data - used for things like sixel graphics
    }

    fn unhook(&mut self) {
        // End of DCS sequence
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        if params.is_empty() {
            return;
        }

        let command = match std::str::from_utf8(params[0]) {
            Ok(s) => s.parse::<u32>().unwrap_or(u32::MAX),
            Err(_) => return,
        };

        match command {
            // Set window title
            0 | 2 => {
                if params.len() > 1 {
                    if let Ok(title) = std::str::from_utf8(params[1]) {
                        self.screen.title = title.to_string();
                        log::debug!("Set title: {}", title);
                    }
                }
            }
            // Set icon name
            1 => {
                if params.len() > 1 {
                    if let Ok(name) = std::str::from_utf8(params[1]) {
                        self.screen.icon_name = name.to_string();
                    }
                }
            }
            // Hyperlink (OSC 8)
            8 => {
                if params.len() >= 3 {
                    let uri = std::str::from_utf8(params[2]).unwrap_or("");
                    if uri.is_empty() {
                        // End hyperlink
                        self.screen.style.hyperlink = None;
                    } else {
                        // Parse params for id
                        let param_str = std::str::from_utf8(params[1]).unwrap_or("");
                        let id = param_str
                            .split(';')
                            .find_map(|p| p.strip_prefix("id="))
                            .map(String::from);

                        let hyperlink = if let Some(id) = id {
                            Hyperlink::with_id(id, uri.to_string())
                        } else {
                            Hyperlink::new(uri.to_string())
                        };

                        self.screen.style.hyperlink = Some(Arc::new(hyperlink));
                    }
                }
            }
            // Set/query colors (10-19)
            // OSC 10 = foreground, 11 = background, 12 = cursor
            10..=12 => {
                if params.len() > 1 {
                    let query = std::str::from_utf8(params[1]).unwrap_or("");
                    if query == "?" {
                        // Color query - respond with current color
                        // Format: OSC Ps ; rgb:RRRR/GGGG/BBBB ST
                        // We respond with a placeholder since actual colors are in the UI layer
                        let color_name = match command {
                            10 => "foreground",
                            11 => "background",
                            12 => "cursor",
                            _ => "unknown",
                        };
                        log::trace!("Color query for {}", color_name);
                        // Queue a color query response event
                        // The actual response will be generated by the UI layer
                        // which has access to the theme colors
                        self.screen.queue_color_query(command as u8);
                    } else {
                        // Color setting - log but don't implement
                        // Dynamic color setting is rarely used
                        log::trace!("Color set OSC {}: {}", command, query);
                    }
                }
            }
            // Other color OSCs (13-19) - less common
            13..=19 => {
                log::trace!("Unhandled color OSC: {}", command);
            }
            // Copy to clipboard (52)
            52 => {
                // OSC 52 ; Pc ; Pd ST
                // Pc = clipboard selection (c=clipboard, p=primary, s=select)
                // Pd = base64 data or ? for query
                if params.len() >= 3 {
                    let selection_str = std::str::from_utf8(params[1]).unwrap_or("c");
                    let data_str = std::str::from_utf8(params[2]).unwrap_or("");

                    // Parse selection - default to clipboard
                    let selection = if selection_str.contains('p') {
                        ClipboardSelection::Primary
                    } else if selection_str.contains('s') {
                        ClipboardSelection::Select
                    } else {
                        ClipboardSelection::Clipboard
                    };

                    if data_str == "?" {
                        // Query clipboard
                        log::debug!("Clipboard query for {:?}", selection);
                        self.screen
                            .queue_clipboard_op(ClipboardOperation::Query { selection });
                    } else if !data_str.is_empty() {
                        // Set clipboard - decode base64
                        use base64::Engine;
                        match base64::engine::general_purpose::STANDARD.decode(data_str) {
                            Ok(decoded) => {
                                log::debug!(
                                    "Clipboard set {:?}: {} bytes",
                                    selection,
                                    decoded.len()
                                );
                                self.screen.queue_clipboard_op(ClipboardOperation::Set {
                                    selection,
                                    data: decoded,
                                });
                            }
                            Err(e) => {
                                log::warn!("Failed to decode OSC 52 base64 data: {}", e);
                            }
                        }
                    }
                }
            }
            _ => {
                log::trace!("Unhandled OSC: {}", command);
            }
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        let params_vec = params_to_vec(params);

        match (action, intermediates) {
            // Cursor Up (CUU)
            ('A', []) => {
                let n = first_param(&params_vec, 1) as i32;
                self.screen.move_cursor_relative(-n, 0);
            }
            // Cursor Down (CUD)
            ('B', []) => {
                let n = first_param(&params_vec, 1) as i32;
                self.screen.move_cursor_relative(n, 0);
            }
            // Cursor Forward (CUF)
            ('C', []) => {
                let n = first_param(&params_vec, 1) as i32;
                self.screen.move_cursor_relative(0, n);
            }
            // Cursor Back (CUB)
            ('D', []) => {
                let n = first_param(&params_vec, 1) as i32;
                self.screen.move_cursor_relative(0, -n);
            }
            // Cursor Next Line (CNL)
            ('E', []) => {
                let n = first_param(&params_vec, 1) as i32;
                self.screen.move_cursor_relative(n, 0);
                self.screen.cursor.col = 0;
            }
            // Cursor Previous Line (CPL)
            ('F', []) => {
                let n = first_param(&params_vec, 1) as i32;
                self.screen.move_cursor_relative(-n, 0);
                self.screen.cursor.col = 0;
            }
            // Cursor Horizontal Absolute (CHA)
            ('G', []) => {
                let col = first_param(&params_vec, 1).saturating_sub(1);
                self.screen.cursor.col = col.min(self.screen.width().saturating_sub(1));
            }
            // Cursor Position (CUP) / Horizontal and Vertical Position (HVP)
            ('H', []) | ('f', []) => {
                let row = first_param(&params_vec, 1).saturating_sub(1);
                let col = second_param(&params_vec, 1).saturating_sub(1);
                self.screen.move_cursor(row, col);
            }
            // Erase in Display (ED)
            ('J', []) => {
                let mode = first_param(&params_vec, 0);
                match mode {
                    0 => self.screen.clear(ClearMode::Below),
                    1 => self.screen.clear(ClearMode::Above),
                    2 => self.screen.clear(ClearMode::All),
                    3 => self.screen.clear(ClearMode::Scrollback),
                    _ => {}
                }
            }
            // Erase in Line (EL)
            ('K', []) => {
                let mode = first_param(&params_vec, 0);
                match mode {
                    0 => self.screen.clear_line(LineClearMode::Right),
                    1 => self.screen.clear_line(LineClearMode::Left),
                    2 => self.screen.clear_line(LineClearMode::All),
                    _ => {}
                }
            }
            // Insert Lines (IL)
            ('L', []) => {
                let n = first_param(&params_vec, 1);
                self.screen.insert_lines(n);
            }
            // Delete Lines (DL)
            ('M', []) => {
                let n = first_param(&params_vec, 1);
                self.screen.delete_lines(n);
            }
            // Delete Characters (DCH)
            ('P', []) => {
                let n = first_param(&params_vec, 1);
                self.screen.delete_chars(n);
            }
            // Scroll Up (SU)
            ('S', []) => {
                let n = first_param(&params_vec, 1);
                self.screen.scroll_up(n);
            }
            // Scroll Down (SD)
            ('T', []) => {
                let n = first_param(&params_vec, 1);
                self.screen.scroll_down(n);
            }
            // Erase Characters (ECH)
            ('X', []) => {
                let n = first_param(&params_vec, 1);
                let cursor_row = self.screen.cursor.row;
                let cursor_col = self.screen.cursor.col;
                let width = self.screen.width();
                let count = n.min(width.saturating_sub(cursor_col));
                if let Some(row) = self.screen.grid_mut().row_mut(cursor_row) {
                    for i in 0..count {
                        row[cursor_col + i].reset();
                    }
                }
            }
            // Cursor Backward Tabulation (CBT)
            ('Z', []) => {
                let n = first_param(&params_vec, 1);
                self.screen.tab_backward(n);
            }
            // Insert Characters (ICH)
            ('@', []) => {
                let n = first_param(&params_vec, 1);
                let cursor_row = self.screen.cursor.row;
                let col = self.screen.cursor.col;
                let width = self.screen.width();
                if let Some(row) = self.screen.grid_mut().row_mut(cursor_row) {
                    // Shift characters right
                    for i in (col + n..width).rev() {
                        row[i] = row[i - n].clone();
                    }
                    // Clear inserted positions
                    for i in col..col + n.min(width.saturating_sub(col)) {
                        row[i].reset();
                    }
                }
            }
            // Vertical Line Position Absolute (VPA)
            ('d', []) => {
                let row = first_param(&params_vec, 1).saturating_sub(1);
                self.screen.cursor.row = row.min(self.screen.height().saturating_sub(1));
            }
            // SGR - Select Graphic Rendition
            ('m', []) => {
                self.handle_sgr(&params_vec);
            }
            // Device Status Report (DSR)
            ('n', []) => {
                let mode = first_param(&params_vec, 0);
                match mode {
                    5 => {
                        // Status report - respond "OK"
                        self.screen.queue_response(b"\x1b[0n".to_vec());
                    }
                    6 => {
                        // Cursor position report - respond with CSI row;col R
                        let row = self.screen.cursor.row + 1;
                        let col = self.screen.cursor.col + 1;
                        let response = format!("\x1b[{};{}R", row, col);
                        self.screen.queue_response(response.into_bytes());
                    }
                    _ => {
                        log::trace!("Unknown DSR mode: {}", mode);
                    }
                }
            }
            // Set Top and Bottom Margins (DECSTBM)
            ('r', []) => {
                let top = first_param(&params_vec, 1).saturating_sub(1);
                let bottom = if params_vec.len() > 1 {
                    params_vec[1]
                } else {
                    self.screen.height()
                };
                self.screen.set_scroll_region(top, bottom);
                self.screen.move_cursor(0, 0);
            }
            // Save Cursor (DECSC)
            ('s', []) => {
                self.screen.save_cursor();
            }
            // Restore Cursor (DECRC)
            ('u', []) => {
                self.screen.restore_cursor();
            }
            // Window manipulation (XTWINOPS)
            ('t', []) => {
                log::trace!("Window manipulation: {:?}", params_vec);
            }
            // Set Mode (SM) / Reset Mode (RM)
            ('h', [b'?']) | ('l', [b'?']) => {
                let set = action == 'h';
                for &param in &params_vec {
                    self.handle_dec_mode(param, set);
                }
            }
            // ANSI modes
            ('h', []) | ('l', []) => {
                let set = action == 'h';
                for &param in &params_vec {
                    self.handle_ansi_mode(param, set);
                }
            }
            // Soft reset (DECSTR)
            ('p', [b'!']) => {
                self.screen.style.reset();
                self.screen.modes.insert_mode = false;
                self.screen.modes.origin_mode = false;
                self.screen.reset_scroll_region();
            }
            // Set cursor style (DECSCUSR)
            ('q', [b' ']) => {
                let style = first_param(&params_vec, 0);
                match style {
                    0 | 1 => {
                        self.screen.cursor.style = CursorStyle::Block;
                        self.screen.cursor.blink = true;
                    }
                    2 => {
                        self.screen.cursor.style = CursorStyle::Block;
                        self.screen.cursor.blink = false;
                    }
                    3 => {
                        self.screen.cursor.style = CursorStyle::Underline;
                        self.screen.cursor.blink = true;
                    }
                    4 => {
                        self.screen.cursor.style = CursorStyle::Underline;
                        self.screen.cursor.blink = false;
                    }
                    5 => {
                        self.screen.cursor.style = CursorStyle::Bar;
                        self.screen.cursor.blink = true;
                    }
                    6 => {
                        self.screen.cursor.style = CursorStyle::Bar;
                        self.screen.cursor.blink = false;
                    }
                    _ => {}
                }
            }
            // Cursor Horizontal Tab forward (CHT)
            ('I', []) => {
                let n = first_param(&params_vec, 1);
                self.screen.tab_forward(n);
            }
            // Tab Clear (TBC)
            ('g', []) => {
                let mode = first_param(&params_vec, 0);
                match mode {
                    0 => self.screen.clear_tab_stop(),
                    3 => self.screen.clear_all_tab_stops(),
                    _ => {}
                }
            }
            _ => {
                log::trace!(
                    "Unhandled CSI: action={:?}, intermediates={:?}, params={:?}",
                    action,
                    intermediates,
                    params_vec
                );
            }
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        match (byte, intermediates) {
            // Reset (RIS)
            (b'c', []) => {
                self.screen.reset();
            }
            // Save Cursor (DECSC)
            (b'7', []) => {
                self.screen.save_cursor();
            }
            // Restore Cursor (DECRC)
            (b'8', []) => {
                self.screen.restore_cursor();
            }
            // Index (IND) - move cursor down, scroll if at bottom
            (b'D', []) => {
                self.screen.line_feed();
            }
            // Next Line (NEL)
            (b'E', []) => {
                self.screen.carriage_return();
                self.screen.line_feed();
            }
            // Reverse Index (RI) - move cursor up, scroll if at top
            (b'M', []) => {
                if self.screen.cursor.row == self.screen.scroll_region().top {
                    self.screen.scroll_down(1);
                } else if self.screen.cursor.row > 0 {
                    self.screen.cursor.row -= 1;
                }
            }
            // Application Keypad (DECKPAM)
            (b'=', []) => {
                self.screen.modes.application_keypad = true;
            }
            // Normal Keypad (DECKPNM)
            (b'>', []) => {
                self.screen.modes.application_keypad = false;
            }
            // Set tab stop at current column (HTS)
            (b'H', []) => {
                self.screen.set_tab_stop();
            }
            _ => {
                log::trace!(
                    "Unhandled ESC: byte=0x{:02x} ({:?}), intermediates={:?}",
                    byte,
                    byte as char,
                    intermediates
                );
            }
        }
    }
}

impl ScreenPerformer<'_> {
    /// Handle SGR (Select Graphic Rendition) sequences
    fn handle_sgr(&mut self, params: &[usize]) {
        if params.is_empty() {
            // Reset all attributes
            self.screen.style.reset();
            return;
        }

        let mut iter = params.iter().peekable();

        while let Some(&param) = iter.next() {
            match param {
                // Reset
                0 => self.screen.style.reset(),
                // Bold
                1 => self.screen.style.attrs.insert(CellAttrs::BOLD),
                // Dim/faint
                2 => self.screen.style.attrs.insert(CellAttrs::DIM),
                // Italic
                3 => self.screen.style.attrs.insert(CellAttrs::ITALIC),
                // Underline
                4 => {
                    // Check for extended underline
                    if let Some(&&sub) = iter.peek() {
                        match sub {
                            0 => {
                                iter.next();
                                self.screen.style.attrs.clear_underline();
                            }
                            1 => {
                                iter.next();
                                self.screen.style.attrs.clear_underline();
                                self.screen.style.attrs.insert(CellAttrs::UNDERLINE);
                            }
                            2 => {
                                iter.next();
                                self.screen.style.attrs.clear_underline();
                                self.screen.style.attrs.insert(CellAttrs::DOUBLE_UNDERLINE);
                            }
                            3 => {
                                iter.next();
                                self.screen.style.attrs.clear_underline();
                                self.screen.style.attrs.insert(CellAttrs::CURLY_UNDERLINE);
                            }
                            4 => {
                                iter.next();
                                self.screen.style.attrs.clear_underline();
                                self.screen.style.attrs.insert(CellAttrs::DOTTED_UNDERLINE);
                            }
                            5 => {
                                iter.next();
                                self.screen.style.attrs.clear_underline();
                                self.screen.style.attrs.insert(CellAttrs::DASHED_UNDERLINE);
                            }
                            _ => {
                                self.screen.style.attrs.insert(CellAttrs::UNDERLINE);
                            }
                        }
                    } else {
                        self.screen.style.attrs.insert(CellAttrs::UNDERLINE);
                    }
                }
                // Blink
                5 | 6 => self.screen.style.attrs.insert(CellAttrs::BLINK),
                // Inverse
                7 => self.screen.style.attrs.insert(CellAttrs::INVERSE),
                // Hidden
                8 => self.screen.style.attrs.insert(CellAttrs::HIDDEN),
                // Strikethrough
                9 => self.screen.style.attrs.insert(CellAttrs::STRIKETHROUGH),
                // Normal intensity (not bold or dim)
                22 => {
                    self.screen.style.attrs.remove(CellAttrs::BOLD);
                    self.screen.style.attrs.remove(CellAttrs::DIM);
                }
                // Not italic
                23 => self.screen.style.attrs.remove(CellAttrs::ITALIC),
                // Not underlined
                24 => self.screen.style.attrs.clear_underline(),
                // Not blinking
                25 => self.screen.style.attrs.remove(CellAttrs::BLINK),
                // Not inverse
                27 => self.screen.style.attrs.remove(CellAttrs::INVERSE),
                // Not hidden
                28 => self.screen.style.attrs.remove(CellAttrs::HIDDEN),
                // Not strikethrough
                29 => self.screen.style.attrs.remove(CellAttrs::STRIKETHROUGH),
                // Foreground colors (30-37)
                30..=37 => {
                    if let Some(color) = AnsiColor::from_index((param - 30) as u8) {
                        self.screen.style.fg = Color::Ansi(color);
                    }
                }
                // Extended foreground color
                38 => {
                    if let Some(color) = self.parse_extended_color(&mut iter) {
                        self.screen.style.fg = color;
                    }
                }
                // Default foreground
                39 => self.screen.style.fg = Color::Default,
                // Background colors (40-47)
                40..=47 => {
                    if let Some(color) = AnsiColor::from_index((param - 40) as u8) {
                        self.screen.style.bg = Color::Ansi(color);
                    }
                }
                // Extended background color
                48 => {
                    if let Some(color) = self.parse_extended_color(&mut iter) {
                        self.screen.style.bg = color;
                    }
                }
                // Default background
                49 => self.screen.style.bg = Color::Default,
                // Overline
                53 => self.screen.style.attrs.insert(CellAttrs::OVERLINE),
                // Not overline
                55 => self.screen.style.attrs.remove(CellAttrs::OVERLINE),
                // Underline color
                58 => {
                    if let Some(color) = self.parse_extended_color(&mut iter) {
                        self.screen.style.underline_color = Some(color);
                    }
                }
                // Default underline color
                59 => self.screen.style.underline_color = None,
                // Bright foreground colors (90-97)
                90..=97 => {
                    if let Some(color) = AnsiColor::from_index((param - 90 + 8) as u8) {
                        self.screen.style.fg = Color::Ansi(color);
                    }
                }
                // Bright background colors (100-107)
                100..=107 => {
                    if let Some(color) = AnsiColor::from_index((param - 100 + 8) as u8) {
                        self.screen.style.bg = Color::Ansi(color);
                    }
                }
                _ => {
                    log::trace!("Unknown SGR parameter: {}", param);
                }
            }
        }
    }

    /// Parse extended color (256-color or RGB)
    fn parse_extended_color(
        &self,
        iter: &mut std::iter::Peekable<std::slice::Iter<usize>>,
    ) -> Option<Color> {
        let mode = *iter.next()?;

        match mode {
            // 256-color
            5 => {
                let index = *iter.next()? as u8;
                Some(Color::Indexed(index))
            }
            // RGB
            2 => {
                let r = *iter.next()? as u8;
                let g = *iter.next()? as u8;
                let b = *iter.next()? as u8;
                Some(Color::Rgb(Rgb::new(r, g, b)))
            }
            _ => None,
        }
    }

    /// Handle DEC private mode set/reset
    fn handle_dec_mode(&mut self, mode: usize, set: bool) {
        match mode {
            // DECCKM - Cursor Keys Mode
            1 => self.screen.modes.application_cursor = set,
            // DECOM - Origin Mode
            6 => {
                self.screen.modes.origin_mode = set;
                self.screen.move_cursor(0, 0);
            }
            // DECAWM - Auto Wrap Mode
            7 => self.screen.modes.auto_wrap = set,
            // X10 Mouse Reporting
            9 => {
                self.screen.modes.mouse_mode = if set { MouseMode::X10 } else { MouseMode::None };
            }
            // DECTCEM - Show Cursor
            25 => self.screen.modes.show_cursor = set,
            // Normal Mouse Tracking
            1000 => {
                self.screen.modes.mouse_mode = if set {
                    MouseMode::Normal
                } else {
                    MouseMode::None
                };
            }
            // Button Event Mouse Tracking
            1002 => {
                self.screen.modes.mouse_mode = if set {
                    MouseMode::ButtonEvent
                } else {
                    MouseMode::None
                };
            }
            // Any Event Mouse Tracking
            1003 => {
                self.screen.modes.mouse_mode = if set {
                    MouseMode::AnyEvent
                } else {
                    MouseMode::None
                };
            }
            // Focus Events
            1004 => self.screen.modes.focus_events = set,
            // UTF-8 Mouse Mode
            1005 => { /* UTF-8 encoding for mouse coordinates - not implemented */ }
            // SGR Mouse Mode (extended coordinates)
            1006 => self.screen.modes.sgr_mouse = set,
            // Alternate Screen Buffer
            1047 => {
                if set {
                    self.screen.enter_alternate_screen();
                } else {
                    self.screen.exit_alternate_screen();
                }
            }
            // Save/Restore Cursor
            1048 => {
                if set {
                    self.screen.save_cursor();
                } else {
                    self.screen.restore_cursor();
                }
            }
            // Alternate Screen Buffer with cursor save/restore
            1049 => {
                if set {
                    self.screen.save_cursor();
                    self.screen.enter_alternate_screen();
                    self.screen.clear(ClearMode::All);
                } else {
                    self.screen.exit_alternate_screen();
                    self.screen.restore_cursor();
                }
            }
            // Bracketed Paste Mode
            2004 => self.screen.modes.bracketed_paste = set,
            _ => {
                log::trace!("Unknown DEC mode: {} = {}", mode, set);
            }
        }
    }

    /// Handle ANSI mode set/reset
    fn handle_ansi_mode(&mut self, mode: usize, set: bool) {
        match mode {
            // IRM - Insert Mode
            4 => self.screen.modes.insert_mode = set,
            // LNM - Line Feed/New Line Mode
            20 => self.screen.modes.line_feed_mode = set,
            _ => {
                log::trace!("Unknown ANSI mode: {} = {}", mode, set);
            }
        }
    }
}

// Helper functions

fn params_to_vec(params: &Params) -> Vec<usize> {
    let mut result = Vec::new();
    for item in params.iter() {
        for &subparam in item {
            result.push(subparam as usize);
        }
    }
    result
}

fn first_param(params: &[usize], default: usize) -> usize {
    params
        .first()
        .copied()
        .filter(|&v| v != 0)
        .unwrap_or(default)
}

fn second_param(params: &[usize], default: usize) -> usize {
    params
        .get(1)
        .copied()
        .filter(|&v| v != 0)
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::ScreenConfig;

    fn make_screen() -> Screen {
        Screen::new(80, 24, ScreenConfig::default())
    }

    #[test]
    fn test_print() {
        let mut screen = make_screen();
        let mut parser = Parser::new();

        parser.parse(&mut screen, b"Hello");

        assert_eq!(screen.get_cell(0, 0).unwrap().c, 'H');
        assert_eq!(screen.get_cell(0, 4).unwrap().c, 'o');
        assert_eq!(screen.cursor.col, 5);
    }

    #[test]
    fn test_cursor_movement() {
        let mut screen = make_screen();
        let mut parser = Parser::new();

        // Move to position (5, 10) - CSI 6;11H (1-indexed)
        parser.parse(&mut screen, b"\x1b[6;11H");

        assert_eq!(screen.cursor.row, 5);
        assert_eq!(screen.cursor.col, 10);
    }

    #[test]
    fn test_sgr_colors() {
        let mut screen = make_screen();
        let mut parser = Parser::new();

        // Red foreground
        parser.parse(&mut screen, b"\x1b[31m");
        assert_eq!(screen.style.fg, Color::Ansi(AnsiColor::Red));

        // Blue background
        parser.parse(&mut screen, b"\x1b[44m");
        assert_eq!(screen.style.bg, Color::Ansi(AnsiColor::Blue));

        // Reset
        parser.parse(&mut screen, b"\x1b[0m");
        assert_eq!(screen.style.fg, Color::Default);
        assert_eq!(screen.style.bg, Color::Default);
    }

    #[test]
    fn test_sgr_256_color() {
        let mut screen = make_screen();
        let mut parser = Parser::new();

        // 256-color: color index 196 (bright red)
        parser.parse(&mut screen, b"\x1b[38;5;196m");
        assert_eq!(screen.style.fg, Color::Indexed(196));
    }

    #[test]
    fn test_sgr_rgb_color() {
        let mut screen = make_screen();
        let mut parser = Parser::new();

        // RGB: #ff8800
        parser.parse(&mut screen, b"\x1b[38;2;255;136;0m");
        assert_eq!(screen.style.fg, Color::Rgb(Rgb::new(255, 136, 0)));
    }

    #[test]
    fn test_clear_screen() {
        let mut screen = make_screen();
        let mut parser = Parser::new();

        parser.parse(&mut screen, b"XXXXX");
        parser.parse(&mut screen, b"\x1b[2J"); // Clear all

        for col in 0..5 {
            assert_eq!(screen.get_cell(0, col).unwrap().c, ' ');
        }
    }

    #[test]
    fn test_alternate_screen() {
        let mut screen = make_screen();
        let mut parser = Parser::new();

        parser.parse(&mut screen, b"Primary");
        parser.parse(&mut screen, b"\x1b[?1049h"); // Enter alternate
        assert!(screen.modes.alternate_screen);

        parser.parse(&mut screen, b"\x1b[?1049l"); // Exit alternate
        assert!(!screen.modes.alternate_screen);
        assert_eq!(screen.get_cell(0, 0).unwrap().c, 'P');
    }
}
