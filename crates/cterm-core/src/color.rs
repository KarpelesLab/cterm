//! Color types for terminal cells
//!
//! Supports:
//! - 16 basic ANSI colors (with bright variants)
//! - 256 indexed colors
//! - 24-bit true color (RGB)

use serde::{Deserialize, Serialize};

/// RGB color value
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parse from hex string like "#RRGGBB" or "RRGGBB"
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.strip_prefix('#').unwrap_or(hex);
        if hex.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some(Self { r, g, b })
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Convert to normalized float values (0.0-1.0)
    pub fn to_f64(&self) -> (f64, f64, f64) {
        (
            self.r as f64 / 255.0,
            self.g as f64 / 255.0,
            self.b as f64 / 255.0,
        )
    }
}

/// Standard ANSI colors (0-15)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum AnsiColor {
    Black = 0,
    Red = 1,
    Green = 2,
    Yellow = 3,
    Blue = 4,
    Magenta = 5,
    Cyan = 6,
    White = 7,
    BrightBlack = 8,
    BrightRed = 9,
    BrightGreen = 10,
    BrightYellow = 11,
    BrightBlue = 12,
    BrightMagenta = 13,
    BrightCyan = 14,
    BrightWhite = 15,
}

impl AnsiColor {
    /// Get the bright variant of a base color
    pub fn bright(self) -> Self {
        match self {
            Self::Black => Self::BrightBlack,
            Self::Red => Self::BrightRed,
            Self::Green => Self::BrightGreen,
            Self::Yellow => Self::BrightYellow,
            Self::Blue => Self::BrightBlue,
            Self::Magenta => Self::BrightMagenta,
            Self::Cyan => Self::BrightCyan,
            Self::White => Self::BrightWhite,
            bright => bright, // Already bright
        }
    }

    /// Get the base (non-bright) variant
    pub fn base(self) -> Self {
        match self {
            Self::BrightBlack => Self::Black,
            Self::BrightRed => Self::Red,
            Self::BrightGreen => Self::Green,
            Self::BrightYellow => Self::Yellow,
            Self::BrightBlue => Self::Blue,
            Self::BrightMagenta => Self::Magenta,
            Self::BrightCyan => Self::Cyan,
            Self::BrightWhite => Self::White,
            base => base,
        }
    }

    pub fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::Black),
            1 => Some(Self::Red),
            2 => Some(Self::Green),
            3 => Some(Self::Yellow),
            4 => Some(Self::Blue),
            5 => Some(Self::Magenta),
            6 => Some(Self::Cyan),
            7 => Some(Self::White),
            8 => Some(Self::BrightBlack),
            9 => Some(Self::BrightRed),
            10 => Some(Self::BrightGreen),
            11 => Some(Self::BrightYellow),
            12 => Some(Self::BrightBlue),
            13 => Some(Self::BrightMagenta),
            14 => Some(Self::BrightCyan),
            15 => Some(Self::BrightWhite),
            _ => None,
        }
    }
}

/// Terminal color specification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Color {
    /// Default foreground/background color
    Default,
    /// One of the 16 ANSI colors
    Ansi(AnsiColor),
    /// 256-color palette index (0-255)
    Indexed(u8),
    /// 24-bit true color
    Rgb(Rgb),
}

impl Default for Color {
    fn default() -> Self {
        Self::Default
    }
}

impl Color {
    /// Create a new RGB color
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::Rgb(Rgb::new(r, g, b))
    }

    /// Convert indexed color to RGB using standard 256-color palette
    pub fn to_rgb(&self, palette: &ColorPalette) -> Rgb {
        match self {
            Self::Default => palette.foreground,
            Self::Ansi(ansi) => palette.ansi[*ansi as usize],
            Self::Indexed(idx) => index_to_rgb(*idx, palette),
            Self::Rgb(rgb) => *rgb,
        }
    }

    /// Check if this is the default color
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }
}

/// Color palette for rendering
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorPalette {
    /// 16 ANSI colors
    pub ansi: [Rgb; 16],
    /// Default foreground color
    pub foreground: Rgb,
    /// Default background color
    pub background: Rgb,
    /// Cursor color
    pub cursor: Rgb,
    /// Selection background color
    pub selection: Rgb,
}

impl Default for ColorPalette {
    fn default() -> Self {
        Self::default_dark()
    }
}

impl ColorPalette {
    /// Default dark theme palette
    pub fn default_dark() -> Self {
        Self {
            ansi: [
                Rgb::new(0x1d, 0x1f, 0x21), // Black
                Rgb::new(0xcc, 0x66, 0x66), // Red
                Rgb::new(0xb5, 0xbd, 0x68), // Green
                Rgb::new(0xf0, 0xc6, 0x74), // Yellow
                Rgb::new(0x81, 0xa2, 0xbe), // Blue
                Rgb::new(0xb2, 0x94, 0xbb), // Magenta
                Rgb::new(0x8a, 0xbe, 0xb7), // Cyan
                Rgb::new(0xc5, 0xc8, 0xc6), // White
                Rgb::new(0x96, 0x98, 0x96), // Bright Black
                Rgb::new(0xde, 0x93, 0x5f), // Bright Red
                Rgb::new(0xb5, 0xbd, 0x68), // Bright Green
                Rgb::new(0xf0, 0xc6, 0x74), // Bright Yellow
                Rgb::new(0x81, 0xa2, 0xbe), // Bright Blue
                Rgb::new(0xb2, 0x94, 0xbb), // Bright Magenta
                Rgb::new(0x8a, 0xbe, 0xb7), // Bright Cyan
                Rgb::new(0xff, 0xff, 0xff), // Bright White
            ],
            foreground: Rgb::new(0xc5, 0xc8, 0xc6),
            background: Rgb::new(0x1d, 0x1f, 0x21),
            cursor: Rgb::new(0xc5, 0xc8, 0xc6),
            selection: Rgb::new(0x37, 0x3b, 0x41),
        }
    }

    /// Default light theme palette
    pub fn default_light() -> Self {
        Self {
            ansi: [
                Rgb::new(0x00, 0x00, 0x00), // Black
                Rgb::new(0xc8, 0x28, 0x29), // Red
                Rgb::new(0x71, 0x8c, 0x00), // Green
                Rgb::new(0xec, 0xa4, 0x00), // Yellow
                Rgb::new(0x25, 0x6f, 0xef), // Blue
                Rgb::new(0x77, 0x59, 0xc8), // Magenta
                Rgb::new(0x00, 0x97, 0xa7), // Cyan
                Rgb::new(0x65, 0x7b, 0x83), // White
                Rgb::new(0x58, 0x6e, 0x75), // Bright Black
                Rgb::new(0xcb, 0x4b, 0x16), // Bright Red
                Rgb::new(0x85, 0x99, 0x00), // Bright Green
                Rgb::new(0xb5, 0x89, 0x00), // Bright Yellow
                Rgb::new(0x26, 0x8b, 0xd2), // Bright Blue
                Rgb::new(0x6c, 0x71, 0xc4), // Bright Magenta
                Rgb::new(0x2a, 0xa1, 0x98), // Bright Cyan
                Rgb::new(0xfd, 0xf6, 0xe3), // Bright White
            ],
            foreground: Rgb::new(0x00, 0x00, 0x00),
            background: Rgb::new(0xff, 0xff, 0xff),
            cursor: Rgb::new(0x00, 0x00, 0x00),
            selection: Rgb::new(0xee, 0xe8, 0xd5),
        }
    }
}

/// Convert 256-color index to RGB
fn index_to_rgb(index: u8, palette: &ColorPalette) -> Rgb {
    match index {
        // Standard ANSI colors (0-15)
        0..=15 => palette.ansi[index as usize],
        // 216-color cube (16-231)
        16..=231 => {
            let idx = index - 16;
            let r = (idx / 36) % 6;
            let g = (idx / 6) % 6;
            let b = idx % 6;

            let to_component = |c: u8| -> u8 {
                if c == 0 { 0 } else { 55 + c * 40 }
            };

            Rgb::new(to_component(r), to_component(g), to_component(b))
        }
        // Grayscale (232-255)
        232..=255 => {
            let gray = 8 + (index - 232) * 10;
            Rgb::new(gray, gray, gray)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgb_from_hex() {
        assert_eq!(Rgb::from_hex("#ff0000"), Some(Rgb::new(255, 0, 0)));
        assert_eq!(Rgb::from_hex("00ff00"), Some(Rgb::new(0, 255, 0)));
        assert_eq!(Rgb::from_hex("#invalid"), None);
    }

    #[test]
    fn test_rgb_to_hex() {
        assert_eq!(Rgb::new(255, 0, 0).to_hex(), "#ff0000");
        assert_eq!(Rgb::new(0, 255, 0).to_hex(), "#00ff00");
    }

    #[test]
    fn test_ansi_color_bright() {
        assert_eq!(AnsiColor::Red.bright(), AnsiColor::BrightRed);
        assert_eq!(AnsiColor::BrightRed.bright(), AnsiColor::BrightRed);
    }

    #[test]
    fn test_index_to_rgb() {
        let palette = ColorPalette::default();
        // First ANSI color
        assert_eq!(index_to_rgb(0, &palette), palette.ansi[0]);
        // 216 color cube - pure red
        let red = index_to_rgb(196, &palette);
        assert_eq!(red, Rgb::new(255, 0, 0));
        // Grayscale
        let gray = index_to_rgb(244, &palette);
        assert_eq!(gray.r, gray.g);
        assert_eq!(gray.g, gray.b);
    }
}
