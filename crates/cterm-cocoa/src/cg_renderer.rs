//! CoreGraphics-based terminal renderer
//!
//! Renders terminal content using CoreGraphics for text drawing.
//! This is simpler than Metal but sufficient for basic functionality.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_app_kit::{NSFont, NSGraphicsContext};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

use cterm_core::color::{Color, Rgb};
use cterm_core::Terminal;
use cterm_ui::theme::Theme;

/// CoreGraphics renderer for terminal display
pub struct CGRenderer {
    font: Retained<NSFont>,
    theme: Theme,
    cell_width: f64,
    cell_height: f64,
    baseline_offset: f64,
}

impl CGRenderer {
    /// Create a new CoreGraphics renderer
    pub fn new(_mtm: MainThreadMarker, font_name: &str, font_size: f64, theme: &Theme) -> Self {
        // Try to get the specified font, fall back to Menlo
        let font = NSFont::fontWithName_size(&NSString::from_str(font_name), font_size)
            .or_else(|| NSFont::fontWithName_size(&NSString::from_str("Menlo"), font_size))
            .unwrap_or_else(|| NSFont::monospacedSystemFontOfSize_weight(font_size, 0.0));

        // Calculate cell dimensions using font metrics
        let cell_width = Self::get_advance_for_glyph(&font);
        let cell_height = font_size * 1.2; // Line height
        let baseline_offset = font_size * 0.2; // Approximate descender

        Self {
            font,
            theme: theme.clone(),
            cell_width,
            cell_height,
            baseline_offset,
        }
    }

    /// Get the advance width for a character
    fn get_advance_for_glyph(font: &NSFont) -> f64 {
        // Use 'M' width as cell width for monospace
        let advancement: NSSize = unsafe {
            let glyph: u32 = msg_send![font, glyphWithName: &*NSString::from_str("M")];
            msg_send![font, advancementForGlyph: glyph]
        };
        if advancement.width > 0.0 {
            advancement.width
        } else {
            // Fallback: estimate based on font size
            font.pointSize() * 0.6
        }
    }

    /// Get cell dimensions
    pub fn cell_size(&self) -> (f64, f64) {
        (self.cell_width, self.cell_height)
    }

    /// Render the terminal content
    pub fn render(&self, terminal: &Terminal, bounds: NSRect) {
        let Some(_context) = NSGraphicsContext::currentContext() else {
            log::warn!("No graphics context");
            return;
        };

        let screen = terminal.screen();
        let cols = screen.width();
        let rows = screen.height();

        // Draw background
        self.draw_background(bounds);

        // Draw cells
        for row in 0..rows {
            for col in 0..cols {
                if let Some(cell) = screen.get_cell(row, col) {
                    let x = col as f64 * self.cell_width;
                    let y = row as f64 * self.cell_height;

                    // Draw cell background if not default
                    if !cell.bg.is_default() {
                        self.draw_cell_background(x, y, &cell.bg);
                    }

                    // Draw character
                    if cell.c != ' ' && cell.c != '\0' {
                        self.draw_char(cell.c, x, y, &cell.fg);
                    }
                }
            }
        }

        // Draw cursor
        let cursor = &screen.cursor;
        if cursor.visible {
            let cursor_x = cursor.col as f64 * self.cell_width;
            let cursor_y = cursor.row as f64 * self.cell_height;
            self.draw_cursor(cursor_x, cursor_y);
        }
    }

    fn draw_background(&self, bounds: NSRect) {
        let bg = &self.theme.colors.background;
        unsafe {
            let color = Self::ns_color(bg.r, bg.g, bg.b);
            let _: () = msg_send![&*color, setFill];
            let _: () = msg_send![class!(NSBezierPath), fillRect: bounds];
        }
    }

    fn draw_cell_background(&self, x: f64, y: f64, color: &Color) {
        let rgb = self.color_to_rgb(color);
        let rect = NSRect::new(
            NSPoint::new(x, y),
            NSSize::new(self.cell_width, self.cell_height),
        );
        unsafe {
            let ns_color = Self::ns_color(rgb.r, rgb.g, rgb.b);
            let _: () = msg_send![&*ns_color, setFill];
            let _: () = msg_send![class!(NSBezierPath), fillRect: rect];
        }
    }

    fn draw_char(&self, ch: char, x: f64, y: f64, color: &Color) {
        let rgb = self.color_to_rgb(color);
        let text = NSString::from_str(&ch.to_string());

        unsafe {
            let ns_color = Self::ns_color(rgb.r, rgb.g, rgb.b);

            // Use the actual string keys for NSAttributedString attributes
            let font_key = NSString::from_str("NSFont");
            let color_key = NSString::from_str("NSColor");

            let keys: [&AnyObject; 2] = [
                std::mem::transmute::<&NSString, &AnyObject>(&font_key),
                std::mem::transmute::<&NSString, &AnyObject>(&color_key),
            ];
            let values: [&AnyObject; 2] = [&*self.font, &*ns_color];

            let dict: Retained<AnyObject> = msg_send![
                class!(NSDictionary),
                dictionaryWithObjects: values.as_ptr(),
                forKeys: keys.as_ptr(),
                count: 2usize
            ];

            // Draw at position (y is flipped, so add cell_height - baseline_offset)
            let point = NSPoint::new(x, y + self.cell_height - self.baseline_offset);
            let _: () = msg_send![&*text, drawAtPoint: point, withAttributes: &*dict];
        }
    }

    fn draw_cursor(&self, x: f64, y: f64) {
        let cursor_color = &self.theme.colors.cursor;
        let rect = NSRect::new(
            NSPoint::new(x, y),
            NSSize::new(self.cell_width, self.cell_height),
        );
        unsafe {
            let color = Self::ns_color_alpha(cursor_color.r, cursor_color.g, cursor_color.b, 0.7);
            let _: () = msg_send![&*color, setFill];
            let _: () = msg_send![class!(NSBezierPath), fillRect: rect];
        }
    }

    fn ns_color(r: u8, g: u8, b: u8) -> Retained<AnyObject> {
        Self::ns_color_alpha(r, g, b, 1.0)
    }

    fn ns_color_alpha(r: u8, g: u8, b: u8, a: f64) -> Retained<AnyObject> {
        unsafe {
            msg_send![
                class!(NSColor),
                colorWithRed: r as f64 / 255.0,
                green: g as f64 / 255.0,
                blue: b as f64 / 255.0,
                alpha: a
            ]
        }
    }

    fn color_to_rgb(&self, color: &Color) -> Rgb {
        match color {
            Color::Default => self.theme.colors.foreground,
            Color::Rgb(rgb) => *rgb,
            Color::Ansi(ansi) => self.theme.colors.ansi[*ansi as usize],
            Color::Indexed(idx) => self.index_to_rgb(*idx),
        }
    }

    fn index_to_rgb(&self, idx: u8) -> Rgb {
        match idx {
            // First 16 are ANSI colors
            0..=15 => self.theme.colors.ansi[idx as usize],
            // 16-231 are a 6x6x6 color cube
            16..=231 => {
                let n = idx - 16;
                let b = (n % 6) * 51;
                let g = ((n / 6) % 6) * 51;
                let r = (n / 36) * 51;
                Rgb::new(r, g, b)
            }
            // 232-255 are grayscale
            232..=255 => {
                let gray = (idx - 232) * 10 + 8;
                Rgb::new(gray, gray, gray)
            }
        }
    }

    /// Update theme colors
    pub fn set_theme(&mut self, theme: &Theme) {
        self.theme = theme.clone();
    }
}
