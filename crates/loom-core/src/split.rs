// SPDX-License-Identifier: GPL-3.0-only
//! A bounded text row for Loom's two-folder list view.
use alloc::format;
use alloc::string::String;

pub fn columns(left: &str, right: &str, width: usize) -> String {
    let mut clipped: String = left.chars().take(width).collect();
    if left.chars().count() > width && width > 0 {
        clipped.pop();
        clipped.push('…');
    }
    format!("{clipped:<width$} │ {right}")
}

/// Both halves must fit a whole tile and the header/footer before pixel drawing.
pub fn grid_fits(width: usize, height: usize, cell_w: usize, cell_h: usize,
                 head: usize, foot: usize) -> bool {
    width / 2 >= cell_w.saturating_add(8)
        && width - width / 2 >= cell_w.saturating_add(8)
        && height >= head.saturating_add(foot).saturating_add(cell_h)
}

/// Tile under a pointer in a two-folder pixel grid. `None` means the pane
/// header or blank space was clicked; its pane can still receive focus.
pub fn grid_hit(width: usize, x: usize, y: usize, cell_w: usize, cell_h: usize,
                head: usize, left_top: usize, right_top: usize,
                left_len: usize, right_len: usize) -> (usize, Option<usize>) {
    let half = width / 2;
    let pane = usize::from(x >= half);
    let local_x = if pane == 0 { x } else { x - half };
    let pane_w = if pane == 0 { half } else { width - half };
    let cols = pane_w.saturating_sub(16).checked_div(cell_w).unwrap_or(0).max(1);
    let (top, len) = if pane == 0 { (left_top, left_len) } else { (right_top, right_len) };
    if y < head || local_x < 8 { return (pane, None); }
    let col = (local_x - 8) / cell_w;
    if col >= cols { return (pane, None); }
    let index = top.saturating_mul(cols).saturating_add((y - head) / cell_h * cols + col);
    (pane, (index < len).then_some(index))
}

/// Tile in a single-pane grid. Footer, header, side margins and the unused
/// remainder of a row are not selections.
pub fn single_grid_hit(width: usize, height: usize, x: usize, y: usize,
                       cell_w: usize, cell_h: usize, head: usize, foot: usize,
                       top: usize, len: usize) -> Option<usize> {
    let cols = width.saturating_sub(8).checked_div(cell_w).unwrap_or(0).max(1);
    let rows = height.saturating_sub(head + foot).checked_div(cell_h).unwrap_or(0).max(1);
    if x < 8 || y < head || y >= head + rows * cell_h
        || (y - head) % cell_h >= cell_h.saturating_sub(6) { return None; }
    let col = (x - 8) / cell_w;
    if col >= cols || x >= 8 + col * cell_w + cell_w - 8 { return None; }
    let index = top.saturating_mul(cols).saturating_add((y - head) / cell_h * cols + col);
    (index < len).then_some(index)
}

#[cfg(test)]
mod tests {
    use super::{columns, grid_fits, grid_hit, single_grid_hit};

    #[test]
    fn right_folder_stays_visible_when_left_name_is_long() {
        let line = columns("> a-very-long-folder-name", "  report.pdf", 12);
        assert_eq!(line, "> a-very-lo… │   report.pdf");
        assert_eq!(columns("* docs", "> notes", 12), "* docs       │ > notes");
    }

    #[test]
    fn pointer_hits_independent_grid_tiles_and_blank_panes() {
        assert_eq!(grid_hit(800, 15, 40, 136, 142, 34, 0, 1, 4, 5), (0, Some(0)));
        assert_eq!(grid_hit(800, 415, 40, 136, 142, 34, 0, 1, 4, 5), (1, Some(2)));
        assert_eq!(grid_hit(800, 415, 10, 136, 142, 34, 0, 1, 4, 5), (1, None));
        assert_eq!(grid_hit(800, 790, 400, 136, 142, 34, 0, 1, 4, 5), (1, None));
    }

    #[test]
    fn single_grid_hit_matches_visible_tile_geometry() {
        let hit = |x, y| single_grid_hit(800, 500, x, y, 136, 142, 34, 40, 0, 6);
        assert_eq!(hit(12, 40), Some(0));
        assert_eq!(hit(284, 40), Some(2));
        assert_eq!(hit(12, 185), Some(5));
        assert_eq!(hit(140, 40), None); // gap between tile boxes
        assert_eq!(hit(12, 172), None); // gap between tile rows
        assert_eq!(hit(284, 10), None);
        assert_eq!(hit(284, 470), None);
        assert_eq!(hit(700, 185), None); // beyond the last row entry
    }

    #[test]
    fn split_grid_requires_a_complete_cell_in_both_panes() {
        let fits = |w, h| grid_fits(w, h, 136, 142, 34, 40);
        assert!(!fits(287, 216));
        assert!(!fits(288, 215));
        assert!(fits(288, 216));
        assert!(fits(289, 216));
    }
}
