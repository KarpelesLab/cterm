//! Keycode conversion for macOS
//!
//! Maps macOS virtual key codes to cterm-ui KeyCode enum.

use cterm_ui::events::{KeyCode, Modifiers};
use objc2_app_kit::{NSEvent, NSEventModifierFlags};

/// Convert NSEvent modifier flags to our Modifiers
pub fn modifiers_from_event(event: &NSEvent) -> Modifiers {
    let flags = event.modifierFlags();
    let mut modifiers = Modifiers::empty();

    if flags.contains(NSEventModifierFlags::Shift) {
        modifiers.insert(Modifiers::SHIFT);
    }
    if flags.contains(NSEventModifierFlags::Control) {
        modifiers.insert(Modifiers::CTRL);
    }
    if flags.contains(NSEventModifierFlags::Option) {
        modifiers.insert(Modifiers::ALT);
    }
    if flags.contains(NSEventModifierFlags::Command) {
        modifiers.insert(Modifiers::SUPER);
    }

    modifiers
}

/// Convert macOS virtual key code to our KeyCode
pub fn keycode_from_event(event: &NSEvent) -> Option<KeyCode> {
    let keycode = event.keyCode();

    // macOS virtual key codes (from Carbon HIToolbox/Events.h)
    Some(match keycode {
        // Letters (QWERTY layout key positions)
        0x00 => KeyCode::A,
        0x0B => KeyCode::B,
        0x08 => KeyCode::C,
        0x02 => KeyCode::D,
        0x0E => KeyCode::E,
        0x03 => KeyCode::F,
        0x05 => KeyCode::G,
        0x04 => KeyCode::H,
        0x22 => KeyCode::I,
        0x26 => KeyCode::J,
        0x28 => KeyCode::K,
        0x25 => KeyCode::L,
        0x2E => KeyCode::M,
        0x2D => KeyCode::N,
        0x1F => KeyCode::O,
        0x23 => KeyCode::P,
        0x0C => KeyCode::Q,
        0x0F => KeyCode::R,
        0x01 => KeyCode::S,
        0x11 => KeyCode::T,
        0x20 => KeyCode::U,
        0x09 => KeyCode::V,
        0x0D => KeyCode::W,
        0x07 => KeyCode::X,
        0x10 => KeyCode::Y,
        0x06 => KeyCode::Z,

        // Numbers
        0x1D => KeyCode::Key0,
        0x12 => KeyCode::Key1,
        0x13 => KeyCode::Key2,
        0x14 => KeyCode::Key3,
        0x15 => KeyCode::Key4,
        0x17 => KeyCode::Key5,
        0x16 => KeyCode::Key6,
        0x1A => KeyCode::Key7,
        0x1C => KeyCode::Key8,
        0x19 => KeyCode::Key9,

        // Function keys
        0x7A => KeyCode::F1,
        0x78 => KeyCode::F2,
        0x63 => KeyCode::F3,
        0x76 => KeyCode::F4,
        0x60 => KeyCode::F5,
        0x61 => KeyCode::F6,
        0x62 => KeyCode::F7,
        0x64 => KeyCode::F8,
        0x65 => KeyCode::F9,
        0x6D => KeyCode::F10,
        0x67 => KeyCode::F11,
        0x6F => KeyCode::F12,

        // Navigation
        0x7E => KeyCode::Up,
        0x7D => KeyCode::Down,
        0x7B => KeyCode::Left,
        0x7C => KeyCode::Right,
        0x73 => KeyCode::Home,
        0x77 => KeyCode::End,
        0x74 => KeyCode::PageUp,
        0x79 => KeyCode::PageDown,

        // Editing
        0x72 => KeyCode::Insert, // Help key on Mac, often mapped to Insert
        0x75 => KeyCode::Delete, // Forward delete
        0x33 => KeyCode::Backspace,
        0x24 => KeyCode::Enter,
        0x30 => KeyCode::Tab,

        // Special
        0x35 => KeyCode::Escape,
        0x31 => KeyCode::Space,

        // Punctuation
        0x1B => KeyCode::Minus,
        0x18 => KeyCode::Equals,
        0x21 => KeyCode::LeftBracket,
        0x1E => KeyCode::RightBracket,
        0x29 => KeyCode::Semicolon,
        0x27 => KeyCode::Quote,
        0x32 => KeyCode::Backquote,
        0x2A => KeyCode::Backslash,
        0x2B => KeyCode::Comma,
        0x2F => KeyCode::Period,
        0x2C => KeyCode::Slash,

        // Numpad
        0x52 => KeyCode::Numpad0,
        0x53 => KeyCode::Numpad1,
        0x54 => KeyCode::Numpad2,
        0x55 => KeyCode::Numpad3,
        0x56 => KeyCode::Numpad4,
        0x57 => KeyCode::Numpad5,
        0x58 => KeyCode::Numpad6,
        0x59 => KeyCode::Numpad7,
        0x5B => KeyCode::Numpad8,
        0x5C => KeyCode::Numpad9,
        0x45 => KeyCode::NumpadAdd,
        0x4E => KeyCode::NumpadSubtract,
        0x43 => KeyCode::NumpadMultiply,
        0x4B => KeyCode::NumpadDivide,
        0x41 => KeyCode::NumpadDecimal,
        0x4C => KeyCode::NumpadEnter,

        _ => return None,
    })
}

/// Get the character string from an NSEvent
pub fn characters_from_event(event: &NSEvent) -> Option<String> {
    event.characters().map(|s| s.to_string())
}

/// Get the character string ignoring modifiers (useful for Ctrl+key combinations)
pub fn characters_ignoring_modifiers(event: &NSEvent) -> Option<String> {
    event
        .charactersIgnoringModifiers()
        .map(|s| s.to_string())
}
