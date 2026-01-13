//! Grid - 2D array of terminal cells
//!
//! The grid represents the visible terminal area and provides efficient
//! access to cells by row and column.

use crate::cell::Cell;

/// A row of cells in the terminal
#[derive(Debug, Clone)]
pub struct Row {
    cells: Vec<Cell>,
    /// Whether this row has been wrapped from the previous row
    pub wrapped: bool,
}

impl Row {
    /// Create a new row with the given width
    pub fn new(width: usize) -> Self {
        Self {
            cells: vec![Cell::default(); width],
            wrapped: false,
        }
    }

    /// Get the width of this row
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Check if the row is empty (no width)
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Resize the row to a new width
    pub fn resize(&mut self, width: usize) {
        self.cells.resize(width, Cell::default());
    }

    /// Clear all cells in the row
    pub fn clear(&mut self) {
        for cell in &mut self.cells {
            cell.reset();
        }
        self.wrapped = false;
    }

    /// Get a reference to a cell at the given column
    pub fn get(&self, col: usize) -> Option<&Cell> {
        self.cells.get(col)
    }

    /// Get a mutable reference to a cell at the given column
    pub fn get_mut(&mut self, col: usize) -> Option<&mut Cell> {
        self.cells.get_mut(col)
    }

    /// Iterator over cells
    pub fn iter(&self) -> impl Iterator<Item = &Cell> {
        self.cells.iter()
    }

    /// Mutable iterator over cells
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Cell> {
        self.cells.iter_mut()
    }

    /// Get the text content of this row (trimmed)
    pub fn text(&self) -> String {
        let mut s: String = self.cells.iter().map(|c| c.c).collect();
        // Trim trailing spaces
        let trimmed_len = s.trim_end().len();
        s.truncate(trimmed_len);
        s
    }

    /// Check if this row contains only empty cells
    pub fn is_all_empty(&self) -> bool {
        self.cells.iter().all(|c| c.is_empty())
    }
}

impl std::ops::Index<usize> for Row {
    type Output = Cell;

    fn index(&self, col: usize) -> &Self::Output {
        &self.cells[col]
    }
}

impl std::ops::IndexMut<usize> for Row {
    fn index_mut(&mut self, col: usize) -> &mut Self::Output {
        &mut self.cells[col]
    }
}

/// 2D grid of terminal cells
#[derive(Debug, Clone)]
pub struct Grid {
    rows: Vec<Row>,
    width: usize,
    height: usize,
}

impl Grid {
    /// Create a new grid with the given dimensions
    pub fn new(width: usize, height: usize) -> Self {
        let rows = (0..height).map(|_| Row::new(width)).collect();
        Self {
            rows,
            width,
            height,
        }
    }

    /// Get the grid width (columns)
    pub fn width(&self) -> usize {
        self.width
    }

    /// Get the grid height (rows)
    pub fn height(&self) -> usize {
        self.height
    }

    /// Resize the grid to new dimensions
    pub fn resize(&mut self, width: usize, height: usize) {
        // Resize existing rows
        for row in &mut self.rows {
            row.resize(width);
        }

        // Add or remove rows as needed
        if height > self.height {
            for _ in self.height..height {
                self.rows.push(Row::new(width));
            }
        } else if height < self.height {
            self.rows.truncate(height);
        }

        self.width = width;
        self.height = height;
    }

    /// Get a reference to a row
    pub fn row(&self, row: usize) -> Option<&Row> {
        self.rows.get(row)
    }

    /// Get a mutable reference to a row
    pub fn row_mut(&mut self, row: usize) -> Option<&mut Row> {
        self.rows.get_mut(row)
    }

    /// Get a reference to a cell at (row, col)
    pub fn get(&self, row: usize, col: usize) -> Option<&Cell> {
        self.rows.get(row)?.get(col)
    }

    /// Get a mutable reference to a cell at (row, col)
    pub fn get_mut(&mut self, row: usize, col: usize) -> Option<&mut Cell> {
        self.rows.get_mut(row)?.get_mut(col)
    }

    /// Clear all cells in the grid
    pub fn clear(&mut self) {
        for row in &mut self.rows {
            row.clear();
        }
    }

    /// Clear a range of rows
    pub fn clear_rows(&mut self, start: usize, end: usize) {
        for row in self.rows[start..end].iter_mut() {
            row.clear();
        }
    }

    /// Scroll the grid up by `count` lines
    /// Returns the rows that were scrolled out (for scrollback)
    pub fn scroll_up(&mut self, count: usize, top: usize, bottom: usize) -> Vec<Row> {
        let count = count.min(bottom - top);
        if count == 0 {
            return Vec::new();
        }

        // Extract the rows being scrolled out
        let scrolled_out: Vec<Row> = self.rows[top..top + count].to_vec();

        // Shift rows up within the scroll region
        for i in top..bottom - count {
            self.rows.swap(i, i + count);
        }

        // Clear the bottom rows
        for i in (bottom - count)..bottom {
            self.rows[i].clear();
        }

        scrolled_out
    }

    /// Scroll the grid down by `count` lines
    pub fn scroll_down(&mut self, count: usize, top: usize, bottom: usize) {
        let count = count.min(bottom - top);
        if count == 0 {
            return;
        }

        // Shift rows down within the scroll region
        for i in (top + count..bottom).rev() {
            self.rows.swap(i, i - count);
        }

        // Clear the top rows
        for i in top..top + count {
            self.rows[i].clear();
        }
    }

    /// Iterator over rows
    pub fn iter(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter()
    }

    /// Mutable iterator over rows
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Row> {
        self.rows.iter_mut()
    }

    /// Get all text content from the grid
    pub fn text(&self) -> String {
        self.rows
            .iter()
            .map(|r| r.text())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl std::ops::Index<usize> for Grid {
    type Output = Row;

    fn index(&self, row: usize) -> &Self::Output {
        &self.rows[row]
    }
}

impl std::ops::IndexMut<usize> for Grid {
    fn index_mut(&mut self, row: usize) -> &mut Self::Output {
        &mut self.rows[row]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_new() {
        let grid = Grid::new(80, 24);
        assert_eq!(grid.width(), 80);
        assert_eq!(grid.height(), 24);
    }

    #[test]
    fn test_grid_access() {
        let mut grid = Grid::new(80, 24);

        // Set a cell
        grid[0][0].c = 'A';
        assert_eq!(grid[0][0].c, 'A');

        // Via get/get_mut
        assert_eq!(grid.get(0, 0).unwrap().c, 'A');
        grid.get_mut(0, 0).unwrap().c = 'B';
        assert_eq!(grid[0][0].c, 'B');
    }

    #[test]
    fn test_grid_resize() {
        let mut grid = Grid::new(80, 24);
        grid[0][0].c = 'A';

        grid.resize(100, 30);
        assert_eq!(grid.width(), 100);
        assert_eq!(grid.height(), 30);
        assert_eq!(grid[0][0].c, 'A'); // Content preserved

        grid.resize(40, 10);
        assert_eq!(grid.width(), 40);
        assert_eq!(grid.height(), 10);
    }

    #[test]
    fn test_grid_scroll_up() {
        let mut grid = Grid::new(80, 5);

        // Set up some content
        for i in 0..5 {
            grid[i][0].c = char::from_digit(i as u32, 10).unwrap();
        }

        // Scroll up by 2 (full screen)
        let scrolled = grid.scroll_up(2, 0, 5);
        assert_eq!(scrolled.len(), 2);
        assert_eq!(scrolled[0][0].c, '0');
        assert_eq!(scrolled[1][0].c, '1');

        // Check remaining content
        assert_eq!(grid[0][0].c, '2');
        assert_eq!(grid[1][0].c, '3');
        assert_eq!(grid[2][0].c, '4');
        assert_eq!(grid[3][0].c, ' '); // Cleared
        assert_eq!(grid[4][0].c, ' '); // Cleared
    }

    #[test]
    fn test_grid_scroll_down() {
        let mut grid = Grid::new(80, 5);

        for i in 0..5 {
            grid[i][0].c = char::from_digit(i as u32, 10).unwrap();
        }

        grid.scroll_down(2, 0, 5);

        assert_eq!(grid[0][0].c, ' '); // Cleared
        assert_eq!(grid[1][0].c, ' '); // Cleared
        assert_eq!(grid[2][0].c, '0');
        assert_eq!(grid[3][0].c, '1');
        assert_eq!(grid[4][0].c, '2');
    }

    #[test]
    fn test_row_text() {
        let mut row = Row::new(10);
        row[0].c = 'H';
        row[1].c = 'i';
        // Rest are spaces

        assert_eq!(row.text(), "Hi");
    }
}
