//! Virtual key code to KeyCode conversion
//!
//! Maps Windows virtual key codes to cterm-ui KeyCode values.

use cterm_ui::events::{KeyCode, Modifiers};
use winapi::um::winuser;

/// Convert a Windows virtual key code to our KeyCode
pub fn vk_to_keycode(vk: u16) -> Option<KeyCode> {
    Some(match vk as i32 {
        // Letters (VK_A through VK_Z are 0x41-0x5A, same as ASCII)
        0x41 => KeyCode::A,
        0x42 => KeyCode::B,
        0x43 => KeyCode::C,
        0x44 => KeyCode::D,
        0x45 => KeyCode::E,
        0x46 => KeyCode::F,
        0x47 => KeyCode::G,
        0x48 => KeyCode::H,
        0x49 => KeyCode::I,
        0x4A => KeyCode::J,
        0x4B => KeyCode::K,
        0x4C => KeyCode::L,
        0x4D => KeyCode::M,
        0x4E => KeyCode::N,
        0x4F => KeyCode::O,
        0x50 => KeyCode::P,
        0x51 => KeyCode::Q,
        0x52 => KeyCode::R,
        0x53 => KeyCode::S,
        0x54 => KeyCode::T,
        0x55 => KeyCode::U,
        0x56 => KeyCode::V,
        0x57 => KeyCode::W,
        0x58 => KeyCode::X,
        0x59 => KeyCode::Y,
        0x5A => KeyCode::Z,

        // Numbers (top row)
        0x30 => KeyCode::Key0,
        0x31 => KeyCode::Key1,
        0x32 => KeyCode::Key2,
        0x33 => KeyCode::Key3,
        0x34 => KeyCode::Key4,
        0x35 => KeyCode::Key5,
        0x36 => KeyCode::Key6,
        0x37 => KeyCode::Key7,
        0x38 => KeyCode::Key8,
        0x39 => KeyCode::Key9,

        // Function keys
        winuser::VK_F1 => KeyCode::F1,
        winuser::VK_F2 => KeyCode::F2,
        winuser::VK_F3 => KeyCode::F3,
        winuser::VK_F4 => KeyCode::F4,
        winuser::VK_F5 => KeyCode::F5,
        winuser::VK_F6 => KeyCode::F6,
        winuser::VK_F7 => KeyCode::F7,
        winuser::VK_F8 => KeyCode::F8,
        winuser::VK_F9 => KeyCode::F9,
        winuser::VK_F10 => KeyCode::F10,
        winuser::VK_F11 => KeyCode::F11,
        winuser::VK_F12 => KeyCode::F12,

        // Navigation
        winuser::VK_UP => KeyCode::Up,
        winuser::VK_DOWN => KeyCode::Down,
        winuser::VK_LEFT => KeyCode::Left,
        winuser::VK_RIGHT => KeyCode::Right,
        winuser::VK_HOME => KeyCode::Home,
        winuser::VK_END => KeyCode::End,
        winuser::VK_PRIOR => KeyCode::PageUp,
        winuser::VK_NEXT => KeyCode::PageDown,

        // Editing
        winuser::VK_INSERT => KeyCode::Insert,
        winuser::VK_DELETE => KeyCode::Delete,
        winuser::VK_BACK => KeyCode::Backspace,
        winuser::VK_RETURN => KeyCode::Enter,
        winuser::VK_TAB => KeyCode::Tab,
        winuser::VK_ESCAPE => KeyCode::Escape,
        winuser::VK_SPACE => KeyCode::Space,

        // Punctuation
        winuser::VK_OEM_MINUS => KeyCode::Minus,
        winuser::VK_OEM_PLUS => KeyCode::Equals,
        winuser::VK_OEM_4 => KeyCode::LeftBracket,  // [
        winuser::VK_OEM_6 => KeyCode::RightBracket, // ]
        winuser::VK_OEM_1 => KeyCode::Semicolon,    // ;
        winuser::VK_OEM_7 => KeyCode::Quote,        // '
        winuser::VK_OEM_3 => KeyCode::Backquote,    // `
        winuser::VK_OEM_5 => KeyCode::Backslash,    // \
        winuser::VK_OEM_COMMA => KeyCode::Comma,
        winuser::VK_OEM_PERIOD => KeyCode::Period,
        winuser::VK_OEM_2 => KeyCode::Slash, // /

        // Numpad
        winuser::VK_NUMPAD0 => KeyCode::Numpad0,
        winuser::VK_NUMPAD1 => KeyCode::Numpad1,
        winuser::VK_NUMPAD2 => KeyCode::Numpad2,
        winuser::VK_NUMPAD3 => KeyCode::Numpad3,
        winuser::VK_NUMPAD4 => KeyCode::Numpad4,
        winuser::VK_NUMPAD5 => KeyCode::Numpad5,
        winuser::VK_NUMPAD6 => KeyCode::Numpad6,
        winuser::VK_NUMPAD7 => KeyCode::Numpad7,
        winuser::VK_NUMPAD8 => KeyCode::Numpad8,
        winuser::VK_NUMPAD9 => KeyCode::Numpad9,
        winuser::VK_ADD => KeyCode::NumpadAdd,
        winuser::VK_SUBTRACT => KeyCode::NumpadSubtract,
        winuser::VK_MULTIPLY => KeyCode::NumpadMultiply,
        winuser::VK_DIVIDE => KeyCode::NumpadDivide,
        winuser::VK_DECIMAL => KeyCode::NumpadDecimal,

        // Lock keys
        winuser::VK_SNAPSHOT => KeyCode::PrintScreen,
        winuser::VK_SCROLL => KeyCode::ScrollLock,
        winuser::VK_PAUSE => KeyCode::Pause,
        winuser::VK_CAPITAL => KeyCode::CapsLock,
        winuser::VK_NUMLOCK => KeyCode::NumLock,

        _ => return None,
    })
}

/// Get the current keyboard modifiers
pub fn get_modifiers() -> Modifiers {
    let mut mods = Modifiers::empty();

    // Check key states using GetKeyState
    // High bit (0x8000) indicates key is down
    unsafe {
        if winapi::um::winuser::GetKeyState(winuser::VK_CONTROL) & 0x8000u16 as i16 != 0 {
            mods.insert(Modifiers::CTRL);
        }
        if winapi::um::winuser::GetKeyState(winuser::VK_SHIFT) & 0x8000u16 as i16 != 0 {
            mods.insert(Modifiers::SHIFT);
        }
        if winapi::um::winuser::GetKeyState(winuser::VK_MENU) & 0x8000u16 as i16 != 0 {
            mods.insert(Modifiers::ALT);
        }
        if winapi::um::winuser::GetKeyState(winuser::VK_LWIN) & 0x8000u16 as i16 != 0
            || winapi::um::winuser::GetKeyState(winuser::VK_RWIN) & 0x8000u16 as i16 != 0
        {
            mods.insert(Modifiers::SUPER);
        }
    }

    mods
}

/// Convert virtual key to terminal escape sequence for special keys
pub fn vk_to_terminal_seq(
    vk: u16,
    modifiers: Modifiers,
    application_mode: bool,
) -> Option<&'static str> {
    let has_shift = modifiers.contains(Modifiers::SHIFT);
    let has_ctrl = modifiers.contains(Modifiers::CTRL);
    let has_alt = modifiers.contains(Modifiers::ALT);

    // Calculate modifier parameter for CSI sequences
    // Modifier = 1 + (shift ? 1 : 0) + (alt ? 2 : 0) + (ctrl ? 4 : 0)
    let mod_param = 1
        + if has_shift { 1 } else { 0 }
        + if has_alt { 2 } else { 0 }
        + if has_ctrl { 4 } else { 0 };
    let has_mods = mod_param > 1;

    match vk as i32 {
        // Arrow keys
        winuser::VK_UP => Some(if application_mode && !has_mods {
            "\x1bOA"
        } else if has_mods {
            match mod_param {
                2 => "\x1b[1;2A", // Shift
                3 => "\x1b[1;3A", // Alt
                4 => "\x1b[1;4A", // Shift+Alt
                5 => "\x1b[1;5A", // Ctrl
                6 => "\x1b[1;6A", // Ctrl+Shift
                7 => "\x1b[1;7A", // Ctrl+Alt
                8 => "\x1b[1;8A", // Ctrl+Alt+Shift
                _ => "\x1b[A",
            }
        } else {
            "\x1b[A"
        }),
        winuser::VK_DOWN => Some(if application_mode && !has_mods {
            "\x1bOB"
        } else if has_mods {
            match mod_param {
                2 => "\x1b[1;2B",
                3 => "\x1b[1;3B",
                4 => "\x1b[1;4B",
                5 => "\x1b[1;5B",
                6 => "\x1b[1;6B",
                7 => "\x1b[1;7B",
                8 => "\x1b[1;8B",
                _ => "\x1b[B",
            }
        } else {
            "\x1b[B"
        }),
        winuser::VK_RIGHT => Some(if application_mode && !has_mods {
            "\x1bOC"
        } else if has_mods {
            match mod_param {
                2 => "\x1b[1;2C",
                3 => "\x1b[1;3C",
                4 => "\x1b[1;4C",
                5 => "\x1b[1;5C",
                6 => "\x1b[1;6C",
                7 => "\x1b[1;7C",
                8 => "\x1b[1;8C",
                _ => "\x1b[C",
            }
        } else {
            "\x1b[C"
        }),
        winuser::VK_LEFT => Some(if application_mode && !has_mods {
            "\x1bOD"
        } else if has_mods {
            match mod_param {
                2 => "\x1b[1;2D",
                3 => "\x1b[1;3D",
                4 => "\x1b[1;4D",
                5 => "\x1b[1;5D",
                6 => "\x1b[1;6D",
                7 => "\x1b[1;7D",
                8 => "\x1b[1;8D",
                _ => "\x1b[D",
            }
        } else {
            "\x1b[D"
        }),

        // Navigation keys
        winuser::VK_HOME => Some("\x1b[H"),
        winuser::VK_END => Some("\x1b[F"),
        winuser::VK_PRIOR => Some("\x1b[5~"), // Page Up
        winuser::VK_NEXT => Some("\x1b[6~"),  // Page Down
        winuser::VK_INSERT => Some("\x1b[2~"),
        winuser::VK_DELETE => Some("\x1b[3~"),

        // Function keys
        winuser::VK_F1 => Some("\x1bOP"),
        winuser::VK_F2 => Some("\x1bOQ"),
        winuser::VK_F3 => Some("\x1bOR"),
        winuser::VK_F4 => Some("\x1bOS"),
        winuser::VK_F5 => Some("\x1b[15~"),
        winuser::VK_F6 => Some("\x1b[17~"),
        winuser::VK_F7 => Some("\x1b[18~"),
        winuser::VK_F8 => Some("\x1b[19~"),
        winuser::VK_F9 => Some("\x1b[20~"),
        winuser::VK_F10 => Some("\x1b[21~"),
        winuser::VK_F11 => Some("\x1b[23~"),
        winuser::VK_F12 => Some("\x1b[24~"),

        // Tab
        winuser::VK_TAB => {
            if has_shift {
                Some("\x1b[Z") // Shift+Tab (backtab)
            } else {
                Some("\t")
            }
        }

        // Backspace
        winuser::VK_BACK => {
            if has_alt {
                Some("\x1b\x7f")
            } else {
                Some("\x7f")
            }
        }

        // Enter
        winuser::VK_RETURN => {
            if has_alt {
                Some("\x1b\r")
            } else {
                Some("\r")
            }
        }

        // Escape
        winuser::VK_ESCAPE => Some("\x1b"),

        _ => None,
    }
}

/// Check if a virtual key is a modifier key
pub fn is_modifier_key(vk: u16) -> bool {
    matches!(
        vk as i32,
        winuser::VK_SHIFT
            | winuser::VK_CONTROL
            | winuser::VK_MENU
            | winuser::VK_LSHIFT
            | winuser::VK_RSHIFT
            | winuser::VK_LCONTROL
            | winuser::VK_RCONTROL
            | winuser::VK_LMENU
            | winuser::VK_RMENU
            | winuser::VK_LWIN
            | winuser::VK_RWIN
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vk_to_keycode_letters() {
        assert_eq!(vk_to_keycode(0x41), Some(KeyCode::A)); // VK_A
        assert_eq!(vk_to_keycode(0x5A), Some(KeyCode::Z)); // VK_Z
    }

    #[test]
    fn test_vk_to_keycode_numbers() {
        assert_eq!(vk_to_keycode(0x30), Some(KeyCode::Key0));
        assert_eq!(vk_to_keycode(0x39), Some(KeyCode::Key9));
    }

    #[test]
    fn test_is_modifier_key() {
        assert!(is_modifier_key(winuser::VK_SHIFT as u16));
        assert!(is_modifier_key(winuser::VK_CONTROL as u16));
        assert!(!is_modifier_key(0x41)); // VK_A
    }
}
