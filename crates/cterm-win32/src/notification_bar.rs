//! Notification bar for file transfers
//!
//! Displays file transfer notifications with save/discard actions.

use cterm_core::color::Rgb;
use cterm_ui::{format_size, Theme};
use windows::core::Interface;
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_POINT_2F, D2D_RECT_F};
use windows::Win32::Graphics::Direct2D::{
    ID2D1HwndRenderTarget, ID2D1RenderTarget, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
};

use crate::dpi::DpiInfo;

/// Notification bar height in logical pixels
pub const NOTIFICATION_BAR_HEIGHT: i32 = 36;

/// Button definitions
const BUTTON_WIDTH: f32 = 80.0;
const BUTTON_HEIGHT: f32 = 24.0;
const BUTTON_MARGIN: f32 = 8.0;
const BUTTON_CORNER_RADIUS: f32 = 4.0;

/// Pending file notification
#[derive(Debug, Clone)]
pub struct PendingFile {
    pub id: u64,
    pub name: Option<String>,
    pub size: usize,
}

/// Button hit area
#[derive(Debug, Clone, Copy)]
pub struct ButtonRect {
    pub bounds: D2D_RECT_F,
}

/// Notification bar state
pub struct NotificationBar {
    pending_file: Option<PendingFile>,
    visible: bool,
    theme: Theme,
    dpi: DpiInfo,
    save_button_rect: D2D_RECT_F,
    save_as_button_rect: D2D_RECT_F,
    discard_button_rect: D2D_RECT_F,
}

impl NotificationBar {
    /// Create a new notification bar
    pub fn new(theme: &Theme) -> Self {
        Self {
            pending_file: None,
            visible: false,
            theme: theme.clone(),
            dpi: DpiInfo::default(),
            save_button_rect: D2D_RECT_F::default(),
            save_as_button_rect: D2D_RECT_F::default(),
            discard_button_rect: D2D_RECT_F::default(),
        }
    }

    /// Get the notification bar height in physical pixels
    pub fn height(&self) -> i32 {
        if self.visible {
            self.dpi.scale(NOTIFICATION_BAR_HEIGHT)
        } else {
            0
        }
    }

    /// Check if visible
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Update DPI
    pub fn set_dpi(&mut self, dpi: DpiInfo) {
        self.dpi = dpi;
    }

    /// Show a file transfer notification
    pub fn show_file(&mut self, id: u64, name: Option<&str>, size: usize) {
        self.pending_file = Some(PendingFile {
            id,
            name: name.map(|s| s.to_string()),
            size,
        });
        self.visible = true;
    }

    /// Hide the notification bar
    pub fn hide(&mut self) {
        self.pending_file = None;
        self.visible = false;
    }

    /// Get the current pending file ID if any
    pub fn pending_file_id(&self) -> Option<u64> {
        self.pending_file.as_ref().map(|f| f.id)
    }

    /// Hit test - returns which button was clicked if any
    pub fn hit_test(&self, x: f32, y: f32) -> Option<NotificationAction> {
        if !self.visible {
            return None;
        }

        if point_in_rect(x, y, &self.save_button_rect) {
            return Some(NotificationAction::Save);
        }
        if point_in_rect(x, y, &self.save_as_button_rect) {
            return Some(NotificationAction::SaveAs);
        }
        if point_in_rect(x, y, &self.discard_button_rect) {
            return Some(NotificationAction::Discard);
        }

        None
    }

    /// Calculate button layout
    fn calculate_layout(&mut self, width: f32) {
        let height = self.dpi.scale_f32(NOTIFICATION_BAR_HEIGHT as f32);
        let button_width = self.dpi.scale_f32(BUTTON_WIDTH);
        let button_height = self.dpi.scale_f32(BUTTON_HEIGHT);
        let margin = self.dpi.scale_f32(BUTTON_MARGIN);

        let button_y = (height - button_height) / 2.0;

        // Buttons from right to left
        let discard_x = width - margin - button_width;
        self.discard_button_rect = D2D_RECT_F {
            left: discard_x,
            top: button_y,
            right: discard_x + button_width,
            bottom: button_y + button_height,
        };

        let save_as_x = discard_x - margin - button_width;
        self.save_as_button_rect = D2D_RECT_F {
            left: save_as_x,
            top: button_y,
            right: save_as_x + button_width,
            bottom: button_y + button_height,
        };

        let save_x = save_as_x - margin - button_width;
        self.save_button_rect = D2D_RECT_F {
            left: save_x,
            top: button_y,
            right: save_x + button_width,
            bottom: button_y + button_height,
        };
    }

    /// Render the notification bar
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

        let height = self.dpi.scale_f32(NOTIFICATION_BAR_HEIGHT as f32);

        // Cast to parent interface to access render methods
        let base: ID2D1RenderTarget = rt.cast()?;

        // Draw background
        let bg_rect = D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: width,
            bottom: height,
        };

        // Use a slightly highlighted background
        let bg_color = Rgb::new(
            (self.theme.ui.tab_bar_background.r as u16 + 20).min(255) as u8,
            (self.theme.ui.tab_bar_background.g as u16 + 20).min(255) as u8,
            (self.theme.ui.tab_bar_background.b as u16 + 30).min(255) as u8,
        );
        let bg_brush = unsafe { base.CreateSolidColorBrush(&rgb_to_d2d_color(bg_color), None)? };
        unsafe { base.FillRectangle(&bg_rect, &bg_brush) };

        // Draw message text
        if let Some(ref file) = self.pending_file {
            let message = format!(
                "File received: {} ({})",
                file.name.as_deref().unwrap_or("unnamed"),
                format_size(file.size)
            );

            let text_color = self.theme.ui.tab_active_text;
            let text_brush =
                unsafe { base.CreateSolidColorBrush(&rgb_to_d2d_color(text_color), None)? };

            let text_wide: Vec<u16> = message.encode_utf16().collect();
            let text_width = self.save_button_rect.left - self.dpi.scale_f32(BUTTON_MARGIN * 2.0);

            let layout: IDWriteTextLayout =
                unsafe { dwrite.CreateTextLayout(&text_wide, text_format, text_width, height)? };

            unsafe {
                layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            }

            let text_origin = D2D_POINT_2F {
                x: self.dpi.scale_f32(BUTTON_MARGIN),
                y: 0.0,
            };
            unsafe { base.DrawTextLayout(text_origin, &layout, &text_brush, Default::default()) };
        }

        // Draw buttons
        self.render_button(
            rt,
            dwrite,
            &self.save_button_rect,
            "Save",
            Rgb::new(0, 128, 0),
            text_format,
        )?;
        self.render_button(
            rt,
            dwrite,
            &self.save_as_button_rect,
            "Save As...",
            Rgb::new(0, 100, 150),
            text_format,
        )?;
        self.render_button(
            rt,
            dwrite,
            &self.discard_button_rect,
            "Discard",
            Rgb::new(150, 50, 50),
            text_format,
        )?;

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

    /// Render a button
    fn render_button(
        &self,
        rt: &ID2D1HwndRenderTarget,
        dwrite: &IDWriteFactory,
        rect: &D2D_RECT_F,
        label: &str,
        bg_color: Rgb,
        text_format: &IDWriteTextFormat,
    ) -> windows::core::Result<()> {
        // Cast to parent interface to access render methods
        let base: ID2D1RenderTarget = rt.cast()?;

        let rounded_rect = D2D1_ROUNDED_RECT {
            rect: *rect,
            radiusX: self.dpi.scale_f32(BUTTON_CORNER_RADIUS),
            radiusY: self.dpi.scale_f32(BUTTON_CORNER_RADIUS),
        };

        // Button background
        let bg_brush = unsafe { base.CreateSolidColorBrush(&rgb_to_d2d_color(bg_color), None)? };
        unsafe { base.FillRoundedRectangle(&rounded_rect, &bg_brush) };

        // Button text
        let text_color = Rgb::new(255, 255, 255);
        let text_brush =
            unsafe { base.CreateSolidColorBrush(&rgb_to_d2d_color(text_color), None)? };

        let text_wide: Vec<u16> = label.encode_utf16().collect();
        let layout: IDWriteTextLayout = unsafe {
            dwrite.CreateTextLayout(
                &text_wide,
                text_format,
                rect.right - rect.left,
                rect.bottom - rect.top,
            )?
        };

        unsafe {
            layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            layout.SetTextAlignment(
                windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_CENTER,
            )?;
        }

        let text_origin = D2D_POINT_2F {
            x: rect.left,
            y: rect.top,
        };
        unsafe { base.DrawTextLayout(text_origin, &layout, &text_brush, Default::default()) };

        Ok(())
    }

    /// Update theme
    pub fn set_theme(&mut self, theme: &Theme) {
        self.theme = theme.clone();
    }
}

/// Notification bar action
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationAction {
    Save,
    SaveAs,
    Discard,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_bar_visibility() {
        let theme = Theme::dark();
        let mut bar = NotificationBar::new(&theme);

        assert!(!bar.is_visible());
        assert_eq!(bar.height(), 0);

        bar.show_file(1, Some("test.txt"), 1024);
        assert!(bar.is_visible());
        assert!(bar.height() > 0);

        bar.hide();
        assert!(!bar.is_visible());
        assert_eq!(bar.height(), 0);
    }

    #[test]
    fn test_notification_bar_pending_file() {
        let theme = Theme::dark();
        let mut bar = NotificationBar::new(&theme);

        assert!(bar.pending_file_id().is_none());

        bar.show_file(42, Some("data.bin"), 2048);
        assert_eq!(bar.pending_file_id(), Some(42));

        bar.hide();
        assert!(bar.pending_file_id().is_none());
    }
}
