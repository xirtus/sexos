// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Text drawn one character a cell (Display's text surfaces): each wrapped line's
//! cells put in visual order, and the glyph each cell shows.
use super::{arabic, bidi};

/// `cells[i]` is the byte offset of cell `i` in `text`; `lines[r]` the first cell of
/// row `r`; `count` the number of cells. Rows holding right-to-left text get their
/// cells reordered in place to the order they are shown in, left to right; a row's
/// closing newline stays last. Rows without such text are left alone, so plain text
/// costs one scan.
pub fn reorder_lines(text: &[u8], cells: &mut [u16], lines: &[u16], count: usize) {
    let count = count.min(cells.len());
    for (r, &first) in lines.iter().enumerate() {
        let start = first as usize;
        let mut end = lines.get(r + 1).map_or(count, |&n| n as usize).min(count);
        if start >= end { continue; }
        let byte = |i: usize| cells[i] as usize;
        let last_end = if end < count { byte(end) } else { text.len() };
        let mut line_end = last_end.min(text.len());
        // Leave a closing newline where it is.
        if line_end > byte(start) && text.get(line_end - 1) == Some(&b'\n') && byte(end - 1) == line_end - 1 { end -= 1; line_end -= 1; }
        if end <= start { continue; }
        let Some(line) = text.get(byte(start)..line_end).and_then(|b| core::str::from_utf8(b).ok()) else { continue };
        let mut order = [0u16; bidi::MAX];
        let Some(n) = bidi::visual_order_into(line, &mut order) else { continue };
        // A wide character has two cells with the same offset: move them together.
        let mut first_cell = [0u16; bidi::MAX];
        let mut chars = 0;
        for k in start..end {
            if k > start && cells[k] == cells[k - 1] { continue; }
            if chars == bidi::MAX { chars = usize::MAX; break; }
            first_cell[chars] = k as u16;
            chars += 1;
        }
        if chars != n { continue; }
        let mut logical = [0u16; bidi::MAX * 2];
        if end - start > logical.len() { continue; }
        logical[..end - start].copy_from_slice(&cells[start..end]);
        let mut at = start;
        for &l in &order[..n] {
            let from = first_cell[l as usize] as usize - start;
            let width = if l as usize + 1 < n { first_cell[l as usize + 1] as usize - start - from } else { end - start - from };
            cells[at..at + width].copy_from_slice(&logical[from..from + width]);
            at += width;
        }
    }
}

/// Whether a character takes two cells of a text grid: CJK ideographs, kana,
/// Hangul, fullwidth forms and emoji (East Asian Width W or F).
pub fn is_wide(scalar: u32) -> bool {
    if scalar < 0x1100 { return false; }
    let table = &super::wide_table::WIDE;
    let i = table.partition_point(|&(_, last)| last < scalar);
    i < table.len() && table[i].0 <= scalar
}

/// The scalar to draw for the character at byte `at` of `text`: an Arabic letter's
/// joining form, otherwise `scalar` itself.
pub fn shown(text: &[u8], at: usize, scalar: u32) -> u32 {
    if (0x0600..=0x06FF).contains(&scalar) { arabic::form_at(text, at).unwrap_or(scalar) } else { scalar }
}
