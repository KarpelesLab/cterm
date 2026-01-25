//! Custom-drawn tab bar using Direct2D
//!
//! Provides a tab bar similar to modern browsers with close buttons and indicators.

use cterm_core::color::Rgb;
use cterm_ui::theme::Theme;
use windows::core::Interface;
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_POINT_2F, D2D_RECT_F};
use windows::Win32::Graphics::Direct2D::{
    ID2D1HwndRenderTarget, ID2D1RenderTarget, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
};

use crate::dpi::DpiInfo;

/// Tab bar height in logical pixels
pub const TAB_BAR_HEIGHT: i32 = 32;
/// Tab padding
const TAB_PADDING: f32 = 12.0;
/// Close button size
const CLOSE_BUTTON_SIZE: f32 = 16.0;
/// Tab corner radius
const TAB_CORNER_RADIUS: f32 = 4.0;
/// New tab button width
const NEW_TAB_BUTTON_WIDTH: f32 = 32.0;
/// Minimum tab width
const MIN_TAB_WIDTH: f32 = 80.0;
/// Maximum tab width
const MAX_TAB_WIDTH: f32 = 200.0;

/// Information about a single tab
#[derive(Debug, Clone)]
pub struct TabInfo {
    pub id: u64,
    pub title: String,
    pub color: Option<Rgb>,
    pub has_bell: bool,
    pub is_active: bool,
}

/// Tab rectangle for hit testing
#[derive(Debug, Clone, Copy)]
pub struct TabRect {
    pub bounds: D2D_RECT_F,
    pub close_button: D2D_RECT_F,
}

/// Tab bar state and rendering
pub struct TabBar {
    tabs: Vec<TabInfo>,
    active_tab_id: Option<u64>,
    tab_rects: Vec<(u64, TabRect)>,
    new_tab_rect: D2D_RECT_F,
    theme: Theme,
    dpi: DpiInfo,
    hover_tab_id: Option<u64>,
    hover_close_button: bool,
    visible: bool,
}

impl TabBar {
    /// Create a new tab bar
    pub fn new(theme: &Theme) -> Self {
        Self {
            tabs: Vec::new(),
            active_tab_id: None,
            tab_rects: Vec::new(),
            new_tab_rect: D2D_RECT_F::default(),
            theme: theme.clone(),
            dpi: DpiInfo::default(),
            hover_tab_id: None,
            hover_close_button: false,
            visible: true,
        }
    }

    /// Get the tab bar height in physical pixels
    pub fn height(&self) -> i32 {
        if self.visible {
            self.dpi.scale(TAB_BAR_HEIGHT)
        } else {
            0
        }
    }

    /// Set visibility
    pub fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    /// Check if visible
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Update visibility based on tab count
    pub fn update_visibility(&mut self) {
        // Show tab bar only if there are multiple tabs
        self.visible = self.tabs.len() > 1;
    }

    /// Update DPI
    pub fn set_dpi(&mut self, dpi: DpiInfo) {
        self.dpi = dpi;
    }

    /// Add a tab
    pub fn add_tab(&mut self, id: u64, title: &str) {
        self.tabs.push(TabInfo {
            id,
            title: title.to_string(),
            color: None,
            has_bell: false,
            is_active: false,
        });
        self.update_visibility();
    }

    /// Remove a tab
    pub fn remove_tab(&mut self, id: u64) {
        self.tabs.retain(|t| t.id != id);
        self.tab_rects.retain(|(tid, _)| *tid != id);
        if self.active_tab_id == Some(id) {
            self.active_tab_id = self.tabs.first().map(|t| t.id);
        }
        self.update_visibility();
    }

    /// Set the active tab
    pub fn set_active(&mut self, id: u64) {
        self.active_tab_id = Some(id);
        for tab in &mut self.tabs {
            tab.is_active = tab.id == id;
        }
    }

    /// Set tab title
    pub fn set_title(&mut self, id: u64, title: &str) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            tab.title = title.to_string();
        }
    }

    /// Set tab color
    pub fn set_color(&mut self, id: u64, color: Option<Rgb>) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            tab.color = color;
        }
    }

    /// Set bell indicator
    pub fn set_bell(&mut self, id: u64, has_bell: bool) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) {
            tab.has_bell = has_bell;
        }
    }

    /// Clear bell indicator
    pub fn clear_bell(&mut self, id: u64) {
        self.set_bell(id, false);
    }

    /// Get tab count
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Set hover state
    pub fn set_hover(&mut self, tab_id: Option<u64>, on_close_button: bool) {
        self.hover_tab_id = tab_id;
        self.hover_close_button = on_close_button;
    }

    /// Hit test - returns (tab_id, is_close_button, is_new_tab_button)
    pub fn hit_test(&self, x: f32, y: f32) -> (Option<u64>, bool, bool) {
        // Check new tab button
        if point_in_rect(x, y, &self.new_tab_rect) {
            return (None, false, true);
        }

        // Check tabs (in reverse order so foreground tabs are hit first)
        for (id, rect) in self.tab_rects.iter().rev() {
            if point_in_rect(x, y, &rect.close_button) {
                return (Some(*id), true, false);
            }
            if point_in_rect(x, y, &rect.bounds) {
                return (Some(*id), false, false);
            }
        }

        (None, false, false)
    }

    /// Calculate tab layout
    fn calculate_layout(&mut self, width: f32) {
        self.tab_rects.clear();

        if self.tabs.is_empty() {
            return;
        }

        let height = self.dpi.scale_f32(TAB_BAR_HEIGHT as f32);
        let padding = self.dpi.scale_f32(TAB_PADDING);
        let close_size = self.dpi.scale_f32(CLOSE_BUTTON_SIZE);
        let new_tab_width = self.dpi.scale_f32(NEW_TAB_BUTTON_WIDTH);
        let min_tab_width = self.dpi.scale_f32(MIN_TAB_WIDTH);
        let max_tab_width = self.dpi.scale_f32(MAX_TAB_WIDTH);

        // Calculate available width for tabs
        let available_width = width - new_tab_width - padding;
        let tab_count = self.tabs.len() as f32;

        // Calculate tab width
        let tab_width = (available_width / tab_count)
            .max(min_tab_width)
            .min(max_tab_width);

        let mut x = padding;

        for tab in &self.tabs {
            let bounds = D2D_RECT_F {
                left: x,
                top: 2.0,
                right: x + tab_width - 2.0,
                bottom: height - 2.0,
            };

            // Close button in the right side of the tab
            let close_button = D2D_RECT_F {
                left: bounds.right - close_size - 4.0,
                top: (height - close_size) / 2.0,
                right: bounds.right - 4.0,
                bottom: (height + close_size) / 2.0,
            };

            self.tab_rects.push((
                tab.id,
                TabRect {
                    bounds,
                    close_button,
                },
            ));
            x += tab_width;
        }

        // New tab button
        self.new_tab_rect = D2D_RECT_F {
            left: x + 4.0,
            top: 2.0,
            right: x + new_tab_width - 4.0,
            bottom: height - 2.0,
        };
    }

    /// Render the tab bar
    pub fn render(
        &mut self,
        rt: &ID2D1HwndRenderTarget,
        dwrite: &IDWriteFactory,
        width: f32,
        text_format: &IDWriteTextFormat,
    ) -> windows::core::Result<()> {
        if !self.visible {
            return Ok(());
        }

        // Calculate layout
        self.calculate_layout(width);

        let height = self.dpi.scale_f32(TAB_BAR_HEIGHT as f32);

        // Cast to parent interface to access render methods
        let base: ID2D1RenderTarget = rt.cast()?;

        // Draw background
        let bg_rect = D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: width,
            bottom: height,
        };
        let bg_color = rgb_to_d2d_color(self.theme.ui.tab_bar_background);
        let bg_brush = unsafe { base.CreateSolidColorBrush(&bg_color, None)? };
        unsafe { base.FillRectangle(&bg_rect, &bg_brush) };

        // Draw each tab
        for tab in &self.tabs {
            self.render_tab(rt, dwrite, tab, text_format)?;
        }

        // Draw new tab button
        self.render_new_tab_button(rt)?;

        // Draw bottom border
        let border_color = rgb_to_d2d_color(self.theme.ui.border);
        let border_brush = unsafe { base.CreateSolidColorBrush(&border_color, None)? };
        unsafe {
            base.DrawLine(
                D2D_POINT_2F { x: 0.0, y: height },
                D2D_POINT_2F {
                    x: width,
                    y: height,
                },
                &border_brush,
                1.0,
                None,
            )
        };

        Ok(())
    }

    /// Render a single tab
    fn render_tab(
        &self,
        rt: &ID2D1HwndRenderTarget,
        dwrite: &IDWriteFactory,
        tab: &TabInfo,
        text_format: &IDWriteTextFormat,
    ) -> windows::core::Result<()> {
        // Cast to parent interface to access render methods
        let base: ID2D1RenderTarget = rt.cast()?;

        let (_, rect) = self
            .tab_rects
            .iter()
            .find(|(id, _)| *id == tab.id)
            .ok_or_else(|| windows::core::Error::from_hresult(windows::core::HRESULT(-1)))?;

        // Background
        let bg_color = if tab.is_active {
            if let Some(color) = tab.color {
                // Blend tab color with active background
                blend_colors(color, self.theme.ui.tab_active_background, 0.3)
            } else {
                self.theme.ui.tab_active_background
            }
        } else if self.hover_tab_id == Some(tab.id) {
            self.theme.ui.tab_active_background
        } else {
            if let Some(color) = tab.color {
                blend_colors(color, self.theme.ui.tab_inactive_background, 0.2)
            } else {
                self.theme.ui.tab_inactive_background
            }
        };

        let rounded_rect = D2D1_ROUNDED_RECT {
            rect: rect.bounds,
            radiusX: self.dpi.scale_f32(TAB_CORNER_RADIUS),
            radiusY: self.dpi.scale_f32(TAB_CORNER_RADIUS),
        };

        let bg_brush = unsafe { base.CreateSolidColorBrush(&rgb_to_d2d_color(bg_color), None)? };
        unsafe { base.FillRoundedRectangle(&rounded_rect, &bg_brush) };

        // Tab indicator bar for custom color
        if let Some(color) = tab.color {
            let indicator_rect = D2D_RECT_F {
                left: rect.bounds.left + 4.0,
                top: rect.bounds.bottom - 3.0,
                right: rect.bounds.right - 4.0,
                bottom: rect.bounds.bottom - 1.0,
            };
            let indicator_brush =
                unsafe { base.CreateSolidColorBrush(&rgb_to_d2d_color(color), None)? };
            unsafe { base.FillRectangle(&indicator_rect, &indicator_brush) };
        }

        // Title text
        let text_color = if tab.is_active {
            self.theme.ui.tab_active_text
        } else {
            self.theme.ui.tab_inactive_text
        };

        let text_brush =
            unsafe { base.CreateSolidColorBrush(&rgb_to_d2d_color(text_color), None)? };

        // Create text with bell indicator if needed
        let display_title = if tab.has_bell {
            format!("* {}", tab.title)
        } else {
            tab.title.clone()
        };

        let text_wide: Vec<u16> = display_title.encode_utf16().collect();
        let close_size = self.dpi.scale_f32(CLOSE_BUTTON_SIZE);
        let text_width = rect.bounds.right - rect.bounds.left - close_size - 8.0;

        let layout: IDWriteTextLayout = unsafe {
            dwrite.CreateTextLayout(
                &text_wide,
                text_format,
                text_width,
                rect.bounds.bottom - rect.bounds.top,
            )?
        };

        // Center text vertically
        unsafe {
            layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        }

        // Draw text
        let text_origin = D2D_POINT_2F {
            x: rect.bounds.left + 8.0,
            y: rect.bounds.top,
        };
        unsafe { base.DrawTextLayout(text_origin, &layout, &text_brush, Default::default()) };

        // Close button
        let close_hover = self.hover_tab_id == Some(tab.id) && self.hover_close_button;
        let close_color = if close_hover {
            Rgb::new(255, 100, 100)
        } else {
            text_color
        };
        let close_brush =
            unsafe { base.CreateSolidColorBrush(&rgb_to_d2d_color(close_color), None)? };

        // Draw X
        let padding = 4.0;
        unsafe {
            base.DrawLine(
                D2D_POINT_2F {
                    x: rect.close_button.left + padding,
                    y: rect.close_button.top + padding,
                },
                D2D_POINT_2F {
                    x: rect.close_button.right - padding,
                    y: rect.close_button.bottom - padding,
                },
                &close_brush,
                1.5,
                None,
            );
            base.DrawLine(
                D2D_POINT_2F {
                    x: rect.close_button.right - padding,
                    y: rect.close_button.top + padding,
                },
                D2D_POINT_2F {
                    x: rect.close_button.left + padding,
                    y: rect.close_button.bottom - padding,
                },
                &close_brush,
                1.5,
                None,
            );
        }

        Ok(())
    }

    /// Render the new tab button
    fn render_new_tab_button(&self, rt: &ID2D1HwndRenderTarget) -> windows::core::Result<()> {
        // Cast to parent interface to access render methods
        let base: ID2D1RenderTarget = rt.cast()?;

        let text_color = self.theme.ui.tab_inactive_text;
        let brush = unsafe { base.CreateSolidColorBrush(&rgb_to_d2d_color(text_color), None)? };

        // Draw + sign
        let cx = (self.new_tab_rect.left + self.new_tab_rect.right) / 2.0;
        let cy = (self.new_tab_rect.top + self.new_tab_rect.bottom) / 2.0;
        let size = 8.0;

        unsafe {
            // Horizontal line
            base.DrawLine(
                D2D_POINT_2F {
                    x: cx - size,
                    y: cy,
                },
                D2D_POINT_2F {
                    x: cx + size,
                    y: cy,
                },
                &brush,
                2.0,
                None,
            );
            // Vertical line
            base.DrawLine(
                D2D_POINT_2F {
                    x: cx,
                    y: cy - size,
                },
                D2D_POINT_2F {
                    x: cx,
                    y: cy + size,
                },
                &brush,
                2.0,
                None,
            );
        }

        Ok(())
    }

    /// Update theme
    pub fn set_theme(&mut self, theme: &Theme) {
        self.theme = theme.clone();
    }
}

/// Check if a point is inside a rectangle
fn point_in_rect(x: f32, y: f32, rect: &D2D_RECT_F) -> bool {
    x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
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

/// Blend two colors
fn blend_colors(color: Rgb, base: Rgb, amount: f32) -> Rgb {
    let r = ((color.r as f32 * amount) + (base.r as f32 * (1.0 - amount))) as u8;
    let g = ((color.g as f32 * amount) + (base.g as f32 * (1.0 - amount))) as u8;
    let b = ((color.b as f32 * amount) + (base.b as f32 * (1.0 - amount))) as u8;
    Rgb::new(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tab_bar_operations() {
        let theme = Theme::dark();
        let mut tab_bar = TabBar::new(&theme);

        // Initially empty
        assert_eq!(tab_bar.tab_count(), 0);
        assert!(!tab_bar.is_visible());

        // Add first tab
        tab_bar.add_tab(1, "Tab 1");
        assert_eq!(tab_bar.tab_count(), 1);
        assert!(!tab_bar.is_visible()); // Still hidden with one tab

        // Add second tab
        tab_bar.add_tab(2, "Tab 2");
        assert_eq!(tab_bar.tab_count(), 2);
        assert!(tab_bar.is_visible()); // Now visible

        // Set active
        tab_bar.set_active(1);
        assert!(tab_bar.tabs[0].is_active);
        assert!(!tab_bar.tabs[1].is_active);

        // Remove tab
        tab_bar.remove_tab(1);
        assert_eq!(tab_bar.tab_count(), 1);
        assert!(!tab_bar.is_visible());
    }

    #[test]
    fn test_point_in_rect() {
        let rect = D2D_RECT_F {
            left: 10.0,
            top: 10.0,
            right: 20.0,
            bottom: 20.0,
        };

        assert!(point_in_rect(15.0, 15.0, &rect));
        assert!(!point_in_rect(5.0, 15.0, &rect));
        assert!(!point_in_rect(25.0, 15.0, &rect));
    }

    #[test]
    fn test_blend_colors() {
        let white = Rgb::new(255, 255, 255);
        let black = Rgb::new(0, 0, 0);

        let blended = blend_colors(white, black, 0.5);
        assert_eq!(blended.r, 127);
        assert_eq!(blended.g, 127);
        assert_eq!(blended.b, 127);
    }
}
