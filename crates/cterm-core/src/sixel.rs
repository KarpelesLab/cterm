//! Sixel graphics decoder
//!
//! Implements the DEC Sixel graphics protocol for inline bitmap images.
//! Sixel is a format where each character (63-126) represents 6 vertical pixels.

/// Decoded sixel image as RGBA pixels
#[derive(Debug, Clone)]
pub struct SixelImage {
    /// RGBA pixel data (4 bytes per pixel)
    pub data: Vec<u8>,
    /// Image width in pixels
    pub width: usize,
    /// Image height in pixels
    pub height: usize,
}

/// Parse state for color/repeat parsing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseState {
    /// Normal sixel data mode
    Normal,
    /// Parsing repeat count (after '!')
    Repeat,
    /// Parsing color number (after '#')
    Color,
    /// Parsing color definition (after '#n;')
    ColorDef,
}

/// Sixel decoder
pub struct SixelDecoder {
    /// Color palette (RGBA)
    palette: [[u8; 4]; 256],
    /// Currently selected color index
    current_color: u8,
    /// Whether background should be transparent
    transparent_bg: bool,
    /// Current X position
    x: usize,
    /// Current 6-pixel band Y position
    band_y: usize,
    /// Pixel data buffer (RGBA)
    pixels: Vec<u8>,
    /// Maximum X position seen (determines width)
    max_x: usize,
    /// Repeat count for next sixel character
    repeat_count: usize,
    /// Current parse state
    parse_state: ParseState,
    /// Accumulated number during parsing
    accum: usize,
    /// Color definition parameters
    color_params: [usize; 5],
    /// Current color parameter index
    color_param_idx: usize,
}

impl SixelDecoder {
    /// Create a new sixel decoder
    pub fn new() -> Self {
        Self {
            palette: Self::default_palette(),
            current_color: 0,
            transparent_bg: false,
            x: 0,
            band_y: 0,
            pixels: Vec::new(),
            max_x: 0,
            repeat_count: 1,
            parse_state: ParseState::Normal,
            accum: 0,
            color_params: [0; 5],
            color_param_idx: 0,
        }
    }

    /// Create a new sixel decoder with parameters from DCS
    pub fn with_params(params: &[u16]) -> Self {
        let mut decoder = Self::new();

        // Parse DCS parameters:
        // Pn1 : pixel aspect ratio (ignored, we use 1:1)
        // Pn2 : background select (0=device default, 1=no change, 2=set to 0)
        // Pn3 : horizontal grid size (ignored)
        if params.len() > 1 {
            match params[1] {
                2 => decoder.transparent_bg = false, // Background set to color 0
                1 => decoder.transparent_bg = true,  // No change (transparent)
                _ => decoder.transparent_bg = false, // Device default
            }
        }

        decoder
    }

    /// Initialize the VT340-compatible 16-color palette
    fn default_palette() -> [[u8; 4]; 256] {
        let mut palette = [[0, 0, 0, 255]; 256];

        // VT340 default 16 colors
        palette[0] = [0, 0, 0, 255];       // Black
        palette[1] = [51, 51, 204, 255];   // Blue
        palette[2] = [204, 51, 51, 255];   // Red
        palette[3] = [51, 204, 51, 255];   // Green
        palette[4] = [204, 51, 204, 255];  // Magenta
        palette[5] = [51, 204, 204, 255];  // Cyan
        palette[6] = [204, 204, 51, 255];  // Yellow
        palette[7] = [204, 204, 204, 255]; // White
        palette[8] = [51, 51, 51, 255];    // Bright Black (Gray)
        palette[9] = [102, 102, 255, 255]; // Bright Blue
        palette[10] = [255, 102, 102, 255]; // Bright Red
        palette[11] = [102, 255, 102, 255]; // Bright Green
        palette[12] = [255, 102, 255, 255]; // Bright Magenta
        palette[13] = [102, 255, 255, 255]; // Bright Cyan
        palette[14] = [255, 255, 102, 255]; // Bright Yellow
        palette[15] = [255, 255, 255, 255]; // Bright White

        // Initialize remaining colors to a basic pattern
        for i in 16..256 {
            let gray = ((i - 16) * 255 / 240) as u8;
            palette[i] = [gray, gray, gray, 255];
        }

        palette
    }

    /// Process a byte of sixel data
    pub fn put(&mut self, byte: u8) {
        match self.parse_state {
            ParseState::Normal => self.put_normal(byte),
            ParseState::Repeat => self.put_repeat(byte),
            ParseState::Color => self.put_color(byte),
            ParseState::ColorDef => self.put_color_def(byte),
        }
    }

    /// Process bytes in normal mode
    fn put_normal(&mut self, byte: u8) {
        match byte {
            // Repeat introducer
            b'!' => {
                self.parse_state = ParseState::Repeat;
                self.accum = 0;
            }
            // Color introducer
            b'#' => {
                self.parse_state = ParseState::Color;
                self.accum = 0;
                self.color_param_idx = 0;
                self.color_params = [0; 5];
            }
            // Carriage return (go to start of band)
            b'$' => {
                self.x = 0;
            }
            // Line feed (next 6-pixel band)
            b'-' => {
                self.x = 0;
                self.band_y += 1;
            }
            // Sixel data (63-126 = '?' to '~')
            63..=126 => {
                self.draw_sixel(byte - 63);
            }
            // Ignore other bytes
            _ => {}
        }
    }

    /// Process bytes in repeat mode
    fn put_repeat(&mut self, byte: u8) {
        match byte {
            b'0'..=b'9' => {
                self.accum = self.accum.saturating_mul(10).saturating_add((byte - b'0') as usize);
            }
            // Sixel data ends repeat mode
            63..=126 => {
                self.repeat_count = self.accum.max(1);
                self.draw_sixel(byte - 63);
                self.repeat_count = 1;
                self.parse_state = ParseState::Normal;
            }
            // Any other byte resets to normal
            _ => {
                self.parse_state = ParseState::Normal;
                self.put_normal(byte);
            }
        }
    }

    /// Process bytes in color selection/definition mode
    fn put_color(&mut self, byte: u8) {
        match byte {
            b'0'..=b'9' => {
                self.accum = self.accum.saturating_mul(10).saturating_add((byte - b'0') as usize);
            }
            b';' => {
                // Store accumulated value and switch to color definition mode
                self.color_params[self.color_param_idx] = self.accum;
                self.color_param_idx += 1;
                self.accum = 0;
                if self.color_param_idx == 1 {
                    self.parse_state = ParseState::ColorDef;
                }
            }
            // Any other byte: just select the color and return to normal
            _ => {
                self.current_color = (self.accum % 256) as u8;
                self.parse_state = ParseState::Normal;
                // Process this byte as normal
                if byte != b'#' {
                    self.put_normal(byte);
                } else {
                    // Start new color sequence
                    self.accum = 0;
                    self.color_param_idx = 0;
                    self.color_params = [0; 5];
                    self.parse_state = ParseState::Color;
                }
            }
        }
    }

    /// Process bytes in color definition mode
    fn put_color_def(&mut self, byte: u8) {
        match byte {
            b'0'..=b'9' => {
                self.accum = self.accum.saturating_mul(10).saturating_add((byte - b'0') as usize);
            }
            b';' => {
                if self.color_param_idx < 5 {
                    self.color_params[self.color_param_idx] = self.accum;
                    self.color_param_idx += 1;
                }
                self.accum = 0;
            }
            // Any other byte ends color definition
            _ => {
                // Store final accumulated value
                if self.color_param_idx < 5 {
                    self.color_params[self.color_param_idx] = self.accum;
                    self.color_param_idx += 1;
                }

                // Define the color
                self.define_color();

                self.parse_state = ParseState::Normal;
                // Process this byte as normal
                if byte != b'#' {
                    self.put_normal(byte);
                } else {
                    // Start new color sequence
                    self.accum = 0;
                    self.color_param_idx = 0;
                    self.color_params = [0; 5];
                    self.parse_state = ParseState::Color;
                }
            }
        }
    }

    /// Define a color in the palette
    fn define_color(&mut self) {
        // params[0] = color number
        // params[1] = color coordinate system (1=HLS, 2=RGB)
        // params[2..4] = color values

        let color_idx = (self.color_params[0] % 256) as u8;
        let color_system = self.color_params[1];

        match color_system {
            // RGB (values are 0-100)
            2 => {
                let r = ((self.color_params[2].min(100) * 255) / 100) as u8;
                let g = ((self.color_params[3].min(100) * 255) / 100) as u8;
                let b = ((self.color_params[4].min(100) * 255) / 100) as u8;
                self.palette[color_idx as usize] = [r, g, b, 255];
            }
            // HLS (hue 0-360, lightness 0-100, saturation 0-100)
            1 => {
                let h = self.color_params[2] % 360;
                let l = self.color_params[3].min(100);
                let s = self.color_params[4].min(100);
                let (r, g, b) = Self::hls_to_rgb(h, l, s);
                self.palette[color_idx as usize] = [r, g, b, 255];
            }
            _ => {
                // Unknown color system, ignore
            }
        }

        // Select this color
        self.current_color = color_idx;
    }

    /// Convert HLS to RGB (H: 0-360, L: 0-100, S: 0-100)
    fn hls_to_rgb(h: usize, l: usize, s: usize) -> (u8, u8, u8) {
        let h = h as f64;
        let l = l as f64 / 100.0;
        let s = s as f64 / 100.0;

        if s == 0.0 {
            let gray = (l * 255.0) as u8;
            return (gray, gray, gray);
        }

        let q = if l < 0.5 {
            l * (1.0 + s)
        } else {
            l + s - l * s
        };
        let p = 2.0 * l - q;

        let h_norm = h / 360.0;

        fn hue_to_rgb(p: f64, q: f64, mut t: f64) -> f64 {
            if t < 0.0 {
                t += 1.0;
            }
            if t > 1.0 {
                t -= 1.0;
            }
            if t < 1.0 / 6.0 {
                return p + (q - p) * 6.0 * t;
            }
            if t < 1.0 / 2.0 {
                return q;
            }
            if t < 2.0 / 3.0 {
                return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
            }
            p
        }

        let r = (hue_to_rgb(p, q, h_norm + 1.0 / 3.0) * 255.0) as u8;
        let g = (hue_to_rgb(p, q, h_norm) * 255.0) as u8;
        let b = (hue_to_rgb(p, q, h_norm - 1.0 / 3.0) * 255.0) as u8;

        (r, g, b)
    }

    /// Draw a sixel character (6 vertical pixels)
    fn draw_sixel(&mut self, sixel: u8) {
        let color = self.palette[self.current_color as usize];

        for _ in 0..self.repeat_count {
            // Ensure we have enough space in the pixel buffer
            let required_height = (self.band_y + 1) * 6;
            let required_width = self.x + 1;

            // Expand buffer if needed
            self.ensure_size(required_width, required_height);

            // Draw 6 vertical pixels
            for bit in 0..6 {
                if (sixel >> bit) & 1 != 0 {
                    let y = self.band_y * 6 + bit;
                    self.set_pixel(self.x, y, color);
                }
            }

            self.x += 1;
            if self.x > self.max_x {
                self.max_x = self.x;
            }
        }
    }

    /// Ensure the pixel buffer is large enough
    fn ensure_size(&mut self, width: usize, height: usize) {
        let current_width = self.max_x.max(1);
        let current_height = self.pixels.len() / (current_width * 4);

        if width > current_width || height > current_height {
            let new_width = width.max(current_width);
            let new_height = height.max(current_height);

            // Create new buffer
            let mut new_pixels = if self.transparent_bg {
                vec![0u8; new_width * new_height * 4]
            } else {
                // Fill with background color (color 0)
                let bg = self.palette[0];
                let mut buf = Vec::with_capacity(new_width * new_height * 4);
                for _ in 0..(new_width * new_height) {
                    buf.extend_from_slice(&bg);
                }
                buf
            };

            // Copy existing data
            for y in 0..current_height {
                for x in 0..current_width {
                    let old_idx = (y * current_width + x) * 4;
                    let new_idx = (y * new_width + x) * 4;
                    if old_idx + 4 <= self.pixels.len() && new_idx + 4 <= new_pixels.len() {
                        new_pixels[new_idx..new_idx + 4]
                            .copy_from_slice(&self.pixels[old_idx..old_idx + 4]);
                    }
                }
            }

            self.pixels = new_pixels;
            self.max_x = new_width;
        }
    }

    /// Set a pixel in the buffer
    fn set_pixel(&mut self, x: usize, y: usize, color: [u8; 4]) {
        let width = self.max_x.max(1);
        let idx = (y * width + x) * 4;
        if idx + 4 <= self.pixels.len() {
            self.pixels[idx..idx + 4].copy_from_slice(&color);
        }
    }

    /// Finalize decoding and return the image
    pub fn finish(self) -> Option<SixelImage> {
        if self.max_x == 0 {
            return None;
        }

        let width = self.max_x;
        let height = (self.band_y + 1) * 6;
        let expected_size = width * height * 4;

        // Ensure correct size
        let mut data = self.pixels;
        if data.len() < expected_size {
            data.resize(expected_size, 0);
        } else if data.len() > expected_size {
            data.truncate(expected_size);
        }

        Some(SixelImage {
            data,
            width,
            height,
        })
    }
}

impl Default for SixelDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_sixel() {
        let mut decoder = SixelDecoder::new();

        // Draw a simple pattern: all 6 pixels on (character '~' = 63)
        // '~' - 63 = 63 = 0b111111 (all 6 bits set)
        decoder.put(b'~');

        let image = decoder.finish().unwrap();
        assert_eq!(image.width, 1);
        assert_eq!(image.height, 6);

        // Check that pixels are set (not transparent/black)
        for y in 0..6 {
            let idx = y * 4;
            // Should be the default color (black from palette[0])
            assert_eq!(image.data[idx + 3], 255); // Alpha should be 255
        }
    }

    #[test]
    fn test_repeat() {
        let mut decoder = SixelDecoder::new();

        // Repeat '~' 5 times: "!5~"
        decoder.put(b'!');
        decoder.put(b'5');
        decoder.put(b'~');

        let image = decoder.finish().unwrap();
        assert_eq!(image.width, 5);
        assert_eq!(image.height, 6);
    }

    #[test]
    fn test_color_select() {
        let mut decoder = SixelDecoder::new();

        // Select color 1 (blue) and draw
        decoder.put(b'#');
        decoder.put(b'1');
        decoder.put(b'~');

        let image = decoder.finish().unwrap();

        // First pixel should be blue-ish
        assert_eq!(image.data[0], 51); // R
        assert_eq!(image.data[1], 51); // G
        assert_eq!(image.data[2], 204); // B
    }

    #[test]
    fn test_color_define_rgb() {
        let mut decoder = SixelDecoder::new();

        // Define color 5 as RGB(100, 50, 0) and draw
        // #5;2;100;50;0
        for byte in b"#5;2;100;50;0~" {
            decoder.put(*byte);
        }

        let image = decoder.finish().unwrap();

        // First pixel should be the defined color
        assert_eq!(image.data[0], 255); // R (100% = 255)
        assert_eq!(image.data[1], 127); // G (50% ≈ 127)
        assert_eq!(image.data[2], 0);   // B (0% = 0)
    }

    #[test]
    fn test_line_feed() {
        let mut decoder = SixelDecoder::new();

        // Draw on first band, then move to second band
        decoder.put(b'~');
        decoder.put(b'-'); // Line feed
        decoder.put(b'~');

        let image = decoder.finish().unwrap();
        assert_eq!(image.width, 1);
        assert_eq!(image.height, 12); // 2 bands * 6 pixels
    }

    #[test]
    fn test_carriage_return() {
        let mut decoder = SixelDecoder::new();

        // Draw 3 pixels, carriage return, draw 2 more (overwrites first 2)
        decoder.put(b'~');
        decoder.put(b'~');
        decoder.put(b'~');
        decoder.put(b'$'); // Carriage return
        decoder.put(b'?'); // Empty sixel (0 bits)
        decoder.put(b'?');

        let image = decoder.finish().unwrap();
        assert_eq!(image.width, 3); // Max width is still 3
    }
}
