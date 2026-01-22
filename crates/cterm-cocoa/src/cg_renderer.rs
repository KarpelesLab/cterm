//! CoreGraphics-based terminal renderer
//!
//! Renders terminal content using CoreGraphics for text drawing.
//! This is simpler than Metal but sufficient for basic functionality.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_app_kit::{NSFont, NSGraphicsContext};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

use cterm_core::cell::CellAttrs;
use cterm_core::color::{Color, Rgb};
use cterm_core::drcs::DrcsGlyph;
use cterm_core::TerminalImage;
use cterm_core::Terminal;
use cterm_ui::theme::Theme;

/// CoreGraphics renderer for terminal display
pub struct CGRenderer {
    font: Retained<NSFont>,
    theme: Theme,
    cell_width: f64,
    cell_height: f64,
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

        log::debug!(
            "CGRenderer: font_size={}, cell_width={}, cell_height={}",
            font_size,
            cell_width,
            cell_height
        );

        Self {
            font,
            theme: theme.clone(),
            cell_width,
            cell_height,
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
            // Get absolute line for scrollback access and selection checking
            let absolute_line = screen.visible_row_to_absolute_line(row);

            for col in 0..cols {
                if let Some(cell) = screen.get_cell_with_scrollback(absolute_line, col) {
                    let x = col as f64 * self.cell_width;
                    let y = row as f64 * self.cell_height;

                    // Check if cell is selected
                    let is_selected = screen.is_selected(absolute_line, col);

                    // XOR selection with INVERSE attribute to determine if colors should be inverted
                    let is_inverted = cell.attrs.contains(CellAttrs::INVERSE) != is_selected;

                    // Determine actual foreground and background colors
                    let (fg_color, bg_color) = if is_inverted {
                        // Inverted: swap foreground and background
                        let fg = if cell.bg.is_default() {
                            self.theme.colors.background
                        } else {
                            self.color_to_rgb(&cell.bg)
                        };
                        let bg = if cell.fg.is_default() {
                            self.theme.colors.foreground
                        } else {
                            self.color_to_rgb(&cell.fg)
                        };
                        (fg, bg)
                    } else {
                        (self.color_to_rgb(&cell.fg), self.color_to_rgb(&cell.bg))
                    };

                    // Draw cell background if not default or if selected/inverted
                    if !cell.bg.is_default() || is_inverted || is_selected {
                        self.draw_cell_background_rgb(x, y, &bg_color);
                    }

                    // Draw character
                    if cell.c != ' ' && cell.c != '\0' {
                        // Check if this should be a DRCS glyph
                        if let Some(glyph) = screen.get_drcs_for_char(cell.c) {
                            self.draw_drcs_glyph(glyph, x, y, &fg_color);
                        } else {
                            self.draw_char_rgb(cell.c, x, y, &fg_color);
                        }
                    }
                }
            }
        }

        // Draw images (Sixel, etc.)
        self.render_images(screen);

        // Draw cursor (only when not scrolled back)
        let cursor = &screen.cursor;
        if cursor.visible && screen.scroll_offset == 0 {
            let cursor_x = cursor.col as f64 * self.cell_width;
            let cursor_y = cursor.row as f64 * self.cell_height;
            self.draw_cursor(cursor_x, cursor_y);
        }
    }

    /// Render terminal images (Sixel graphics, etc.)
    fn render_images(&self, screen: &cterm_core::Screen) {
        for image in screen.visible_images() {
            if let Some(visible_row) = screen.image_visible_row(image) {
                let x = image.col as f64 * self.cell_width;
                let y = visible_row as f64 * self.cell_height;

                // Calculate display size (preserve aspect ratio, fit to pixel dimensions)
                let width = image.pixel_width as f64;
                let height = image.pixel_height as f64;

                self.draw_image(image, x, y, width, height);
            }
        }
    }

    /// Draw a terminal image at the specified position
    fn draw_image(&self, image: &TerminalImage, x: f64, y: f64, width: f64, height: f64) {
        // CoreGraphics FFI declarations
        #[repr(C)]
        #[derive(Copy, Clone)]
        struct CGPoint {
            x: f64,
            y: f64,
        }

        #[repr(C)]
        #[derive(Copy, Clone)]
        struct CGSize {
            width: f64,
            height: f64,
        }

        #[repr(C)]
        #[derive(Copy, Clone)]
        struct CGRect {
            origin: CGPoint,
            size: CGSize,
        }

        extern "C" {
            fn CGDataProviderCreateWithData(
                info: *mut std::ffi::c_void,
                data: *const u8,
                size: usize,
                release_callback: *const std::ffi::c_void,
            ) -> *mut std::ffi::c_void;

            fn CGImageCreate(
                width: usize,
                height: usize,
                bits_per_component: usize,
                bits_per_pixel: usize,
                bytes_per_row: usize,
                color_space: *mut std::ffi::c_void,
                bitmap_info: u32,
                provider: *mut std::ffi::c_void,
                decode: *const f64,
                should_interpolate: bool,
                intent: u32,
            ) -> *mut std::ffi::c_void;

            fn CGContextDrawImage(
                context: *mut std::ffi::c_void,
                rect: CGRect,
                image: *mut std::ffi::c_void,
            );

            fn CGImageRelease(image: *mut std::ffi::c_void);
            fn CGDataProviderRelease(provider: *mut std::ffi::c_void);
            fn CGColorSpaceCreateDeviceRGB() -> *mut std::ffi::c_void;
            fn CGColorSpaceRelease(color_space: *mut std::ffi::c_void);
            fn CGContextSaveGState(context: *mut std::ffi::c_void);
            fn CGContextRestoreGState(context: *mut std::ffi::c_void);
            fn CGContextTranslateCTM(context: *mut std::ffi::c_void, tx: f64, ty: f64);
            fn CGContextScaleCTM(context: *mut std::ffi::c_void, sx: f64, sy: f64);
        }

        unsafe {
            let data_ptr = image.data.as_ptr();
            let data_len = image.data.len();

            let cg_color_space = CGColorSpaceCreateDeviceRGB();
            if cg_color_space.is_null() {
                log::warn!("Failed to create CGColorSpace");
                return;
            }

            let provider = CGDataProviderCreateWithData(
                std::ptr::null_mut(),
                data_ptr,
                data_len,
                std::ptr::null(),
            );

            if provider.is_null() {
                log::warn!("Failed to create CGDataProvider for image");
                CGColorSpaceRelease(cg_color_space);
                return;
            }

            // Create CGImage
            // kCGImageAlphaLast = 3, kCGBitmapByteOrderDefault = 0
            const K_CG_IMAGE_ALPHA_LAST: u32 = 3;
            let cg_image = CGImageCreate(
                image.pixel_width,
                image.pixel_height,
                8,                          // bits per component
                32,                         // bits per pixel
                image.pixel_width * 4,      // bytes per row
                cg_color_space,
                K_CG_IMAGE_ALPHA_LAST,
                provider,
                std::ptr::null(),           // no decode array
                true,                       // interpolate
                0,                          // rendering intent (default)
            );

            CGDataProviderRelease(provider);
            CGColorSpaceRelease(cg_color_space);

            if cg_image.is_null() {
                log::warn!("Failed to create CGImage");
                return;
            }

            // Get current graphics context
            if let Some(context) = NSGraphicsContext::currentContext() {
                let cg_context: *mut std::ffi::c_void = msg_send![&context, CGContext];

                if !cg_context.is_null() {
                    CGContextSaveGState(cg_context);

                    // In a flipped view, we need to flip the image back
                    // Since our view is flipped (origin at top-left), but CGImage
                    // draws with origin at bottom-left, we need to flip vertically
                    CGContextTranslateCTM(cg_context, x, y + height);
                    CGContextScaleCTM(cg_context, 1.0, -1.0);

                    let draw_rect = CGRect {
                        origin: CGPoint { x: 0.0, y: 0.0 },
                        size: CGSize { width, height },
                    };

                    CGContextDrawImage(cg_context, draw_rect, cg_image);
                    CGContextRestoreGState(cg_context);
                }
            }

            CGImageRelease(cg_image);
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
        self.draw_cell_background_rgb(x, y, &rgb);
    }

    fn draw_cell_background_rgb(&self, x: f64, y: f64, rgb: &Rgb) {
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
        self.draw_char_rgb(ch, x, y, &rgb);
    }

    fn draw_char_rgb(&self, ch: char, x: f64, y: f64, rgb: &Rgb) {
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

            // In a flipped view, drawAtPoint places text with point as top-left of the text
            let point = NSPoint::new(x, y);
            let _: () = msg_send![&*text, drawAtPoint: point, withAttributes: &*dict];
        }
    }

    /// Draw a DRCS (soft font) glyph
    fn draw_drcs_glyph(&self, glyph: &DrcsGlyph, x: f64, y: f64, rgb: &Rgb) {
        // Calculate scaling factors to fit glyph into cell
        let scale_x = self.cell_width / glyph.width as f64;
        let scale_y = self.cell_height / glyph.height as f64;

        unsafe {
            let ns_color = Self::ns_color(rgb.r, rgb.g, rgb.b);
            let _: () = msg_send![&*ns_color, setFill];

            // Draw each pixel of the glyph as a small rectangle
            for gy in 0..glyph.height {
                for gx in 0..glyph.width {
                    if glyph.get_pixel(gx, gy) {
                        let px = x + gx as f64 * scale_x;
                        let py = y + gy as f64 * scale_y;
                        let rect = NSRect::new(
                            NSPoint::new(px, py),
                            NSSize::new(scale_x.ceil(), scale_y.ceil()),
                        );
                        let _: () = msg_send![class!(NSBezierPath), fillRect: rect];
                    }
                }
            }
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

    /// Render IME marked text (composition text) at cursor position
    pub fn render_marked_text(&self, text: &str, cursor_row: usize, cursor_col: usize) {
        if text.is_empty() {
            return;
        }

        let x = cursor_col as f64 * self.cell_width;
        let y = cursor_row as f64 * self.cell_height;

        // Calculate the width of the marked text
        let char_count: usize = text.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum();
        let text_width = char_count as f64 * self.cell_width;

        // Draw background for marked text (slightly different from regular background)
        let bg_rect = NSRect::new(
            NSPoint::new(x, y),
            NSSize::new(text_width, self.cell_height),
        );
        unsafe {
            // Use a light yellow background for marked text
            let bg_color = Self::ns_color_alpha(255, 255, 200, 0.9);
            let _: () = msg_send![&*bg_color, setFill];
            let _: () = msg_send![class!(NSBezierPath), fillRect: bg_rect];
        }

        // Draw the marked text
        let ns_text = NSString::from_str(text);
        unsafe {
            // Use dark text color for marked text
            let text_color = Self::ns_color(0, 0, 0);

            let font_key = NSString::from_str("NSFont");
            let color_key = NSString::from_str("NSColor");

            let keys: [&AnyObject; 2] = [
                std::mem::transmute::<&NSString, &AnyObject>(&font_key),
                std::mem::transmute::<&NSString, &AnyObject>(&color_key),
            ];
            let values: [&AnyObject; 2] = [&*self.font, &*text_color];

            let dict: Retained<AnyObject> = msg_send![
                class!(NSDictionary),
                dictionaryWithObjects: values.as_ptr(),
                forKeys: keys.as_ptr(),
                count: 2usize
            ];

            let point = NSPoint::new(x, y);
            let _: () = msg_send![&*ns_text, drawAtPoint: point, withAttributes: &*dict];
        }

        // Draw underline to indicate composition
        let underline_rect = NSRect::new(
            NSPoint::new(x, y + self.cell_height - 2.0),
            NSSize::new(text_width, 2.0),
        );
        unsafe {
            let underline_color = Self::ns_color(0, 100, 200);
            let _: () = msg_send![&*underline_color, setFill];
            let _: () = msg_send![class!(NSBezierPath), fillRect: underline_rect];
        }
    }
}
