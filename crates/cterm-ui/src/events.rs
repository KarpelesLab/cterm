//! Input events abstraction
//!
//! Defines platform-agnostic input events.

use bitflags::bitflags;

bitflags! {
    /// Keyboard modifiers
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
    pub struct Modifiers: u8 {
        const SHIFT = 1 << 0;
        const CTRL = 1 << 1;
        const ALT = 1 << 2;
        const SUPER = 1 << 3;
    }
}

/// Keyboard key codes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyCode {
    // Letters
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,

    // Numbers
    Key0,
    Key1,
    Key2,
    Key3,
    Key4,
    Key5,
    Key6,
    Key7,
    Key8,
    Key9,

    // Function keys
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,

    // Navigation
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,

    // Editing
    Insert,
    Delete,
    Backspace,
    Enter,
    Tab,

    // Modifiers (as keys)
    Escape,
    Space,

    // Punctuation
    Minus,
    Equals,
    LeftBracket,
    RightBracket,
    Semicolon,
    Quote,
    Backquote,
    Backslash,
    Comma,
    Period,
    Slash,

    // Numpad
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
    NumpadAdd,
    NumpadSubtract,
    NumpadMultiply,
    NumpadDivide,
    NumpadDecimal,
    NumpadEnter,

    // Other
    PrintScreen,
    ScrollLock,
    Pause,
    CapsLock,
    NumLock,

    /// Unknown key
    Unknown,
}

/// Mouse button
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Back,
    Forward,
}

/// Scroll direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Input event types
#[derive(Debug, Clone)]
pub enum InputEvent {
    /// Key was pressed
    KeyPress {
        key: KeyCode,
        modifiers: Modifiers,
        /// Character representation if available
        text: Option<String>,
    },

    /// Key was released
    KeyRelease { key: KeyCode, modifiers: Modifiers },

    /// Mouse button pressed
    MousePress {
        button: MouseButton,
        x: f64,
        y: f64,
        modifiers: Modifiers,
    },

    /// Mouse button released
    MouseRelease {
        button: MouseButton,
        x: f64,
        y: f64,
        modifiers: Modifiers,
    },

    /// Mouse moved
    MouseMove {
        x: f64,
        y: f64,
        modifiers: Modifiers,
    },

    /// Mouse scroll
    Scroll {
        direction: ScrollDirection,
        delta: f64,
        x: f64,
        y: f64,
        modifiers: Modifiers,
    },

    /// Window focus gained
    FocusIn,

    /// Window focus lost
    FocusOut,

    /// Window resized
    Resize { width: f64, height: f64 },

    /// Paste from clipboard
    Paste(String),
}

impl KeyCode {
    /// Get the character for this key (without modifiers)
    pub fn to_char(&self) -> Option<char> {
        match self {
            Self::A => Some('a'),
            Self::B => Some('b'),
            Self::C => Some('c'),
            Self::D => Some('d'),
            Self::E => Some('e'),
            Self::F => Some('f'),
            Self::G => Some('g'),
            Self::H => Some('h'),
            Self::I => Some('i'),
            Self::J => Some('j'),
            Self::K => Some('k'),
            Self::L => Some('l'),
            Self::M => Some('m'),
            Self::N => Some('n'),
            Self::O => Some('o'),
            Self::P => Some('p'),
            Self::Q => Some('q'),
            Self::R => Some('r'),
            Self::S => Some('s'),
            Self::T => Some('t'),
            Self::U => Some('u'),
            Self::V => Some('v'),
            Self::W => Some('w'),
            Self::X => Some('x'),
            Self::Y => Some('y'),
            Self::Z => Some('z'),
            Self::Key0 => Some('0'),
            Self::Key1 => Some('1'),
            Self::Key2 => Some('2'),
            Self::Key3 => Some('3'),
            Self::Key4 => Some('4'),
            Self::Key5 => Some('5'),
            Self::Key6 => Some('6'),
            Self::Key7 => Some('7'),
            Self::Key8 => Some('8'),
            Self::Key9 => Some('9'),
            Self::Space => Some(' '),
            Self::Minus => Some('-'),
            Self::Equals => Some('='),
            Self::LeftBracket => Some('['),
            Self::RightBracket => Some(']'),
            Self::Semicolon => Some(';'),
            Self::Quote => Some('\''),
            Self::Backquote => Some('`'),
            Self::Backslash => Some('\\'),
            Self::Comma => Some(','),
            Self::Period => Some('.'),
            Self::Slash => Some('/'),
            _ => None,
        }
    }

    /// Get the shifted character for this key
    pub fn to_shifted_char(&self) -> Option<char> {
        match self {
            Self::A => Some('A'),
            Self::B => Some('B'),
            Self::C => Some('C'),
            Self::D => Some('D'),
            Self::E => Some('E'),
            Self::F => Some('F'),
            Self::G => Some('G'),
            Self::H => Some('H'),
            Self::I => Some('I'),
            Self::J => Some('J'),
            Self::K => Some('K'),
            Self::L => Some('L'),
            Self::M => Some('M'),
            Self::N => Some('N'),
            Self::O => Some('O'),
            Self::P => Some('P'),
            Self::Q => Some('Q'),
            Self::R => Some('R'),
            Self::S => Some('S'),
            Self::T => Some('T'),
            Self::U => Some('U'),
            Self::V => Some('V'),
            Self::W => Some('W'),
            Self::X => Some('X'),
            Self::Y => Some('Y'),
            Self::Z => Some('Z'),
            Self::Key0 => Some(')'),
            Self::Key1 => Some('!'),
            Self::Key2 => Some('@'),
            Self::Key3 => Some('#'),
            Self::Key4 => Some('$'),
            Self::Key5 => Some('%'),
            Self::Key6 => Some('^'),
            Self::Key7 => Some('&'),
            Self::Key8 => Some('*'),
            Self::Key9 => Some('('),
            Self::Space => Some(' '),
            Self::Minus => Some('_'),
            Self::Equals => Some('+'),
            Self::LeftBracket => Some('{'),
            Self::RightBracket => Some('}'),
            Self::Semicolon => Some(':'),
            Self::Quote => Some('"'),
            Self::Backquote => Some('~'),
            Self::Backslash => Some('|'),
            Self::Comma => Some('<'),
            Self::Period => Some('>'),
            Self::Slash => Some('?'),
            _ => None,
        }
    }
}

/// Action that can be bound to a shortcut
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Action {
    // Tab actions
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    NextAlertedTab,
    Tab(u8), // Tab 1-9

    // Window actions
    NewWindow,
    CloseWindow,

    // Edit actions
    Copy,
    Paste,
    SelectAll,

    // View actions
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleFullscreen,

    // Scroll actions
    ScrollUp,
    ScrollDown,
    ScrollPageUp,
    ScrollPageDown,
    ScrollToTop,
    ScrollToBottom,

    // Other
    OpenPreferences,
    FindText,
    ResetTerminal,
    QuickOpenTemplate,
}

/// A keyboard shortcut
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Shortcut {
    pub key: KeyCode,
    pub modifiers: Modifiers,
}

impl Shortcut {
    pub fn new(key: KeyCode, modifiers: Modifiers) -> Self {
        Self { key, modifiers }
    }

    /// Create a shortcut with Ctrl modifier
    pub fn ctrl(key: KeyCode) -> Self {
        Self::new(key, Modifiers::CTRL)
    }

    /// Create a shortcut with Ctrl+Shift modifiers
    pub fn ctrl_shift(key: KeyCode) -> Self {
        Self::new(key, Modifiers::CTRL | Modifiers::SHIFT)
    }

    /// Check if an input event matches this shortcut
    pub fn matches(&self, event: &InputEvent) -> bool {
        match event {
            InputEvent::KeyPress { key, modifiers, .. } => {
                *key == self.key && *modifiers == self.modifiers
            }
            _ => false,
        }
    }
}
