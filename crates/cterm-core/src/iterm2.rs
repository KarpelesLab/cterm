//! iTerm2 inline image protocol support
//!
//! Implements parsing for OSC 1337 File= protocol used by iTerm2 for
//! inline images and file transfers.
//!
//! Protocol format:
//! ```text
//! OSC 1337 ; File=[params] : base64data ST
//! ```
//!
//! Parameters (semicolon-separated key=value):
//! - `name=base64_filename` - Filename (base64 encoded)
//! - `size=bytes` - Expected file size
//! - `width=auto|Npx|Ncells|N%` - Display width
//! - `height=auto|Npx|Ncells|N%` - Display height
//! - `preserveAspectRatio=0|1` - Default 1
//! - `inline=0|1` - 1=display image, 0=file transfer

use base64::Engine;

/// Dimension specification for iTerm2 images
#[derive(Debug, Clone, PartialEq)]
pub enum Iterm2Dimension {
    /// Automatic sizing based on image dimensions
    Auto,
    /// Size in pixels
    Pixels(usize),
    /// Size in terminal cells
    Cells(usize),
    /// Size as percentage of terminal width/height
    Percent(f32),
}

impl Default for Iterm2Dimension {
    fn default() -> Self {
        Self::Auto
    }
}

impl Iterm2Dimension {
    /// Parse a dimension string like "auto", "100px", "10", or "50%"
    pub fn parse(s: &str) -> Self {
        let s = s.trim();
        if s.is_empty() || s.eq_ignore_ascii_case("auto") {
            return Self::Auto;
        }

        if let Some(px) = s.strip_suffix("px") {
            if let Ok(n) = px.parse::<usize>() {
                return Self::Pixels(n);
            }
        }

        if let Some(pct) = s.strip_suffix('%') {
            if let Ok(n) = pct.parse::<f32>() {
                return Self::Percent(n);
            }
        }

        // Plain number = cells
        if let Ok(n) = s.parse::<usize>() {
            return Self::Cells(n);
        }

        Self::Auto
    }

    /// Calculate pixel size given cell dimension and image native size
    pub fn to_pixels(
        &self,
        cell_size: f64,
        terminal_size_cells: usize,
        image_native_pixels: usize,
    ) -> usize {
        match self {
            Self::Auto => image_native_pixels,
            Self::Pixels(px) => *px,
            Self::Cells(cells) => (*cells as f64 * cell_size).round() as usize,
            Self::Percent(pct) => {
                let terminal_pixels = terminal_size_cells as f64 * cell_size;
                (terminal_pixels * (*pct as f64 / 100.0)).round() as usize
            }
        }
    }
}

/// Parameters parsed from an iTerm2 File= sequence
#[derive(Debug, Clone, Default)]
pub struct Iterm2FileParams {
    /// Decoded filename (from base64)
    pub name: Option<String>,
    /// Expected file size in bytes
    pub size: Option<usize>,
    /// Display width specification
    pub width: Iterm2Dimension,
    /// Display height specification
    pub height: Iterm2Dimension,
    /// Whether to preserve aspect ratio (default true)
    pub preserve_aspect_ratio: bool,
    /// Whether to display inline (true) or as file transfer (false)
    pub inline: bool,
}

impl Iterm2FileParams {
    /// Parse parameter string from OSC 1337 ; File=<params> : data
    ///
    /// The params are semicolon-separated key=value pairs.
    pub fn parse(param_str: &str) -> Self {
        let mut params = Self {
            preserve_aspect_ratio: true, // Default to preserving aspect ratio
            ..Default::default()
        };

        for part in param_str.split(';') {
            let part = part.trim();
            if let Some((key, value)) = part.split_once('=') {
                let key = key.trim();
                let value = value.trim();

                match key {
                    "name" => {
                        // Name is base64 encoded
                        if let Ok(decoded) =
                            base64::engine::general_purpose::STANDARD.decode(value)
                        {
                            if let Ok(s) = String::from_utf8(decoded) {
                                params.name = Some(s);
                            }
                        }
                    }
                    "size" => {
                        params.size = value.parse().ok();
                    }
                    "width" => {
                        params.width = Iterm2Dimension::parse(value);
                    }
                    "height" => {
                        params.height = Iterm2Dimension::parse(value);
                    }
                    "preserveAspectRatio" => {
                        params.preserve_aspect_ratio = value != "0";
                    }
                    "inline" => {
                        params.inline = value == "1";
                    }
                    _ => {
                        log::trace!("Unknown iTerm2 File parameter: {}={}", key, value);
                    }
                }
            }
        }

        params
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dimension_parse() {
        assert_eq!(Iterm2Dimension::parse("auto"), Iterm2Dimension::Auto);
        assert_eq!(Iterm2Dimension::parse(""), Iterm2Dimension::Auto);
        assert_eq!(Iterm2Dimension::parse("100px"), Iterm2Dimension::Pixels(100));
        assert_eq!(Iterm2Dimension::parse("10"), Iterm2Dimension::Cells(10));
        assert_eq!(Iterm2Dimension::parse("50%"), Iterm2Dimension::Percent(50.0));
    }

    #[test]
    fn test_params_parse_inline_image() {
        // name=dGVzdC5wbmc= is base64 for "test.png"
        let params = Iterm2FileParams::parse("name=dGVzdC5wbmc=;inline=1;width=100px");
        assert_eq!(params.name, Some("test.png".to_string()));
        assert!(params.inline);
        assert_eq!(params.width, Iterm2Dimension::Pixels(100));
        assert!(params.preserve_aspect_ratio);
    }

    #[test]
    fn test_params_parse_file_transfer() {
        let params = Iterm2FileParams::parse("name=ZmlsZS5iaW4=;size=1024;inline=0");
        assert_eq!(params.name, Some("file.bin".to_string()));
        assert_eq!(params.size, Some(1024));
        assert!(!params.inline);
    }

    #[test]
    fn test_params_parse_preserve_aspect_ratio() {
        let params1 = Iterm2FileParams::parse("preserveAspectRatio=0");
        assert!(!params1.preserve_aspect_ratio);

        let params2 = Iterm2FileParams::parse("preserveAspectRatio=1");
        assert!(params2.preserve_aspect_ratio);

        // Default should be true
        let params3 = Iterm2FileParams::parse("");
        assert!(params3.preserve_aspect_ratio);
    }

    #[test]
    fn test_dimension_to_pixels() {
        let cell_size = 10.0;
        let terminal_cells = 80;
        let image_native = 200;

        assert_eq!(
            Iterm2Dimension::Auto.to_pixels(cell_size, terminal_cells, image_native),
            200
        );
        assert_eq!(
            Iterm2Dimension::Pixels(150).to_pixels(cell_size, terminal_cells, image_native),
            150
        );
        assert_eq!(
            Iterm2Dimension::Cells(5).to_pixels(cell_size, terminal_cells, image_native),
            50
        );
        assert_eq!(
            Iterm2Dimension::Percent(50.0).to_pixels(cell_size, terminal_cells, image_native),
            400 // 50% of 800 (80 cells * 10 pixels)
        );
    }
}
