//! Theme and color scheme types
//!
//! Defines the theme structure for customizing terminal appearance.

use cterm_core::color::{ColorPalette, Rgb};
use serde::{Deserialize, Serialize};

/// Complete terminal theme
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    /// Theme name
    pub name: String,
    /// Theme author (optional)
    pub author: Option<String>,
    /// Color palette for terminal
    pub colors: ColorPalette,
    /// UI colors
    pub ui: UiColors,
    /// Cursor appearance
    pub cursor: CursorTheme,
}

impl Default for Theme {
    fn default() -> Self {
        Self::dark()
    }
}

impl Theme {
    /// Create a dark theme
    pub fn dark() -> Self {
        Self {
            name: "Default Dark".into(),
            author: None,
            colors: ColorPalette::default_dark(),
            ui: UiColors::dark(),
            cursor: CursorTheme::default(),
        }
    }

    /// Create a light theme
    pub fn light() -> Self {
        Self {
            name: "Default Light".into(),
            author: None,
            colors: ColorPalette::default_light(),
            ui: UiColors::light(),
            cursor: CursorTheme {
                color: Rgb::new(0, 0, 0),
                text_color: Rgb::new(255, 255, 255),
            },
        }
    }

    /// Tokyo Night theme
    pub fn tokyo_night() -> Self {
        Self {
            name: "Tokyo Night".into(),
            author: Some("folke".into()),
            colors: ColorPalette {
                ansi: [
                    Rgb::new(0x15, 0x16, 0x1e), // Black
                    Rgb::new(0xf7, 0x76, 0x8e), // Red
                    Rgb::new(0x9e, 0xce, 0x6a), // Green
                    Rgb::new(0xe0, 0xaf, 0x68), // Yellow
                    Rgb::new(0x7a, 0xa2, 0xf7), // Blue
                    Rgb::new(0xbb, 0x9a, 0xf7), // Magenta
                    Rgb::new(0x7d, 0xcf, 0xff), // Cyan
                    Rgb::new(0xa9, 0xb1, 0xd6), // White
                    Rgb::new(0x41, 0x48, 0x68), // Bright Black
                    Rgb::new(0xf7, 0x76, 0x8e), // Bright Red
                    Rgb::new(0x9e, 0xce, 0x6a), // Bright Green
                    Rgb::new(0xe0, 0xaf, 0x68), // Bright Yellow
                    Rgb::new(0x7a, 0xa2, 0xf7), // Bright Blue
                    Rgb::new(0xbb, 0x9a, 0xf7), // Bright Magenta
                    Rgb::new(0x7d, 0xcf, 0xff), // Bright Cyan
                    Rgb::new(0xc0, 0xca, 0xf5), // Bright White
                ],
                foreground: Rgb::new(0xc0, 0xca, 0xf5),
                background: Rgb::new(0x1a, 0x1b, 0x26),
                cursor: Rgb::new(0xc0, 0xca, 0xf5),
                selection: Rgb::new(0x28, 0x3b, 0x61),
            },
            ui: UiColors {
                tab_bar_background: Rgb::new(0x16, 0x16, 0x1e),
                tab_active_background: Rgb::new(0x1a, 0x1b, 0x26),
                tab_inactive_background: Rgb::new(0x16, 0x16, 0x1e),
                tab_active_text: Rgb::new(0xc0, 0xca, 0xf5),
                tab_inactive_text: Rgb::new(0x56, 0x5f, 0x89),
                border: Rgb::new(0x28, 0x28, 0x40),
                scrollbar: Rgb::new(0x41, 0x48, 0x68),
                scrollbar_hover: Rgb::new(0x56, 0x5f, 0x89),
            },
            cursor: CursorTheme {
                color: Rgb::new(0xc0, 0xca, 0xf5),
                text_color: Rgb::new(0x1a, 0x1b, 0x26),
            },
        }
    }

    /// Dracula theme
    pub fn dracula() -> Self {
        Self {
            name: "Dracula".into(),
            author: Some("Zeno Rocha".into()),
            colors: ColorPalette {
                ansi: [
                    Rgb::new(0x21, 0x22, 0x2c), // Black
                    Rgb::new(0xff, 0x55, 0x55), // Red
                    Rgb::new(0x50, 0xfa, 0x7b), // Green
                    Rgb::new(0xf1, 0xfa, 0x8c), // Yellow
                    Rgb::new(0xbd, 0x93, 0xf9), // Blue
                    Rgb::new(0xff, 0x79, 0xc6), // Magenta
                    Rgb::new(0x8b, 0xe9, 0xfd), // Cyan
                    Rgb::new(0xf8, 0xf8, 0xf2), // White
                    Rgb::new(0x62, 0x72, 0xa4), // Bright Black
                    Rgb::new(0xff, 0x6e, 0x6e), // Bright Red
                    Rgb::new(0x69, 0xff, 0x94), // Bright Green
                    Rgb::new(0xff, 0xff, 0xa5), // Bright Yellow
                    Rgb::new(0xd6, 0xac, 0xff), // Bright Blue
                    Rgb::new(0xff, 0x92, 0xdf), // Bright Magenta
                    Rgb::new(0xa4, 0xff, 0xff), // Bright Cyan
                    Rgb::new(0xff, 0xff, 0xff), // Bright White
                ],
                foreground: Rgb::new(0xf8, 0xf8, 0xf2),
                background: Rgb::new(0x28, 0x2a, 0x36),
                cursor: Rgb::new(0xf8, 0xf8, 0xf2),
                selection: Rgb::new(0x44, 0x47, 0x5a),
            },
            ui: UiColors {
                tab_bar_background: Rgb::new(0x21, 0x22, 0x2c),
                tab_active_background: Rgb::new(0x28, 0x2a, 0x36),
                tab_inactive_background: Rgb::new(0x21, 0x22, 0x2c),
                tab_active_text: Rgb::new(0xf8, 0xf8, 0xf2),
                tab_inactive_text: Rgb::new(0x62, 0x72, 0xa4),
                border: Rgb::new(0x44, 0x47, 0x5a),
                scrollbar: Rgb::new(0x44, 0x47, 0x5a),
                scrollbar_hover: Rgb::new(0x62, 0x72, 0xa4),
            },
            cursor: CursorTheme {
                color: Rgb::new(0xf8, 0xf8, 0xf2),
                text_color: Rgb::new(0x28, 0x2a, 0x36),
            },
        }
    }

    /// Nord theme
    pub fn nord() -> Self {
        Self {
            name: "Nord".into(),
            author: Some("Arctic Ice Studio".into()),
            colors: ColorPalette {
                ansi: [
                    Rgb::new(0x3b, 0x42, 0x52), // Black
                    Rgb::new(0xbf, 0x61, 0x6a), // Red
                    Rgb::new(0xa3, 0xbe, 0x8c), // Green
                    Rgb::new(0xeb, 0xcb, 0x8b), // Yellow
                    Rgb::new(0x81, 0xa1, 0xc1), // Blue
                    Rgb::new(0xb4, 0x8e, 0xad), // Magenta
                    Rgb::new(0x88, 0xc0, 0xd0), // Cyan
                    Rgb::new(0xe5, 0xe9, 0xf0), // White
                    Rgb::new(0x4c, 0x56, 0x6a), // Bright Black
                    Rgb::new(0xbf, 0x61, 0x6a), // Bright Red
                    Rgb::new(0xa3, 0xbe, 0x8c), // Bright Green
                    Rgb::new(0xeb, 0xcb, 0x8b), // Bright Yellow
                    Rgb::new(0x81, 0xa1, 0xc1), // Bright Blue
                    Rgb::new(0xb4, 0x8e, 0xad), // Bright Magenta
                    Rgb::new(0x8f, 0xbc, 0xbb), // Bright Cyan
                    Rgb::new(0xec, 0xef, 0xf4), // Bright White
                ],
                foreground: Rgb::new(0xd8, 0xde, 0xe9),
                background: Rgb::new(0x2e, 0x34, 0x40),
                cursor: Rgb::new(0xd8, 0xde, 0xe9),
                selection: Rgb::new(0x43, 0x4c, 0x5e),
            },
            ui: UiColors {
                tab_bar_background: Rgb::new(0x2e, 0x34, 0x40),
                tab_active_background: Rgb::new(0x3b, 0x42, 0x52),
                tab_inactive_background: Rgb::new(0x2e, 0x34, 0x40),
                tab_active_text: Rgb::new(0xec, 0xef, 0xf4),
                tab_inactive_text: Rgb::new(0x4c, 0x56, 0x6a),
                border: Rgb::new(0x4c, 0x56, 0x6a),
                scrollbar: Rgb::new(0x4c, 0x56, 0x6a),
                scrollbar_hover: Rgb::new(0x5e, 0x81, 0xac),
            },
            cursor: CursorTheme {
                color: Rgb::new(0xd8, 0xde, 0xe9),
                text_color: Rgb::new(0x2e, 0x34, 0x40),
            },
        }
    }

    /// Get all built-in themes
    pub fn builtin_themes() -> Vec<Theme> {
        vec![
            Theme::dark(),
            Theme::light(),
            Theme::tokyo_night(),
            Theme::dracula(),
            Theme::nord(),
        ]
    }
}

/// UI element colors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiColors {
    /// Tab bar background
    pub tab_bar_background: Rgb,
    /// Active tab background
    pub tab_active_background: Rgb,
    /// Inactive tab background
    pub tab_inactive_background: Rgb,
    /// Active tab text
    pub tab_active_text: Rgb,
    /// Inactive tab text
    pub tab_inactive_text: Rgb,
    /// Border color
    pub border: Rgb,
    /// Scrollbar color
    pub scrollbar: Rgb,
    /// Scrollbar hover color
    pub scrollbar_hover: Rgb,
}

impl UiColors {
    /// Dark UI colors
    pub fn dark() -> Self {
        Self {
            tab_bar_background: Rgb::new(0x1a, 0x1a, 0x1a),
            tab_active_background: Rgb::new(0x2d, 0x2d, 0x2d),
            tab_inactive_background: Rgb::new(0x1a, 0x1a, 0x1a),
            tab_active_text: Rgb::new(0xff, 0xff, 0xff),
            tab_inactive_text: Rgb::new(0x80, 0x80, 0x80),
            border: Rgb::new(0x40, 0x40, 0x40),
            scrollbar: Rgb::new(0x50, 0x50, 0x50),
            scrollbar_hover: Rgb::new(0x70, 0x70, 0x70),
        }
    }

    /// Light UI colors
    pub fn light() -> Self {
        Self {
            tab_bar_background: Rgb::new(0xf0, 0xf0, 0xf0),
            tab_active_background: Rgb::new(0xff, 0xff, 0xff),
            tab_inactive_background: Rgb::new(0xe0, 0xe0, 0xe0),
            tab_active_text: Rgb::new(0x00, 0x00, 0x00),
            tab_inactive_text: Rgb::new(0x60, 0x60, 0x60),
            border: Rgb::new(0xc0, 0xc0, 0xc0),
            scrollbar: Rgb::new(0xc0, 0xc0, 0xc0),
            scrollbar_hover: Rgb::new(0xa0, 0xa0, 0xa0),
        }
    }
}

/// Cursor appearance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorTheme {
    /// Cursor color
    pub color: Rgb,
    /// Text color under cursor
    pub text_color: Rgb,
}

impl Default for CursorTheme {
    fn default() -> Self {
        Self {
            color: Rgb::new(0xc5, 0xc8, 0xc6),
            text_color: Rgb::new(0x1d, 0x1f, 0x21),
        }
    }
}

/// Font configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontConfig {
    /// Font family name
    pub family: String,
    /// Font size in points
    pub size: f64,
    /// Whether to use font ligatures
    pub ligatures: bool,
    /// Line height multiplier
    pub line_height: f64,
    /// Letter spacing adjustment
    pub letter_spacing: f64,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: Self::default_font_family().into(),
            size: 12.0,
            ligatures: true,
            line_height: 1.0,
            letter_spacing: 0.0,
        }
    }
}

impl FontConfig {
    /// Get the default font family for the current platform
    fn default_font_family() -> &'static str {
        #[cfg(target_os = "windows")]
        {
            // Cascadia Mono is included with Windows Terminal and newer Windows
            // Consolas is the fallback, included since Windows Vista
            "Cascadia Mono, Consolas"
        }
        #[cfg(target_os = "macos")]
        {
            // SF Mono is the system monospace font on macOS
            // Menlo is the fallback for older systems
            "SF Mono, Menlo"
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            // On Linux, use generic monospace which respects fontconfig
            "monospace"
        }
    }

    /// Create config for JetBrains Mono
    pub fn jetbrains_mono() -> Self {
        Self {
            family: "JetBrains Mono".into(),
            ..Default::default()
        }
    }

    /// Create config for Fira Code
    pub fn fira_code() -> Self {
        Self {
            family: "Fira Code".into(),
            ..Default::default()
        }
    }

    /// Create config for Cascadia Code
    pub fn cascadia_code() -> Self {
        Self {
            family: "Cascadia Code".into(),
            ..Default::default()
        }
    }
}
