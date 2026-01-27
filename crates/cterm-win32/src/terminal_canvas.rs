//! Terminal canvas with Direct2D rendering
//!
//! Hardware-accelerated terminal rendering using Direct2D and DirectWrite.

use std::collections::HashMap;

use cterm_core::color::{Color, Rgb};
use cterm_core::{Cell, CellAttrs, Screen, Selection};
use cterm_ui::theme::Theme;
use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_POINT_2F, D2D_RECT_F,
    D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1RenderTarget,
    ID2D1SolidColorBrush, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_FACTORY_OPTIONS,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout,
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_BOLD, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_TEXT_METRICS,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

use crate::dpi::DpiInfo;

/// Cell dimensions
#[derive(Debug, Clone, Copy)]
pub struct CellDimensions {
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
}

impl Default for CellDimensions {
    fn default() -> Self {
        Self {
            width: 8.0,
            height: 16.0,
            baseline: 12.0,
        }
    }
}

/// Terminal renderer using Direct2D
pub struct TerminalRenderer {
    factory: ID2D1Factory,
    dwrite_factory: IDWriteFactory,
    render_target: Option<ID2D1HwndRenderTarget>,
    text_format: Option<IDWriteTextFormat>,
    text_format_bold: Option<IDWriteTextFormat>,
    cell_dims: CellDimensions,
    font_size: f32,
    font_family: String,
    theme: Theme,
    dpi: DpiInfo,
    brush_cache: HashMap<u32, ID2D1SolidColorBrush>,
    hwnd: HWND,
    /// Optional background color override (from template)
    background_override: Option<Rgb>,
}

impl TerminalRenderer {
    /// Create a new terminal renderer
    pub fn new(
        hwnd: HWND,
        theme: &Theme,
        font_family: &str,
        font_size: f32,
    ) -> windows::core::Result<Self> {
        // Create D2D factory
        let factory: ID2D1Factory = unsafe {
            D2D1CreateFactory(
                D2D1_FACTORY_TYPE_SINGLE_THREADED,
                Some(&D2D1_FACTORY_OPTIONS::default()),
            )?
        };

        // Create DirectWrite factory
        let dwrite_factory: IDWriteFactory =
            unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };

        let mut renderer = Self {
            factory,
            dwrite_factory,
            render_target: None,
            text_format: None,
            text_format_bold: None,
            cell_dims: CellDimensions::default(),
            font_size,
            font_family: font_family.to_string(),
            theme: theme.clone(),
            dpi: DpiInfo::system(),
            brush_cache: HashMap::new(),
            hwnd,
            background_override: None,
        };

        renderer.create_device_resources()?;

        Ok(renderer)
    }

    /// Create device-dependent resources
    fn create_device_resources(&mut self) -> windows::core::Result<()> {
        // Get window size
        let mut rect = RECT::default();
        unsafe { GetClientRect(self.hwnd, &mut rect)? };

        // Ensure minimum size of 1x1 to avoid D2D errors
        let width = ((rect.right - rect.left) as u32).max(1);
        let height = ((rect.bottom - rect.top) as u32).max(1);

        // Get DPI
        self.dpi = DpiInfo::for_window(self.hwnd);

        // Create render target properties
        let render_props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: self.dpi.dpi as f32,
            dpiY: self.dpi.dpi as f32,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };

        let hwnd_props = D2D1_HWND_RENDER_TARGET_PROPERTIES {
            hwnd: self.hwnd,
            pixelSize: D2D_SIZE_U { width, height },
            presentOptions: D2D1_PRESENT_OPTIONS_NONE,
        };

        // Create HWND render target
        let render_target = unsafe {
            self.factory
                .CreateHwndRenderTarget(&render_props, &hwnd_props)?
        };

        unsafe {
            render_target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);
            render_target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }

        self.render_target = Some(render_target);
        self.brush_cache.clear();

        // Create text format
        self.create_text_format()?;

        Ok(())
    }

    /// Create text format and measure cell dimensions
    fn create_text_format(&mut self) -> windows::core::Result<()> {
        let scaled_font_size = self.dpi.scale_f32(self.font_size);

        // Locale for DirectWrite (empty string = user default)
        let locale: Vec<u16> = "".encode_utf16().chain(std::iter::once(0)).collect();

        // Try each font in the comma-separated list until one works
        let font_families: Vec<&str> = self.font_family.split(',').map(|s| s.trim()).collect();

        let mut text_format = None;
        let mut text_format_bold = None;

        for font_family in &font_families {
            let font_family_wide: Vec<u16> = font_family
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            // Try to create normal text format
            let result = unsafe {
                self.dwrite_factory.CreateTextFormat(
                    PCWSTR(font_family_wide.as_ptr()),
                    None,
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    scaled_font_size,
                    PCWSTR(locale.as_ptr()),
                )
            };

            if let Ok(tf) = result {
                // Also create bold variant
                let bold_result = unsafe {
                    self.dwrite_factory.CreateTextFormat(
                        PCWSTR(font_family_wide.as_ptr()),
                        None,
                        DWRITE_FONT_WEIGHT_BOLD,
                        DWRITE_FONT_STYLE_NORMAL,
                        DWRITE_FONT_STRETCH_NORMAL,
                        scaled_font_size,
                        PCWSTR(locale.as_ptr()),
                    )
                };

                if let Ok(tfb) = bold_result {
                    text_format = Some(tf);
                    text_format_bold = Some(tfb);
                    log::info!("Using font: {}", font_family);
                    break;
                }
            }
        }

        // If no font worked, return error
        let text_format = text_format.ok_or_else(|| {
            let msg = format!("No suitable font found in: {}", self.font_family);
            windows::core::Error::new(windows::core::HRESULT(-1), msg)
        })?;
        let text_format_bold = text_format_bold.unwrap();

        // Measure cell dimensions using 'M' character
        let test_char: Vec<u16> = "M".encode_utf16().collect();
        let layout: IDWriteTextLayout = unsafe {
            self.dwrite_factory
                .CreateTextLayout(&test_char, &text_format, 1000.0, 1000.0)?
        };

        let mut metrics = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut metrics)? };

        self.cell_dims = CellDimensions {
            width: metrics.width,
            height: metrics.height * 1.1,    // Add some line spacing
            baseline: metrics.height * 0.85, // Approximate baseline
        };

        self.text_format = Some(text_format);
        self.text_format_bold = Some(text_format_bold);

        Ok(())
    }

    /// Get or create a solid color brush
    fn get_brush(&mut self, color: Rgb) -> windows::core::Result<ID2D1SolidColorBrush> {
        let key = (color.r as u32) << 16 | (color.g as u32) << 8 | (color.b as u32);

        if let Some(brush) = self.brush_cache.get(&key) {
            return Ok(brush.clone());
        }

        // Clone and cast to parent interface to access methods
        let rt = self.render_target.clone().unwrap();
        let base: ID2D1RenderTarget = rt.cast()?;
        let d2d_color = rgb_to_d2d_color(color);
        let brush = unsafe { base.CreateSolidColorBrush(&d2d_color, None)? };

        self.brush_cache.insert(key, brush.clone());
        Ok(brush)
    }

    /// Resize the render target
    pub fn resize(&mut self, width: u32, height: u32) -> windows::core::Result<()> {
        if let Some(ref rt) = self.render_target {
            let size = D2D_SIZE_U { width, height };
            unsafe { rt.Resize(&size)? };
        }
        Ok(())
    }

    /// Handle DPI change
    pub fn update_dpi(&mut self, dpi: u32) -> windows::core::Result<()> {
        self.dpi = DpiInfo::from_dpi(dpi);
        self.create_device_resources()
    }

    /// Get the cell dimensions
    pub fn cell_dimensions(&self) -> CellDimensions {
        self.cell_dims
    }

    /// Set an optional background color override (hex string like "#1a1b26")
    pub fn set_background_override(&mut self, color: Option<&str>) {
        self.background_override = color.and_then(|hex| {
            let hex = hex.trim_start_matches('#');
            if hex.len() == 6 {
                let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
                let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
                let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
                Some(Rgb::new(r, g, b))
            } else {
                None
            }
        });
    }

    /// Calculate terminal size in cells
    pub fn terminal_size(&self, width: u32, height: u32) -> (usize, usize) {
        let cols = (width as f32 / self.cell_dims.width).floor() as usize;
        let rows = (height as f32 / self.cell_dims.height).floor() as usize;
        (cols.max(1), rows.max(1))
    }

    /// Render the terminal screen
    pub fn render(&mut self, screen: &Screen) -> windows::core::Result<()> {
        if self.render_target.is_none() {
            return Ok(());
        }

        // Begin drawing
        unsafe {
            let rt = self.render_target.as_ref().unwrap();
            rt.BeginDraw();

            // Clear with background color (use override if set)
            let bg = self
                .background_override
                .as_ref()
                .unwrap_or(&self.theme.colors.background);
            let bg_color = rgb_to_d2d_color(*bg);
            rt.Clear(Some(&bg_color));
        }

        // Draw grid cells
        self.draw_grid(screen)?;

        // Draw selection
        if let Some(selection) = screen.selection.clone() {
            self.draw_selection(screen, &selection)?;
        }

        // Draw cursor
        self.draw_cursor(screen)?;

        // End drawing
        unsafe {
            let rt = self.render_target.as_ref().unwrap();
            rt.EndDraw(None, None)?;
        }

        Ok(())
    }

    /// Draw the terminal grid
    fn draw_grid(&mut self, screen: &Screen) -> windows::core::Result<()> {
        let grid = screen.grid();
        let scroll_offset = screen.scroll_offset;
        let rows = grid.height();
        let cols = grid.width();

        for row in 0..rows {
            let grid_row = if scroll_offset > 0 {
                row.saturating_sub(scroll_offset)
            } else {
                row
            };

            if grid_row >= rows {
                continue;
            }

            for col in 0..cols {
                if let Some(cell) = grid.get(grid_row, col) {
                    self.draw_cell(row, col, cell)?;
                }
            }
        }

        Ok(())
    }

    /// Draw a single cell
    fn draw_cell(&mut self, row: usize, col: usize, cell: &Cell) -> windows::core::Result<()> {
        let x = col as f32 * self.cell_dims.width;
        let y = row as f32 * self.cell_dims.height;

        let attrs = cell.attrs;
        let (fg, bg) = self.resolve_colors(cell);

        // Get brushes first (this mutably borrows self temporarily)
        let bg_brush = if bg != self.theme.colors.background {
            Some(self.get_brush(bg)?)
        } else {
            None
        };

        let c = cell.c;
        let needs_fg = c != ' ' && c != '\0'
            || attrs.has_underline()
            || attrs.contains(CellAttrs::STRIKETHROUGH);
        let fg_brush = if needs_fg {
            Some(self.get_brush(fg)?)
        } else {
            None
        };

        // Clone and cast to parent interface to access methods
        let rt = self.render_target.clone().unwrap();
        let base: ID2D1RenderTarget = rt.cast()?;

        // Draw background if not default
        if let Some(ref brush) = bg_brush {
            let rect = D2D_RECT_F {
                left: x,
                top: y,
                right: x + self.cell_dims.width,
                bottom: y + self.cell_dims.height,
            };
            unsafe { base.FillRectangle(&rect, brush) };
        }

        // Draw character
        if c != ' ' && c != '\0' {
            let text_format = if attrs.contains(CellAttrs::BOLD) {
                self.text_format_bold.as_ref().unwrap()
            } else {
                self.text_format.as_ref().unwrap()
            };

            let mut buf = [0u16; 2];
            let text: &[u16] = c.encode_utf16(&mut buf);

            let layout: IDWriteTextLayout = unsafe {
                self.dwrite_factory.CreateTextLayout(
                    text,
                    text_format,
                    self.cell_dims.width * 2.0, // Allow for wide chars
                    self.cell_dims.height,
                )?
            };

            let origin = D2D_POINT_2F { x, y };
            unsafe {
                base.DrawTextLayout(
                    origin,
                    &layout,
                    fg_brush.as_ref().unwrap(),
                    Default::default(),
                )
            };
        }

        // Draw underline
        if attrs.has_underline() {
            let underline_y = y + self.cell_dims.baseline + 2.0;
            unsafe {
                base.DrawLine(
                    D2D_POINT_2F { x, y: underline_y },
                    D2D_POINT_2F {
                        x: x + self.cell_dims.width,
                        y: underline_y,
                    },
                    fg_brush.as_ref().unwrap(),
                    1.0,
                    None,
                )
            };
        }

        // Draw strikethrough
        if attrs.contains(CellAttrs::STRIKETHROUGH) {
            let strike_y = y + self.cell_dims.height / 2.0;
            unsafe {
                base.DrawLine(
                    D2D_POINT_2F { x, y: strike_y },
                    D2D_POINT_2F {
                        x: x + self.cell_dims.width,
                        y: strike_y,
                    },
                    fg_brush.as_ref().unwrap(),
                    1.0,
                    None,
                )
            };
        }

        Ok(())
    }

    /// Resolve foreground and background colors from a cell
    fn resolve_colors(&self, cell: &Cell) -> (Rgb, Rgb) {
        let palette = &self.theme.colors;

        let mut fg = cell.fg.to_rgb(palette);
        let mut bg = if cell.bg == Color::Default {
            palette.background
        } else {
            cell.bg.to_rgb(palette)
        };

        // Handle inverse
        if cell.attrs.contains(CellAttrs::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }

        // Handle dim
        if cell.attrs.contains(CellAttrs::DIM) {
            fg = Rgb::new(fg.r / 2, fg.g / 2, fg.b / 2);
        }

        (fg, bg)
    }

    /// Draw selection highlight
    fn draw_selection(
        &mut self,
        screen: &Screen,
        selection: &Selection,
    ) -> windows::core::Result<()> {
        let selection_color = self.theme.colors.selection;
        let brush = self.get_brush(selection_color)?;

        let (start, end) = selection.ordered();
        let rows = screen.grid().height();
        let cols = screen.grid().width();

        // Clone and cast to parent interface to access methods
        let rt = self.render_target.clone().unwrap();
        let base: ID2D1RenderTarget = rt.cast()?;

        for line in start.line..=end.line {
            if line >= rows {
                continue;
            }

            let start_col = if line == start.line { start.col } else { 0 };
            let end_col = if line == end.line {
                end.col
            } else {
                cols.saturating_sub(1)
            };

            let x = start_col as f32 * self.cell_dims.width;
            let y = line as f32 * self.cell_dims.height;
            let width = ((end_col - start_col + 1) as f32) * self.cell_dims.width;

            let rect = D2D_RECT_F {
                left: x,
                top: y,
                right: x + width,
                bottom: y + self.cell_dims.height,
            };

            unsafe { base.FillRectangle(&rect, &brush) };
        }

        Ok(())
    }

    /// Draw the cursor
    fn draw_cursor(&mut self, screen: &Screen) -> windows::core::Result<()> {
        // Check DECTCEM mode for cursor visibility
        if !screen.modes.show_cursor {
            return Ok(());
        }

        let cursor = &screen.cursor;

        let x = cursor.col as f32 * self.cell_dims.width;
        let y = cursor.row as f32 * self.cell_dims.height;

        let cursor_color = self.theme.cursor.color;
        let brush = self.get_brush(cursor_color)?;

        let rect = D2D_RECT_F {
            left: x,
            top: y,
            right: x + self.cell_dims.width,
            bottom: y + self.cell_dims.height,
        };

        // Clone and cast to parent interface to access methods
        let rt = self.render_target.clone().unwrap();
        let base: ID2D1RenderTarget = rt.cast()?;

        // Draw filled block cursor
        unsafe {
            base.FillRectangle(&rect, &brush);
        }

        // Draw the character under cursor with inverted color
        let grid = screen.grid();
        if let Some(cell) = grid.get(cursor.row, cursor.col) {
            let c = cell.c;

            if c != ' ' && c != '\0' {
                let text_color = self.theme.cursor.text_color;
                let text_brush = self.get_brush(text_color)?;

                let text_format = self.text_format.as_ref().unwrap();
                let mut buf = [0u16; 2];
                let text: &[u16] = c.encode_utf16(&mut buf);

                let layout: IDWriteTextLayout = unsafe {
                    self.dwrite_factory.CreateTextLayout(
                        text,
                        text_format,
                        self.cell_dims.width * 2.0,
                        self.cell_dims.height,
                    )?
                };

                let origin = D2D_POINT_2F { x, y };
                unsafe {
                    base.DrawTextLayout(origin, &layout, &text_brush, Default::default());
                }
            }
        }

        Ok(())
    }

    /// Update the theme
    pub fn set_theme(&mut self, theme: &Theme) {
        self.theme = theme.clone();
        self.brush_cache.clear();
    }

    /// Update font settings
    pub fn set_font(&mut self, family: &str, size: f32) -> windows::core::Result<()> {
        self.font_family = family.to_string();
        self.font_size = size;
        self.create_text_format()
    }

    /// Get current font size
    pub fn font_size(&self) -> f32 {
        self.font_size
    }

    /// Set font size only
    pub fn set_font_size(&mut self, size: f32) -> windows::core::Result<()> {
        self.font_size = size;
        self.create_text_format()
    }
}

/// Convert Rgb to D2D1_COLOR_F
fn rgb_to_d2d_color(rgb: Rgb) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: rgb.r as f32 / 255.0,
        g: rgb.g as f32 / 255.0,
        b: rgb.b as f32 / 255.0,
        a: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rgb_to_d2d_color() {
        let rgb = Rgb::new(255, 128, 0);
        let color = rgb_to_d2d_color(rgb);
        assert_eq!(color.r, 1.0);
        assert!((color.g - 0.5).abs() < 0.01);
        assert_eq!(color.b, 0.0);
        assert_eq!(color.a, 1.0);
    }

    #[test]
    fn test_cell_dimensions_default() {
        let dims = CellDimensions::default();
        assert!(dims.width > 0.0);
        assert!(dims.height > 0.0);
    }
}
