// SPDX-License-Identifier: GPL-3.0-only
//! Soft wrap: a line split into screen rows of at most `cols` columns, breaking after
//! spaces where it can and mid-word only when a word is wider than the row. The text
//! itself never changes; rows are byte ranges into the line.

use alloc::vec::Vec;

/// Display columns of one character (tabs to the next multiple of four).
fn width(c: u8, col: usize) -> usize { if c == b'\t' { 4 - col % 4 } else { 1 } }

/// Rows of `line` (one line, no `\n`) at `cols` columns: (start, end) byte offsets.
/// An empty line is one empty row.
pub fn rows(line: &[u8], cols: usize) -> Vec<(usize, usize)> {
    let cols = cols.max(1);
    let mut out = Vec::new();
    let mut start = 0;
    while start < line.len() {
        let mut col = 0;
        let mut at = start;
        let mut last_break = None;
        while at < line.len() {
            let w = width(line[at], col);
            if col + w > cols { break; }
            col += w;
            let next = crate::next_char(line, at);
            if line[at] == b' ' || line[at] == b'\t' { last_break = Some(next); }
            at = next;
        }
        if at >= line.len() { out.push((start, line.len())); return out; }
        let end = match last_break { Some(b) if b > start => b, _ => at.max(crate::next_char(line, start)) };
        out.push((start, end));
        start = end;
    }
    if out.is_empty() { out.push((0, 0)); }
    out
}

/// Which row of `rows` holds byte `at` (the end of the line belongs to the last row;
/// a break point belongs to the row it starts).
pub fn row_of(rows: &[(usize, usize)], at: usize) -> usize {
    rows.iter().position(|r| at < r.1).unwrap_or(rows.len().saturating_sub(1))
}
