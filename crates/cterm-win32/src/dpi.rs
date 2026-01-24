//! High DPI support for Windows
//!
//! Handles DPI awareness and scaling calculations.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::{
    GetDpiForSystem, GetDpiForWindow, SetProcessDpiAwarenessContext,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};

/// Default DPI value (96 = 100% scaling)
pub const DEFAULT_DPI: u32 = 96;

/// Set up DPI awareness for the application
///
/// This should be called early in the application startup, before creating any windows.
pub fn setup_dpi_awareness() {
    unsafe {
        // Try to set per-monitor DPI awareness v2 (Windows 10 1703+)
        // This provides the best high DPI experience
        let result = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if result.is_err() {
            log::warn!("Failed to set DPI awareness context: {:?}", result);
        }
    }
}

/// Get the DPI for a specific window
pub fn get_window_dpi(hwnd: HWND) -> u32 {
    unsafe { GetDpiForWindow(hwnd) }
}

/// Get the system DPI (for the primary monitor)
pub fn get_system_dpi() -> u32 {
    unsafe { GetDpiForSystem() }
}

/// Calculate the scaling factor from DPI
pub fn scale_factor(dpi: u32) -> f32 {
    dpi as f32 / DEFAULT_DPI as f32
}

/// Scale a value by DPI
pub fn scale_by_dpi(value: i32, dpi: u32) -> i32 {
    ((value as f32) * scale_factor(dpi)).round() as i32
}

/// Scale a float value by DPI
pub fn scale_f32_by_dpi(value: f32, dpi: u32) -> f32 {
    value * scale_factor(dpi)
}

/// Unscale a value (from physical to logical)
pub fn unscale_by_dpi(value: i32, dpi: u32) -> i32 {
    ((value as f32) / scale_factor(dpi)).round() as i32
}

/// DPI information for a window
#[derive(Debug, Clone, Copy)]
pub struct DpiInfo {
    /// The DPI value
    pub dpi: u32,
    /// Scaling factor (1.0 = 100%, 1.5 = 150%, etc.)
    pub scale: f32,
}

impl DpiInfo {
    /// Create DPI info from a DPI value
    pub fn from_dpi(dpi: u32) -> Self {
        Self {
            dpi,
            scale: scale_factor(dpi),
        }
    }

    /// Get DPI info for a window
    pub fn for_window(hwnd: HWND) -> Self {
        Self::from_dpi(get_window_dpi(hwnd))
    }

    /// Get system DPI info
    pub fn system() -> Self {
        Self::from_dpi(get_system_dpi())
    }

    /// Scale an integer value
    pub fn scale(&self, value: i32) -> i32 {
        ((value as f32) * self.scale).round() as i32
    }

    /// Scale a float value
    pub fn scale_f32(&self, value: f32) -> f32 {
        value * self.scale
    }

    /// Unscale an integer value (physical to logical)
    pub fn unscale(&self, value: i32) -> i32 {
        ((value as f32) / self.scale).round() as i32
    }

    /// Unscale a float value (physical to logical)
    pub fn unscale_f32(&self, value: f32) -> f32 {
        value / self.scale
    }
}

impl Default for DpiInfo {
    fn default() -> Self {
        Self {
            dpi: DEFAULT_DPI,
            scale: 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scale_factor() {
        assert_eq!(scale_factor(96), 1.0);
        assert_eq!(scale_factor(144), 1.5);
        assert_eq!(scale_factor(192), 2.0);
    }

    #[test]
    fn test_scale_by_dpi() {
        // 100% scaling
        assert_eq!(scale_by_dpi(100, 96), 100);
        // 150% scaling
        assert_eq!(scale_by_dpi(100, 144), 150);
        // 200% scaling
        assert_eq!(scale_by_dpi(100, 192), 200);
    }

    #[test]
    fn test_dpi_info() {
        let dpi = DpiInfo::from_dpi(144); // 150%
        assert_eq!(dpi.scale(100), 150);
        assert_eq!(dpi.unscale(150), 100);
    }
}
