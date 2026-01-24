//! Mouse handling and selection
//!
//! Handles mouse events for terminal selection and interaction.

use cterm_core::screen::{Selection, SelectionMode, SelectionPoint};
use cterm_ui::events::{Modifiers, MouseButton, ScrollDirection};

use crate::terminal_canvas::CellDimensions;

/// Mouse state tracking
#[derive(Debug, Clone)]
pub struct MouseState {
    /// Current selection mode
    pub mode: SelectionMode,
    /// Whether a selection is in progress
    pub selecting: bool,
    /// Start point of selection
    pub start: Option<SelectionPoint>,
    /// Current/end point of selection
    pub current: Option<SelectionPoint>,
    /// Last click time for double/triple click detection
    pub last_click_time: std::time::Instant,
    /// Last click position
    pub last_click_pos: (i32, i32),
    /// Click count (1=single, 2=double, 3=triple)
    pub click_count: u8,
}

impl Default for MouseState {
    fn default() -> Self {
        Self {
            mode: SelectionMode::Normal,
            selecting: false,
            start: None,
            current: None,
            last_click_time: std::time::Instant::now(),
            last_click_pos: (0, 0),
            click_count: 0,
        }
    }
}

impl MouseState {
    /// Create new mouse state
    pub fn new() -> Self {
        Self::default()
    }

    /// Handle mouse button press
    pub fn on_button_down(
        &mut self,
        button: MouseButton,
        x: i32,
        y: i32,
        modifiers: Modifiers,
        cell_dims: &CellDimensions,
        scroll_offset: usize,
    ) -> Option<Selection> {
        if button != MouseButton::Left {
            return None;
        }

        // Convert pixel position to cell position
        let (col, row) = pixel_to_cell(x, y, cell_dims, scroll_offset);

        // Check for double/triple click
        let now = std::time::Instant::now();
        let elapsed = now.duration_since(self.last_click_time);
        let same_position =
            (x - self.last_click_pos.0).abs() < 5 && (y - self.last_click_pos.1).abs() < 5;

        if elapsed.as_millis() < 500 && same_position {
            self.click_count = (self.click_count % 3) + 1;
        } else {
            self.click_count = 1;
        }

        self.last_click_time = now;
        self.last_click_pos = (x, y);

        // Determine selection mode based on click count
        self.mode = match self.click_count {
            1 => SelectionMode::Normal,
            2 => SelectionMode::Word,
            3 => SelectionMode::Line,
            _ => SelectionMode::Normal,
        };

        // Start selection
        self.selecting = true;
        let point = SelectionPoint { row, col };
        self.start = Some(point);
        self.current = Some(point);

        // Return initial selection
        Some(Selection {
            start: point,
            end: point,
            mode: self.mode,
        })
    }

    /// Handle mouse movement during selection
    pub fn on_mouse_move(
        &mut self,
        x: i32,
        y: i32,
        cell_dims: &CellDimensions,
        scroll_offset: usize,
    ) -> Option<Selection> {
        if !self.selecting {
            return None;
        }

        let (col, row) = pixel_to_cell(x, y, cell_dims, scroll_offset);
        let point = SelectionPoint { row, col };
        self.current = Some(point);

        self.start.map(|start| Selection {
            start,
            end: point,
            mode: self.mode,
        })
    }

    /// Handle mouse button release
    pub fn on_button_up(
        &mut self,
        button: MouseButton,
        x: i32,
        y: i32,
        cell_dims: &CellDimensions,
        scroll_offset: usize,
    ) -> Option<Selection> {
        if button != MouseButton::Left || !self.selecting {
            return None;
        }

        let (col, row) = pixel_to_cell(x, y, cell_dims, scroll_offset);
        let point = SelectionPoint { row, col };
        self.current = Some(point);
        self.selecting = false;

        self.start.map(|start| Selection {
            start,
            end: point,
            mode: self.mode,
        })
    }

    /// Clear selection state
    pub fn clear_selection(&mut self) {
        self.selecting = false;
        self.start = None;
        self.current = None;
        self.click_count = 0;
    }

    /// Check if there's an active selection
    pub fn has_selection(&self) -> bool {
        self.start.is_some() && self.current.is_some()
    }

    /// Get current selection if any
    pub fn get_selection(&self) -> Option<Selection> {
        match (self.start, self.current) {
            (Some(start), Some(end)) => Some(Selection {
                start,
                end,
                mode: self.mode,
            }),
            _ => None,
        }
    }
}

/// Convert pixel position to cell position
pub fn pixel_to_cell(
    x: i32,
    y: i32,
    cell_dims: &CellDimensions,
    scroll_offset: usize,
) -> (usize, usize) {
    let col = (x.max(0) as f32 / cell_dims.width).floor() as usize;
    let row = (y.max(0) as f32 / cell_dims.height).floor() as usize + scroll_offset;
    (col, row)
}

/// Convert cell position to pixel position (top-left corner)
pub fn cell_to_pixel(row: usize, col: usize, cell_dims: &CellDimensions) -> (f32, f32) {
    let x = col as f32 * cell_dims.width;
    let y = row as f32 * cell_dims.height;
    (x, y)
}

/// Handle scroll wheel events
pub fn handle_scroll(
    direction: ScrollDirection,
    delta: f32,
    modifiers: Modifiers,
    current_scroll: usize,
    max_scroll: usize,
    rows: usize,
) -> usize {
    let lines = if modifiers.contains(Modifiers::SHIFT) {
        // Shift+scroll = one line at a time
        1
    } else if modifiers.contains(Modifiers::CTRL) {
        // Ctrl+scroll = page at a time
        rows.saturating_sub(1)
    } else {
        // Normal scroll = 3 lines
        3
    };

    let scroll_amount = (delta.abs() as usize).max(1) * lines;

    match direction {
        ScrollDirection::Up => {
            // Scroll up = go back in history (increase scroll offset)
            (current_scroll + scroll_amount).min(max_scroll)
        }
        ScrollDirection::Down => {
            // Scroll down = go towards present (decrease scroll offset)
            current_scroll.saturating_sub(scroll_amount)
        }
        _ => current_scroll,
    }
}

/// Convert Windows mouse button to our MouseButton
pub fn win32_button_to_button(button: u32) -> Option<MouseButton> {
    use winapi::um::winuser::*;

    match button {
        x if x == WM_LBUTTONDOWN || x == WM_LBUTTONUP => Some(MouseButton::Left),
        x if x == WM_MBUTTONDOWN || x == WM_MBUTTONUP => Some(MouseButton::Middle),
        x if x == WM_RBUTTONDOWN || x == WM_RBUTTONUP => Some(MouseButton::Right),
        x if x == WM_XBUTTONDOWN || x == WM_XBUTTONUP => {
            // X buttons need GET_XBUTTON_WPARAM, handled separately
            None
        }
        _ => None,
    }
}

/// Get scroll direction and delta from wheel message
pub fn get_scroll_delta(wparam: usize) -> (ScrollDirection, f32) {
    // High word of wParam contains wheel delta
    let delta = ((wparam >> 16) & 0xFFFF) as i16;
    let direction = if delta > 0 {
        ScrollDirection::Up
    } else {
        ScrollDirection::Down
    };
    (direction, (delta.abs() as f32) / 120.0) // 120 = WHEEL_DELTA
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pixel_to_cell() {
        let dims = CellDimensions {
            width: 10.0,
            height: 20.0,
            baseline: 15.0,
        };

        assert_eq!(pixel_to_cell(0, 0, &dims, 0), (0, 0));
        assert_eq!(pixel_to_cell(15, 25, &dims, 0), (1, 1));
        assert_eq!(pixel_to_cell(15, 25, &dims, 5), (1, 6));
    }

    #[test]
    fn test_cell_to_pixel() {
        let dims = CellDimensions {
            width: 10.0,
            height: 20.0,
            baseline: 15.0,
        };

        assert_eq!(cell_to_pixel(0, 0, &dims), (0.0, 0.0));
        assert_eq!(cell_to_pixel(1, 2, &dims), (20.0, 20.0));
    }

    #[test]
    fn test_handle_scroll() {
        let rows = 24;
        let max_scroll = 100;

        // Scroll up from 0
        assert_eq!(
            handle_scroll(
                ScrollDirection::Up,
                1.0,
                Modifiers::empty(),
                0,
                max_scroll,
                rows
            ),
            3
        );

        // Scroll down from 10
        assert_eq!(
            handle_scroll(
                ScrollDirection::Down,
                1.0,
                Modifiers::empty(),
                10,
                max_scroll,
                rows
            ),
            7
        );

        // Scroll up with Shift
        assert_eq!(
            handle_scroll(
                ScrollDirection::Up,
                1.0,
                Modifiers::SHIFT,
                0,
                max_scroll,
                rows
            ),
            1
        );

        // Scroll up with Ctrl (page)
        assert_eq!(
            handle_scroll(
                ScrollDirection::Up,
                1.0,
                Modifiers::CTRL,
                0,
                max_scroll,
                rows
            ),
            23
        );

        // Don't scroll past max
        assert_eq!(
            handle_scroll(
                ScrollDirection::Up,
                1.0,
                Modifiers::empty(),
                99,
                max_scroll,
                rows
            ),
            100
        );

        // Don't scroll below 0
        assert_eq!(
            handle_scroll(
                ScrollDirection::Down,
                1.0,
                Modifiers::empty(),
                1,
                max_scroll,
                rows
            ),
            0
        );
    }

    #[test]
    fn test_mouse_state_click_detection() {
        let mut state = MouseState::new();
        let dims = CellDimensions::default();

        // First click
        state.on_button_down(MouseButton::Left, 10, 10, Modifiers::empty(), &dims, 0);
        assert_eq!(state.click_count, 1);
        assert_eq!(state.mode, SelectionMode::Normal);

        // Quick second click (double click)
        state.on_button_down(MouseButton::Left, 10, 10, Modifiers::empty(), &dims, 0);
        assert_eq!(state.click_count, 2);
        assert_eq!(state.mode, SelectionMode::Word);

        // Quick third click (triple click)
        state.on_button_down(MouseButton::Left, 10, 10, Modifiers::empty(), &dims, 0);
        assert_eq!(state.click_count, 3);
        assert_eq!(state.mode, SelectionMode::Line);
    }
}
