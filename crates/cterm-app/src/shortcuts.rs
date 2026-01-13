//! Keyboard shortcut management
//!
//! Handles parsing, matching, and managing keyboard shortcuts.

use std::collections::HashMap;
use std::str::FromStr;

use cterm_ui::events::{Action, KeyCode, Modifiers, Shortcut};

use crate::config::ShortcutsConfig;

/// Manages keyboard shortcuts
#[derive(Clone)]
pub struct ShortcutManager {
    /// Map from shortcut to action
    shortcuts: HashMap<Shortcut, Action>,
    /// Map from action to shortcut (for display)
    actions: HashMap<Action, Shortcut>,
}

impl ShortcutManager {
    /// Create a new shortcut manager with default shortcuts
    pub fn new() -> Self {
        let mut manager = Self {
            shortcuts: HashMap::new(),
            actions: HashMap::new(),
        };

        manager.load_defaults();
        manager
    }

    /// Load shortcuts from configuration
    pub fn from_config(config: &ShortcutsConfig) -> Self {
        let mut manager = Self::new();

        // Override with config values
        manager.bind_str(&config.new_tab, Action::NewTab);
        manager.bind_str(&config.close_tab, Action::CloseTab);
        manager.bind_str(&config.next_tab, Action::NextTab);
        manager.bind_str(&config.prev_tab, Action::PrevTab);
        manager.bind_str(&config.new_window, Action::NewWindow);
        manager.bind_str(&config.close_window, Action::CloseWindow);
        manager.bind_str(&config.copy, Action::Copy);
        manager.bind_str(&config.paste, Action::Paste);
        manager.bind_str(&config.select_all, Action::SelectAll);
        manager.bind_str(&config.zoom_in, Action::ZoomIn);
        manager.bind_str(&config.zoom_out, Action::ZoomOut);
        manager.bind_str(&config.zoom_reset, Action::ZoomReset);
        manager.bind_str(&config.scroll_up, Action::ScrollUp);
        manager.bind_str(&config.scroll_down, Action::ScrollDown);
        manager.bind_str(&config.scroll_page_up, Action::ScrollPageUp);
        manager.bind_str(&config.scroll_page_down, Action::ScrollPageDown);
        manager.bind_str(&config.preferences, Action::OpenPreferences);
        manager.bind_str(&config.find, Action::FindText);
        manager.bind_str(&config.reset, Action::ResetTerminal);

        manager
    }

    /// Load default shortcuts
    fn load_defaults(&mut self) {
        // Tab shortcuts
        self.bind(Shortcut::ctrl_shift(KeyCode::T), Action::NewTab);
        self.bind(Shortcut::ctrl_shift(KeyCode::W), Action::CloseTab);
        self.bind(Shortcut::new(KeyCode::Tab, Modifiers::CTRL), Action::NextTab);
        self.bind(Shortcut::ctrl_shift(KeyCode::Tab), Action::PrevTab);

        // Tab number shortcuts (Ctrl+1-9)
        self.bind(Shortcut::ctrl(KeyCode::Key1), Action::Tab(1));
        self.bind(Shortcut::ctrl(KeyCode::Key2), Action::Tab(2));
        self.bind(Shortcut::ctrl(KeyCode::Key3), Action::Tab(3));
        self.bind(Shortcut::ctrl(KeyCode::Key4), Action::Tab(4));
        self.bind(Shortcut::ctrl(KeyCode::Key5), Action::Tab(5));
        self.bind(Shortcut::ctrl(KeyCode::Key6), Action::Tab(6));
        self.bind(Shortcut::ctrl(KeyCode::Key7), Action::Tab(7));
        self.bind(Shortcut::ctrl(KeyCode::Key8), Action::Tab(8));
        self.bind(Shortcut::ctrl(KeyCode::Key9), Action::Tab(9));

        // Window shortcuts
        self.bind(Shortcut::ctrl_shift(KeyCode::N), Action::NewWindow);
        self.bind(Shortcut::ctrl_shift(KeyCode::Q), Action::CloseWindow);

        // Edit shortcuts
        self.bind(Shortcut::ctrl_shift(KeyCode::C), Action::Copy);
        self.bind(Shortcut::ctrl_shift(KeyCode::V), Action::Paste);
        self.bind(Shortcut::ctrl_shift(KeyCode::A), Action::SelectAll);

        // Zoom shortcuts
        self.bind(Shortcut::ctrl(KeyCode::Equals), Action::ZoomIn);
        self.bind(Shortcut::ctrl(KeyCode::Minus), Action::ZoomOut);
        self.bind(Shortcut::ctrl(KeyCode::Key0), Action::ZoomReset);

        // Scroll shortcuts
        self.bind(
            Shortcut::new(KeyCode::PageUp, Modifiers::SHIFT),
            Action::ScrollPageUp,
        );
        self.bind(
            Shortcut::new(KeyCode::PageDown, Modifiers::SHIFT),
            Action::ScrollPageDown,
        );
        self.bind(
            Shortcut::ctrl_shift(KeyCode::Home),
            Action::ScrollToTop,
        );
        self.bind(
            Shortcut::ctrl_shift(KeyCode::End),
            Action::ScrollToBottom,
        );

        // Other shortcuts
        self.bind(
            Shortcut::ctrl(KeyCode::Comma),
            Action::OpenPreferences,
        );
        self.bind(Shortcut::ctrl_shift(KeyCode::F), Action::FindText);
    }

    /// Bind a shortcut to an action
    pub fn bind(&mut self, shortcut: Shortcut, action: Action) {
        // Remove old binding for this action
        if let Some(old_shortcut) = self.actions.remove(&action) {
            self.shortcuts.remove(&old_shortcut);
        }

        // Remove old action for this shortcut
        if let Some(old_action) = self.shortcuts.remove(&shortcut) {
            self.actions.remove(&old_action);
        }

        self.shortcuts.insert(shortcut.clone(), action.clone());
        self.actions.insert(action, shortcut);
    }

    /// Bind a shortcut from a string description
    pub fn bind_str(&mut self, shortcut_str: &str, action: Action) {
        if let Some(shortcut) = parse_shortcut(shortcut_str) {
            self.bind(shortcut, action);
        }
    }

    /// Get the action for a shortcut
    pub fn get_action(&self, shortcut: &Shortcut) -> Option<&Action> {
        self.shortcuts.get(shortcut)
    }

    /// Get the shortcut for an action
    pub fn get_shortcut(&self, action: &Action) -> Option<&Shortcut> {
        self.actions.get(action)
    }

    /// Get shortcut string for display
    pub fn shortcut_string(&self, action: &Action) -> Option<String> {
        self.actions.get(action).map(format_shortcut)
    }

    /// Try to match an event and return the action
    pub fn match_event(&self, key: KeyCode, modifiers: Modifiers) -> Option<&Action> {
        let shortcut = Shortcut::new(key, modifiers);
        self.shortcuts.get(&shortcut)
    }
}

impl Default for ShortcutManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a shortcut string like "Ctrl+Shift+T"
pub fn parse_shortcut(s: &str) -> Option<Shortcut> {
    let parts: Vec<&str> = s.split('+').map(|p| p.trim()).collect();
    if parts.is_empty() {
        return None;
    }

    let mut modifiers = Modifiers::empty();
    let mut key = None;

    for part in parts {
        match part.to_lowercase().as_str() {
            "ctrl" | "control" => modifiers.insert(Modifiers::CTRL),
            "shift" => modifiers.insert(Modifiers::SHIFT),
            "alt" => modifiers.insert(Modifiers::ALT),
            "super" | "meta" | "cmd" | "win" => modifiers.insert(Modifiers::SUPER),
            other => {
                key = parse_key(other);
            }
        }
    }

    key.map(|k| Shortcut::new(k, modifiers))
}

/// Parse a key name
fn parse_key(s: &str) -> Option<KeyCode> {
    match s.to_lowercase().as_str() {
        // Letters
        "a" => Some(KeyCode::A),
        "b" => Some(KeyCode::B),
        "c" => Some(KeyCode::C),
        "d" => Some(KeyCode::D),
        "e" => Some(KeyCode::E),
        "f" => Some(KeyCode::F),
        "g" => Some(KeyCode::G),
        "h" => Some(KeyCode::H),
        "i" => Some(KeyCode::I),
        "j" => Some(KeyCode::J),
        "k" => Some(KeyCode::K),
        "l" => Some(KeyCode::L),
        "m" => Some(KeyCode::M),
        "n" => Some(KeyCode::N),
        "o" => Some(KeyCode::O),
        "p" => Some(KeyCode::P),
        "q" => Some(KeyCode::Q),
        "r" => Some(KeyCode::R),
        "s" => Some(KeyCode::S),
        "t" => Some(KeyCode::T),
        "u" => Some(KeyCode::U),
        "v" => Some(KeyCode::V),
        "w" => Some(KeyCode::W),
        "x" => Some(KeyCode::X),
        "y" => Some(KeyCode::Y),
        "z" => Some(KeyCode::Z),

        // Numbers
        "0" => Some(KeyCode::Key0),
        "1" => Some(KeyCode::Key1),
        "2" => Some(KeyCode::Key2),
        "3" => Some(KeyCode::Key3),
        "4" => Some(KeyCode::Key4),
        "5" => Some(KeyCode::Key5),
        "6" => Some(KeyCode::Key6),
        "7" => Some(KeyCode::Key7),
        "8" => Some(KeyCode::Key8),
        "9" => Some(KeyCode::Key9),

        // Function keys
        "f1" => Some(KeyCode::F1),
        "f2" => Some(KeyCode::F2),
        "f3" => Some(KeyCode::F3),
        "f4" => Some(KeyCode::F4),
        "f5" => Some(KeyCode::F5),
        "f6" => Some(KeyCode::F6),
        "f7" => Some(KeyCode::F7),
        "f8" => Some(KeyCode::F8),
        "f9" => Some(KeyCode::F9),
        "f10" => Some(KeyCode::F10),
        "f11" => Some(KeyCode::F11),
        "f12" => Some(KeyCode::F12),

        // Navigation
        "up" => Some(KeyCode::Up),
        "down" => Some(KeyCode::Down),
        "left" => Some(KeyCode::Left),
        "right" => Some(KeyCode::Right),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        "pageup" => Some(KeyCode::PageUp),
        "pagedown" => Some(KeyCode::PageDown),

        // Editing
        "insert" => Some(KeyCode::Insert),
        "delete" | "del" => Some(KeyCode::Delete),
        "backspace" => Some(KeyCode::Backspace),
        "enter" | "return" => Some(KeyCode::Enter),
        "tab" => Some(KeyCode::Tab),
        "escape" | "esc" => Some(KeyCode::Escape),
        "space" => Some(KeyCode::Space),

        // Punctuation
        "minus" | "-" => Some(KeyCode::Minus),
        "equals" | "=" | "plus" => Some(KeyCode::Equals),
        "comma" | "," => Some(KeyCode::Comma),
        "period" | "." => Some(KeyCode::Period),
        "slash" | "/" => Some(KeyCode::Slash),
        "backslash" | "\\" => Some(KeyCode::Backslash),
        "semicolon" | ";" => Some(KeyCode::Semicolon),
        "quote" | "'" => Some(KeyCode::Quote),
        "bracketleft" | "[" => Some(KeyCode::LeftBracket),
        "bracketright" | "]" => Some(KeyCode::RightBracket),
        "grave" | "`" => Some(KeyCode::Backquote),

        _ => None,
    }
}

/// Format a shortcut for display
pub fn format_shortcut(shortcut: &Shortcut) -> String {
    let mut parts = Vec::new();

    if shortcut.modifiers.contains(Modifiers::CTRL) {
        parts.push("Ctrl");
    }
    if shortcut.modifiers.contains(Modifiers::ALT) {
        parts.push("Alt");
    }
    if shortcut.modifiers.contains(Modifiers::SHIFT) {
        parts.push("Shift");
    }
    if shortcut.modifiers.contains(Modifiers::SUPER) {
        parts.push("Super");
    }

    parts.push(format_key(&shortcut.key));

    parts.join("+")
}

/// Format a key for display
fn format_key(key: &KeyCode) -> &'static str {
    match key {
        KeyCode::A => "A",
        KeyCode::B => "B",
        KeyCode::C => "C",
        KeyCode::D => "D",
        KeyCode::E => "E",
        KeyCode::F => "F",
        KeyCode::G => "G",
        KeyCode::H => "H",
        KeyCode::I => "I",
        KeyCode::J => "J",
        KeyCode::K => "K",
        KeyCode::L => "L",
        KeyCode::M => "M",
        KeyCode::N => "N",
        KeyCode::O => "O",
        KeyCode::P => "P",
        KeyCode::Q => "Q",
        KeyCode::R => "R",
        KeyCode::S => "S",
        KeyCode::T => "T",
        KeyCode::U => "U",
        KeyCode::V => "V",
        KeyCode::W => "W",
        KeyCode::X => "X",
        KeyCode::Y => "Y",
        KeyCode::Z => "Z",
        KeyCode::Key0 => "0",
        KeyCode::Key1 => "1",
        KeyCode::Key2 => "2",
        KeyCode::Key3 => "3",
        KeyCode::Key4 => "4",
        KeyCode::Key5 => "5",
        KeyCode::Key6 => "6",
        KeyCode::Key7 => "7",
        KeyCode::Key8 => "8",
        KeyCode::Key9 => "9",
        KeyCode::F1 => "F1",
        KeyCode::F2 => "F2",
        KeyCode::F3 => "F3",
        KeyCode::F4 => "F4",
        KeyCode::F5 => "F5",
        KeyCode::F6 => "F6",
        KeyCode::F7 => "F7",
        KeyCode::F8 => "F8",
        KeyCode::F9 => "F9",
        KeyCode::F10 => "F10",
        KeyCode::F11 => "F11",
        KeyCode::F12 => "F12",
        KeyCode::Up => "Up",
        KeyCode::Down => "Down",
        KeyCode::Left => "Left",
        KeyCode::Right => "Right",
        KeyCode::Home => "Home",
        KeyCode::End => "End",
        KeyCode::PageUp => "PageUp",
        KeyCode::PageDown => "PageDown",
        KeyCode::Insert => "Insert",
        KeyCode::Delete => "Delete",
        KeyCode::Backspace => "Backspace",
        KeyCode::Enter => "Enter",
        KeyCode::Tab => "Tab",
        KeyCode::Escape => "Escape",
        KeyCode::Space => "Space",
        KeyCode::Minus => "-",
        KeyCode::Equals => "=",
        KeyCode::Comma => ",",
        KeyCode::Period => ".",
        KeyCode::Slash => "/",
        KeyCode::Backslash => "\\",
        KeyCode::Semicolon => ";",
        KeyCode::Quote => "'",
        KeyCode::LeftBracket => "[",
        KeyCode::RightBracket => "]",
        KeyCode::Backquote => "`",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_shortcut() {
        let shortcut = parse_shortcut("Ctrl+Shift+T").unwrap();
        assert_eq!(shortcut.key, KeyCode::T);
        assert!(shortcut.modifiers.contains(Modifiers::CTRL));
        assert!(shortcut.modifiers.contains(Modifiers::SHIFT));
    }

    #[test]
    fn test_format_shortcut() {
        let shortcut = Shortcut::ctrl_shift(KeyCode::T);
        let formatted = format_shortcut(&shortcut);
        assert_eq!(formatted, "Ctrl+Shift+T");
    }

    #[test]
    fn test_shortcut_manager() {
        let manager = ShortcutManager::new();
        let action = manager.match_event(KeyCode::T, Modifiers::CTRL | Modifiers::SHIFT);
        assert_eq!(action, Some(&Action::NewTab));
    }
}
