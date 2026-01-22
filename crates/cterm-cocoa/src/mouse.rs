//! Mouse reporting utilities for terminal applications
//!
//! Implements xterm-style mouse reporting escape sequences for applications
//! that request mouse events (vim, tmux, htop, etc.)

use cterm_core::screen::MouseMode;

/// Mouse button types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    /// Release event (no button)
    Release,
    /// Scroll up
    WheelUp,
    /// Scroll down
    WheelDown,
}

/// Modifier keys held during mouse event
#[derive(Debug, Clone, Copy, Default)]
pub struct MouseModifiers {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// Generate mouse event escape sequence
///
/// Returns the escape sequence to send to the PTY, or None if mouse reporting
/// is not active for this event type.
pub fn encode_mouse_event(
    mode: MouseMode,
    sgr_encoding: bool,
    button: MouseButton,
    col: usize,
    row: usize,
    modifiers: MouseModifiers,
    is_drag: bool,
) -> Option<Vec<u8>> {
    // Check if this event type should be reported based on mode
    match mode {
        MouseMode::None => return None,
        MouseMode::X10 => {
            // X10 only reports button presses (not releases, drags, or wheel)
            if matches!(button, MouseButton::Release) || is_drag {
                return None;
            }
            if matches!(button, MouseButton::WheelUp | MouseButton::WheelDown) {
                return None;
            }
        }
        MouseMode::Normal => {
            // Normal reports presses and releases, but not motion
            if is_drag {
                return None;
            }
        }
        MouseMode::ButtonEvent => {
            // Button event reports presses, releases, and dragging with button held
            // Motion without button is not reported
        }
        MouseMode::AnyEvent => {
            // Any event reports everything including motion
        }
    }

    // Calculate button code
    let button_code = match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::Release => 3,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    };

    // Add modifier bits
    let mut code = button_code;
    if modifiers.shift {
        code |= 4;
    }
    if modifiers.alt {
        code |= 8;
    }
    if modifiers.ctrl {
        code |= 16;
    }
    // Drag bit (motion with button held)
    if is_drag && !matches!(button, MouseButton::WheelUp | MouseButton::WheelDown) {
        code |= 32;
    }

    if sgr_encoding {
        // SGR encoding: CSI < button ; col ; row M (press) or m (release)
        let suffix = if matches!(button, MouseButton::Release) {
            'm'
        } else {
            'M'
        };
        // SGR uses 1-based coordinates
        Some(format!("\x1b[<{};{};{}{}", code, col + 1, row + 1, suffix).into_bytes())
    } else {
        // X10/Normal encoding: CSI M button col row
        // Coordinates are encoded as (value + 32) to make them printable ASCII
        // This limits coordinates to 223 (255 - 32)
        let col_byte = ((col.min(222) + 1) + 32) as u8;
        let row_byte = ((row.min(222) + 1) + 32) as u8;
        let button_byte = (code + 32) as u8;

        Some(vec![0x1b, b'[', b'M', button_byte, col_byte, row_byte])
    }
}

/// Check if mouse events should be captured (not used for selection)
pub fn should_capture_mouse(mode: MouseMode) -> bool {
    !matches!(mode, MouseMode::None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sgr_encoding() {
        let seq = encode_mouse_event(
            MouseMode::Normal,
            true,
            MouseButton::Left,
            10,
            5,
            MouseModifiers::default(),
            false,
        );
        assert_eq!(seq, Some(b"\x1b[<0;11;6M".to_vec()));
    }

    #[test]
    fn test_sgr_release() {
        let seq = encode_mouse_event(
            MouseMode::Normal,
            true,
            MouseButton::Release,
            10,
            5,
            MouseModifiers::default(),
            false,
        );
        assert_eq!(seq, Some(b"\x1b[<3;11;6m".to_vec()));
    }

    #[test]
    fn test_x10_encoding() {
        let seq = encode_mouse_event(
            MouseMode::Normal,
            false,
            MouseButton::Left,
            10,
            5,
            MouseModifiers::default(),
            false,
        );
        // button=32, col=10+1+32=43, row=5+1+32=38
        assert_eq!(seq, Some(vec![0x1b, b'[', b'M', 32, 43, 38]));
    }

    #[test]
    fn test_x10_mode_no_release() {
        let seq = encode_mouse_event(
            MouseMode::X10,
            false,
            MouseButton::Release,
            10,
            5,
            MouseModifiers::default(),
            false,
        );
        assert_eq!(seq, None);
    }
}
