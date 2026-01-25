//! Screen and cell conversion between cterm-core and proto

use crate::convert::color::color_to_proto;
use crate::proto;
use cterm_core::{Cell, CellAttrs, Screen};

/// Convert cell attributes to proto
pub fn attrs_to_proto(attrs: CellAttrs) -> proto::CellAttributes {
    proto::CellAttributes {
        bold: attrs.contains(CellAttrs::BOLD),
        italic: attrs.contains(CellAttrs::ITALIC),
        underline: attrs.contains(CellAttrs::UNDERLINE),
        double_underline: attrs.contains(CellAttrs::DOUBLE_UNDERLINE),
        curly_underline: attrs.contains(CellAttrs::CURLY_UNDERLINE),
        dotted_underline: attrs.contains(CellAttrs::DOTTED_UNDERLINE),
        dashed_underline: attrs.contains(CellAttrs::DASHED_UNDERLINE),
        blink: attrs.contains(CellAttrs::BLINK),
        inverse: attrs.contains(CellAttrs::INVERSE),
        hidden: attrs.contains(CellAttrs::HIDDEN),
        strikethrough: attrs.contains(CellAttrs::STRIKETHROUGH),
        dim: attrs.contains(CellAttrs::DIM),
        overline: attrs.contains(CellAttrs::OVERLINE),
        wide: attrs.contains(CellAttrs::WIDE),
        wide_spacer: attrs.contains(CellAttrs::WIDE_SPACER),
    }
}

/// Convert proto attributes to cell attributes
pub fn proto_to_attrs(attrs: &proto::CellAttributes) -> CellAttrs {
    let mut result = CellAttrs::empty();
    if attrs.bold {
        result |= CellAttrs::BOLD;
    }
    if attrs.italic {
        result |= CellAttrs::ITALIC;
    }
    if attrs.underline {
        result |= CellAttrs::UNDERLINE;
    }
    if attrs.double_underline {
        result |= CellAttrs::DOUBLE_UNDERLINE;
    }
    if attrs.curly_underline {
        result |= CellAttrs::CURLY_UNDERLINE;
    }
    if attrs.dotted_underline {
        result |= CellAttrs::DOTTED_UNDERLINE;
    }
    if attrs.dashed_underline {
        result |= CellAttrs::DASHED_UNDERLINE;
    }
    if attrs.blink {
        result |= CellAttrs::BLINK;
    }
    if attrs.inverse {
        result |= CellAttrs::INVERSE;
    }
    if attrs.hidden {
        result |= CellAttrs::HIDDEN;
    }
    if attrs.strikethrough {
        result |= CellAttrs::STRIKETHROUGH;
    }
    if attrs.dim {
        result |= CellAttrs::DIM;
    }
    if attrs.overline {
        result |= CellAttrs::OVERLINE;
    }
    if attrs.wide {
        result |= CellAttrs::WIDE;
    }
    if attrs.wide_spacer {
        result |= CellAttrs::WIDE_SPACER;
    }
    result
}

/// Convert a cell to proto
pub fn cell_to_proto(cell: &Cell) -> proto::Cell {
    proto::Cell {
        char: cell.c.to_string(),
        fg: Some(color_to_proto(&cell.fg)),
        bg: Some(color_to_proto(&cell.bg)),
        attrs: Some(attrs_to_proto(cell.attrs)),
        underline_color: cell.underline_color.as_ref().map(color_to_proto),
        hyperlink: cell.hyperlink.as_ref().map(|h| proto::Hyperlink {
            id: h.id.clone(),
            uri: h.uri.clone(),
        }),
    }
}

/// Convert a row of cells to proto
pub fn row_to_proto(cells: &[Cell]) -> proto::Row {
    proto::Row {
        cells: cells.iter().map(cell_to_proto).collect(),
    }
}

/// Convert screen to proto representation
pub fn screen_to_proto(screen: &Screen, include_scrollback: bool) -> proto::GetScreenResponse {
    let cursor = proto::CursorPosition {
        row: screen.cursor.row as u32,
        col: screen.cursor.col as u32,
        visible: screen.cursor.visible,
        style: proto::CursorStyle::Block as i32,
    };

    // Get visible rows
    let visible_rows: Vec<proto::Row> = (0..screen.height())
        .map(|row_idx| {
            let cells: Vec<Cell> = (0..screen.width())
                .map(|col| screen.get_cell(row_idx, col).cloned().unwrap_or_default())
                .collect();
            row_to_proto(&cells)
        })
        .collect();

    // Get scrollback if requested
    let scrollback = if include_scrollback {
        screen
            .scrollback()
            .iter()
            .map(|row| {
                let cells: Vec<Cell> = row.iter().cloned().collect();
                row_to_proto(&cells)
            })
            .collect()
    } else {
        Vec::new()
    };

    proto::GetScreenResponse {
        cols: screen.width() as u32,
        rows: screen.height() as u32,
        cursor: Some(cursor),
        visible_rows,
        scrollback,
        title: screen.title.clone(),
        modes: Some(proto::TerminalModes {
            application_cursor: screen.modes.application_cursor,
            application_keypad: screen.modes.application_keypad,
            bracketed_paste: screen.modes.bracketed_paste,
            focus_events: screen.modes.focus_events,
        }),
    }
}

/// Get screen text as lines
pub fn screen_to_text(
    screen: &Screen,
    include_scrollback: bool,
    start_row: Option<u32>,
    end_row: Option<u32>,
) -> Vec<String> {
    let mut lines = Vec::new();

    // Add scrollback if requested
    if include_scrollback {
        for row in screen.scrollback().iter() {
            let text: String = row.iter().map(|c| c.c).collect();
            lines.push(text.trim_end().to_string());
        }
    }

    // Add visible rows
    let start = start_row.unwrap_or(0) as usize;
    let end = end_row.map(|e| e as usize + 1).unwrap_or(screen.height());
    let end = end.min(screen.height());

    for row_idx in start..end {
        let text: String = (0..screen.width())
            .map(|col| screen.get_cell(row_idx, col).map(|c| c.c).unwrap_or(' '))
            .collect();
        lines.push(text.trim_end().to_string());
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attrs_roundtrip() {
        let attrs = CellAttrs::BOLD | CellAttrs::ITALIC | CellAttrs::UNDERLINE;
        let proto = attrs_to_proto(attrs);
        let back = proto_to_attrs(&proto);
        assert_eq!(attrs, back);
    }

    #[test]
    fn test_cell_to_proto() {
        let cell = Cell::new('A');
        let proto = cell_to_proto(&cell);
        assert_eq!(proto.char, "A");
    }
}
