//! Terminal - Main terminal state combining screen and parser
//!
//! Provides a high-level interface for terminal emulation.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::parser::Parser;
use crate::pty::{Pty, PtyConfig, PtyError};
use crate::screen::{Screen, ScreenConfig};

/// Events emitted by the terminal
#[derive(Debug, Clone)]
pub enum TerminalEvent {
    /// Terminal title changed
    TitleChanged(String),
    /// Bell was rung
    Bell,
    /// Process exited with code
    ProcessExited(u32),
    /// Terminal content changed (needs redraw)
    ContentChanged,
}

/// Terminal configuration
#[derive(Debug, Clone)]
pub struct TerminalConfig {
    /// Screen configuration
    pub screen: ScreenConfig,
    /// PTY configuration
    pub pty: PtyConfig,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            screen: ScreenConfig::default(),
            pty: PtyConfig::default(),
        }
    }
}

/// Terminal instance managing screen, parser, and PTY
pub struct Terminal {
    screen: Screen,
    parser: Parser,
    pty: Option<Pty>,
    last_title: String,
}

impl Terminal {
    /// Create a new terminal with the given dimensions
    pub fn new(cols: usize, rows: usize, config: ScreenConfig) -> Self {
        Self {
            screen: Screen::new(cols, rows, config),
            parser: Parser::new(),
            pty: None,
            last_title: String::new(),
        }
    }

    /// Create a terminal and spawn a shell
    pub fn with_shell(
        cols: usize,
        rows: usize,
        screen_config: ScreenConfig,
        pty_config: &PtyConfig,
    ) -> Result<Self, PtyError> {
        let mut config = pty_config.clone();
        config.size.cols = cols as u16;
        config.size.rows = rows as u16;

        let pty = Pty::new(&config)?;

        Ok(Self {
            screen: Screen::new(cols, rows, screen_config),
            parser: Parser::new(),
            pty: Some(pty),
            last_title: String::new(),
        })
    }

    /// Get a reference to the screen
    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    /// Get a mutable reference to the screen
    pub fn screen_mut(&mut self) -> &mut Screen {
        &mut self.screen
    }

    /// Get the PTY if available
    pub fn pty(&self) -> Option<&Pty> {
        self.pty.as_ref()
    }

    /// Process input from the PTY and update the screen
    pub fn process(&mut self, data: &[u8]) -> Vec<TerminalEvent> {
        let mut events = Vec::new();

        self.parser.parse(&mut self.screen, data);

        // Check for title change
        if self.screen.title != self.last_title {
            self.last_title = self.screen.title.clone();
            events.push(TerminalEvent::TitleChanged(self.last_title.clone()));
        }

        // Always emit content changed if there was data
        if !data.is_empty() {
            events.push(TerminalEvent::ContentChanged);
        }

        events
    }

    /// Write input to the PTY (keyboard input)
    pub fn write(&self, data: &[u8]) -> Result<(), PtyError> {
        if let Some(ref pty) = self.pty {
            pty.write(data)?;
        }
        Ok(())
    }

    /// Write a string to the PTY
    pub fn write_str(&self, s: &str) -> Result<(), PtyError> {
        self.write(s.as_bytes())
    }

    /// Resize the terminal
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.screen.resize(cols, rows);
        if let Some(ref pty) = self.pty {
            let _ = pty.resize(rows as u16, cols as u16);
        }
    }

    /// Check if the process is still running
    pub fn is_running(&self) -> bool {
        self.pty.as_ref().map_or(false, |p| p.is_running())
    }

    /// Get terminal width
    pub fn cols(&self) -> usize {
        self.screen.width()
    }

    /// Get terminal height
    pub fn rows(&self) -> usize {
        self.screen.height()
    }

    /// Get current title
    pub fn title(&self) -> &str {
        &self.screen.title
    }

    /// Scroll viewport up (into scrollback)
    pub fn scroll_viewport_up(&mut self, lines: usize) {
        let max_offset = self.screen.scrollback().len();
        self.screen.scroll_offset = (self.screen.scroll_offset + lines).min(max_offset);
    }

    /// Scroll viewport down (towards bottom)
    pub fn scroll_viewport_down(&mut self, lines: usize) {
        self.screen.scroll_offset = self.screen.scroll_offset.saturating_sub(lines);
    }

    /// Reset viewport to bottom
    pub fn scroll_viewport_to_bottom(&mut self) {
        self.screen.scroll_offset = 0;
    }

    /// Check if viewport is at bottom
    pub fn is_at_bottom(&self) -> bool {
        self.screen.scroll_offset == 0
    }

    /// Handle keyboard input and generate appropriate escape sequences
    pub fn handle_key(&self, key: Key, modifiers: Modifiers) -> Option<Vec<u8>> {
        let app_cursor = self.screen.modes.application_cursor;
        let app_keypad = self.screen.modes.application_keypad;

        match key {
            Key::Char(c) => {
                if modifiers.contains(Modifiers::CTRL) {
                    // Control characters
                    let ctrl_char = match c.to_ascii_lowercase() {
                        'a'..='z' => Some((c.to_ascii_lowercase() as u8 - b'a' + 1) as u8),
                        '[' => Some(0x1b),
                        '\\' => Some(0x1c),
                        ']' => Some(0x1d),
                        '^' => Some(0x1e),
                        '_' => Some(0x1f),
                        _ => None,
                    };

                    ctrl_char.map(|b| vec![b])
                } else if modifiers.contains(Modifiers::ALT) {
                    // Alt + char = Escape + char
                    let mut buf = String::from('\x1b');
                    buf.push(c);
                    Some(buf.into_bytes())
                } else {
                    // Regular character
                    let mut buf = [0u8; 4];
                    let s = c.encode_utf8(&mut buf);
                    Some(s.as_bytes().to_vec())
                }
            }
            Key::Enter => Some(b"\r".to_vec()),
            Key::Tab => Some(b"\t".to_vec()),
            Key::Backspace => Some(b"\x7f".to_vec()),
            Key::Escape => Some(b"\x1b".to_vec()),
            Key::Up => Some(cursor_key(b'A', modifiers, app_cursor)),
            Key::Down => Some(cursor_key(b'B', modifiers, app_cursor)),
            Key::Right => Some(cursor_key(b'C', modifiers, app_cursor)),
            Key::Left => Some(cursor_key(b'D', modifiers, app_cursor)),
            Key::Home => Some(cursor_key(b'H', modifiers, app_cursor)),
            Key::End => Some(cursor_key(b'F', modifiers, app_cursor)),
            Key::PageUp => Some(format!("\x1b[5~").into_bytes()),
            Key::PageDown => Some(format!("\x1b[6~").into_bytes()),
            Key::Insert => Some(format!("\x1b[2~").into_bytes()),
            Key::Delete => Some(format!("\x1b[3~").into_bytes()),
            Key::F(n) => Some(function_key(n, modifiers)),
        }
    }
}

fn cursor_key(key: u8, modifiers: Modifiers, app_cursor: bool) -> Vec<u8> {
    let modifier = modifier_param(modifiers);

    if modifier > 1 {
        format!("\x1b[1;{}{}", modifier, key as char).into_bytes()
    } else if app_cursor {
        vec![0x1b, b'O', key]
    } else {
        vec![0x1b, b'[', key]
    }
}

fn function_key(n: u8, modifiers: Modifiers) -> Vec<u8> {
    let modifier = modifier_param(modifiers);

    let code = match n {
        1 => "11",
        2 => "12",
        3 => "13",
        4 => "14",
        5 => "15",
        6 => "17",
        7 => "18",
        8 => "19",
        9 => "20",
        10 => "21",
        11 => "23",
        12 => "24",
        _ => return Vec::new(),
    };

    if modifier > 1 {
        format!("\x1b[{};{}~", code, modifier).into_bytes()
    } else {
        format!("\x1b[{}~", code).into_bytes()
    }
}

fn modifier_param(modifiers: Modifiers) -> u8 {
    let mut param = 1u8;
    if modifiers.contains(Modifiers::SHIFT) {
        param += 1;
    }
    if modifiers.contains(Modifiers::ALT) {
        param += 2;
    }
    if modifiers.contains(Modifiers::CTRL) {
        param += 4;
    }
    param
}

/// Keyboard key
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    F(u8),
}

bitflags::bitflags! {
    /// Keyboard modifiers
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Modifiers: u8 {
        const SHIFT = 1 << 0;
        const CTRL = 1 << 1;
        const ALT = 1 << 2;
        const SUPER = 1 << 3;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terminal_new() {
        let term = Terminal::new(80, 24, ScreenConfig::default());
        assert_eq!(term.cols(), 80);
        assert_eq!(term.rows(), 24);
    }

    #[test]
    fn test_terminal_process() {
        let mut term = Terminal::new(80, 24, ScreenConfig::default());

        term.process(b"Hello, World!");

        assert_eq!(term.screen().get_cell(0, 0).unwrap().c, 'H');
        assert_eq!(term.screen().get_cell(0, 12).unwrap().c, '!');
    }

    #[test]
    fn test_terminal_resize() {
        let mut term = Terminal::new(80, 24, ScreenConfig::default());

        term.process(b"X");
        term.resize(100, 30);

        assert_eq!(term.cols(), 100);
        assert_eq!(term.rows(), 30);
        assert_eq!(term.screen().get_cell(0, 0).unwrap().c, 'X');
    }

    #[test]
    fn test_handle_key() {
        let term = Terminal::new(80, 24, ScreenConfig::default());

        // Regular character
        assert_eq!(
            term.handle_key(Key::Char('a'), Modifiers::empty()),
            Some(b"a".to_vec())
        );

        // Enter
        assert_eq!(
            term.handle_key(Key::Enter, Modifiers::empty()),
            Some(b"\r".to_vec())
        );

        // Ctrl+C
        assert_eq!(
            term.handle_key(Key::Char('c'), Modifiers::CTRL),
            Some(vec![0x03])
        );

        // Arrow key
        let up = term.handle_key(Key::Up, Modifiers::empty());
        assert_eq!(up, Some(b"\x1b[A".to_vec()));
    }
}
