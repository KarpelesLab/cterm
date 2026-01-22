//! DRCS (Dynamically Redefinable Character Sets) support
//!
//! Implements DECDLD (DEC Download) for soft font definitions.
//! This allows applications to define custom character glyphs.

use std::collections::HashMap;

/// A single DRCS glyph (character bitmap)
#[derive(Debug, Clone)]
pub struct DrcsGlyph {
    /// Bitmap data (1 bit per pixel, row-major)
    /// Each byte contains 8 horizontal pixels
    pub data: Vec<u8>,
    /// Width in pixels
    pub width: usize,
    /// Height in pixels
    pub height: usize,
}

impl DrcsGlyph {
    /// Create a new empty glyph
    pub fn new(width: usize, height: usize) -> Self {
        let bytes_per_row = (width + 7) / 8;
        Self {
            data: vec![0; bytes_per_row * height],
            width,
            height,
        }
    }

    /// Set a pixel in the glyph
    pub fn set_pixel(&mut self, x: usize, y: usize, value: bool) {
        if x >= self.width || y >= self.height {
            return;
        }
        let bytes_per_row = (self.width + 7) / 8;
        let byte_idx = y * bytes_per_row + x / 8;
        let bit_idx = 7 - (x % 8);
        if value {
            self.data[byte_idx] |= 1 << bit_idx;
        } else {
            self.data[byte_idx] &= !(1 << bit_idx);
        }
    }

    /// Get a pixel from the glyph
    pub fn get_pixel(&self, x: usize, y: usize) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        let bytes_per_row = (self.width + 7) / 8;
        let byte_idx = y * bytes_per_row + x / 8;
        let bit_idx = 7 - (x % 8);
        (self.data[byte_idx] >> bit_idx) & 1 != 0
    }
}

/// A DRCS font (collection of glyphs)
#[derive(Debug, Clone)]
pub struct DrcsFont {
    /// Font number (0-2)
    pub font_number: u8,
    /// Character set designator (e.g., " @" for unregistered)
    pub designator: String,
    /// Character matrix width in pixels
    pub cell_width: usize,
    /// Character matrix height in pixels
    pub cell_height: usize,
    /// Whether this is a 96-character set (vs 94)
    pub is_96_char: bool,
    /// Whether this is a full-cell font (vs text font)
    pub full_cell: bool,
    /// Glyphs indexed by character position (0-95)
    pub glyphs: HashMap<u8, DrcsGlyph>,
}

impl DrcsFont {
    /// Create a new empty DRCS font
    pub fn new(
        font_number: u8,
        designator: String,
        cell_width: usize,
        cell_height: usize,
        is_96_char: bool,
        full_cell: bool,
    ) -> Self {
        Self {
            font_number,
            designator,
            cell_width,
            cell_height,
            is_96_char,
            full_cell,
            glyphs: HashMap::new(),
        }
    }

    /// Get a glyph by character position
    pub fn get_glyph(&self, pos: u8) -> Option<&DrcsGlyph> {
        self.glyphs.get(&pos)
    }
}

/// DECDLD decoder state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecodeState {
    /// Parsing character set designator (Dscs)
    Designator,
    /// Parsing sixel data for a glyph
    SixelData,
}

/// DECDLD decoder for parsing soft font downloads
pub struct DecdldDecoder {
    /// Font number (Pfn)
    font_number: u8,
    /// Starting character (Pcn)
    start_char: u8,
    /// Erase control (Pe)
    erase_control: u8,
    /// Character matrix width (Pcmw)
    cell_width: usize,
    /// Character matrix height (Pcmh)
    cell_height: usize,
    /// 96-character set (Pcss)
    is_96_char: bool,
    /// Full-cell font (Pt)
    full_cell: bool,
    /// Character set designator
    designator: String,
    /// Current decode state
    state: DecodeState,
    /// Current character being decoded
    current_char: u8,
    /// Current X position in glyph
    x: usize,
    /// Current Y position (sixel row, each is 6 pixels)
    sixel_row: usize,
    /// Current glyph being built
    current_glyph: Option<DrcsGlyph>,
    /// Completed glyphs
    glyphs: HashMap<u8, DrcsGlyph>,
}

impl DecdldDecoder {
    /// Create a new DECDLD decoder from DCS parameters
    ///
    /// Parameters are: Pfn ; Pcn ; Pe ; Pcmw ; Pss ; Pt ; Pcmh ; Pcss
    pub fn new(params: &[u16]) -> Self {
        // Parse parameters with defaults
        let font_number = params.first().copied().unwrap_or(0) as u8;
        let start_char = params.get(1).copied().unwrap_or(0) as u8;
        let erase_control = params.get(2).copied().unwrap_or(0) as u8;
        let pcmw = params.get(3).copied().unwrap_or(0);
        let _pss = params.get(4).copied().unwrap_or(0); // Font set size (ignored for now)
        let pt = params.get(5).copied().unwrap_or(0);
        let pcmh = params.get(6).copied().unwrap_or(0);
        let pcss = params.get(7).copied().unwrap_or(0);

        // Determine cell dimensions
        // Pcmw: 0 = default (10 for 80-col, 6 for 132-col), 2-4 = VT200 compat, 5-10 = VT510
        let cell_width = match pcmw {
            0 => 10, // Default
            2 => 5,  // VT200 compatibility (doubled height)
            3 => 6,
            4 => 7,
            w @ 5..=10 => w as usize,
            _ => 10,
        };

        // Pcmh: 0 = default (16), 1-16 = explicit height
        let cell_height = match pcmh {
            0 => 16,
            h @ 1..=16 => h as usize,
            _ => 16,
        };

        // Pcss: 0 = 94-character set, 1 = 96-character set
        let is_96_char = pcss == 1;

        // Pt: 0,1 = text font, 2 = full-cell font
        let full_cell = pt == 2;

        Self {
            font_number,
            start_char,
            erase_control,
            cell_width,
            cell_height,
            is_96_char,
            full_cell,
            designator: String::new(),
            state: DecodeState::Designator,
            current_char: start_char,
            x: 0,
            sixel_row: 0,
            current_glyph: None,
            glyphs: HashMap::new(),
        }
    }

    /// Process a byte of DECDLD data
    pub fn put(&mut self, byte: u8) {
        match self.state {
            DecodeState::Designator => self.parse_designator(byte),
            DecodeState::SixelData => self.parse_sixel(byte),
        }
    }

    /// Parse character set designator
    fn parse_designator(&mut self, byte: u8) {
        match byte {
            // Intermediate characters (SP to /)
            0x20..=0x2F => {
                self.designator.push(byte as char);
            }
            // Final character (0 to ~)
            0x30..=0x7E => {
                self.designator.push(byte as char);
                // Designator complete, switch to sixel data
                self.state = DecodeState::SixelData;
                self.start_new_glyph();
            }
            _ => {
                // Invalid, ignore
            }
        }
    }

    /// Parse sixel data for glyphs
    fn parse_sixel(&mut self, byte: u8) {
        match byte {
            // Sixel data (? to ~, values 0-63)
            0x3F..=0x7E => {
                let sixel = byte - 0x3F;
                self.draw_sixel(sixel);
            }
            // Row separator (/)
            0x2F => {
                // Move to next sixel row (6 pixels down)
                self.sixel_row += 1;
                self.x = 0;
            }
            // Character separator (;)
            0x3B => {
                // Finish current glyph and start next
                self.finish_glyph();
                self.current_char = self.current_char.saturating_add(1);
                self.start_new_glyph();
            }
            _ => {
                // Ignore other bytes
            }
        }
    }

    /// Start a new glyph
    fn start_new_glyph(&mut self) {
        self.current_glyph = Some(DrcsGlyph::new(self.cell_width, self.cell_height));
        self.x = 0;
        self.sixel_row = 0;
    }

    /// Draw a sixel (6 vertical pixels) at current position
    fn draw_sixel(&mut self, sixel: u8) {
        if let Some(ref mut glyph) = self.current_glyph {
            let base_y = self.sixel_row * 6;

            // Draw 6 vertical pixels
            for bit in 0..6 {
                let y = base_y + bit;
                if y < self.cell_height {
                    let pixel = (sixel >> bit) & 1 != 0;
                    glyph.set_pixel(self.x, y, pixel);
                }
            }

            self.x += 1;
        }
    }

    /// Finish the current glyph
    fn finish_glyph(&mut self) {
        if let Some(glyph) = self.current_glyph.take() {
            self.glyphs.insert(self.current_char, glyph);
        }
    }

    /// Finish decoding and return the font
    pub fn finish(mut self) -> Option<DrcsFont> {
        // Finish any pending glyph
        self.finish_glyph();

        if self.glyphs.is_empty() {
            return None;
        }

        let mut font = DrcsFont::new(
            self.font_number,
            self.designator,
            self.cell_width,
            self.cell_height,
            self.is_96_char,
            self.full_cell,
        );

        font.glyphs = self.glyphs;

        Some(font)
    }

    /// Get the erase control value
    pub fn erase_control(&self) -> u8 {
        self.erase_control
    }

    /// Get the font number
    pub fn font_number(&self) -> u8 {
        self.font_number
    }
}

impl Default for DecdldDecoder {
    fn default() -> Self {
        Self::new(&[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glyph_pixel_operations() {
        let mut glyph = DrcsGlyph::new(8, 12);

        // Set some pixels
        glyph.set_pixel(0, 0, true);
        glyph.set_pixel(7, 0, true);
        glyph.set_pixel(3, 5, true);

        // Check pixels
        assert!(glyph.get_pixel(0, 0));
        assert!(glyph.get_pixel(7, 0));
        assert!(glyph.get_pixel(3, 5));
        assert!(!glyph.get_pixel(1, 0));
        assert!(!glyph.get_pixel(0, 1));
    }

    #[test]
    fn test_decoder_simple() {
        // Create decoder with default params
        let mut decoder = DecdldDecoder::new(&[0, 0, 0, 0, 0, 0, 0, 0]);

        // Feed designator " @" (unregistered character set)
        decoder.put(b' ');
        decoder.put(b'@');

        // Feed a simple glyph pattern (all pixels on in first column)
        // '?' + 63 = '~' (all 6 bits set)
        decoder.put(b'~');

        let font = decoder.finish().unwrap();
        assert_eq!(font.designator, " @");
        assert!(font.glyphs.contains_key(&0));

        let glyph = font.get_glyph(0).unwrap();
        // Check that first column has pixels set
        assert!(glyph.get_pixel(0, 0));
        assert!(glyph.get_pixel(0, 5));
    }

    #[test]
    fn test_decoder_multiple_glyphs() {
        let mut decoder = DecdldDecoder::new(&[0, 0, 0, 0, 0, 0, 0, 0]);

        // Designator
        decoder.put(b'@');

        // First glyph
        decoder.put(b'~');
        decoder.put(b';'); // Separator

        // Second glyph
        decoder.put(b'?'); // All bits off (0)

        let font = decoder.finish().unwrap();
        assert_eq!(font.glyphs.len(), 2);
        assert!(font.glyphs.contains_key(&0));
        assert!(font.glyphs.contains_key(&1));
    }

    #[test]
    fn test_decoder_row_separator() {
        let mut decoder = DecdldDecoder::new(&[0, 0, 0, 0, 0, 0, 0, 0]);

        // Designator
        decoder.put(b'@');

        // First sixel row (y=0-5)
        decoder.put(b'~');
        decoder.put(b'/'); // Row separator
                           // Second sixel row (y=6-11)
        decoder.put(b'~');

        let font = decoder.finish().unwrap();
        let glyph = font.get_glyph(0).unwrap();

        // Check pixels in both sixel rows
        assert!(glyph.get_pixel(0, 0));
        assert!(glyph.get_pixel(0, 5));
        assert!(glyph.get_pixel(0, 6));
        assert!(glyph.get_pixel(0, 11));
    }
}
