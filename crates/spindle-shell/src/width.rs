// SPDX-License-Identifier: GPL-3.0-only
//! Terminal cell widths: CJK and emoji take two cells, combining marks none.

use alloc::string::String;

/// Cells a character takes on a terminal grid.
pub fn char_width(c: char) -> usize {
    let u = c as u32;
    if u < 0x20 || (0x7f..0xa0).contains(&u) { return 0; }
    if u < 0x300 { return 1; }
    if is_zero_width(u) { return 0; }
    if sex_shape::cells::is_wide(u) { 2 } else { 1 }
}

fn is_zero_width(u: u32) -> bool {
    matches!(u,
        0x0300..=0x036F | 0x0483..=0x0489 | 0x0591..=0x05BD | 0x0610..=0x061A | 0x064B..=0x065F
        | 0x0E31 | 0x0E34..=0x0E3A | 0x0E47..=0x0E4E
        | 0x1AB0..=0x1AFF | 0x1DC0..=0x1DFF | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064
        | 0x20D0..=0x20FF | 0xFE00..=0xFE0F | 0xFE20..=0xFE2F | 0xFEFF | 0xE0100..=0xE01EF)
}

/// Cells a string takes.
pub fn str_width(s: &str) -> usize { s.chars().map(char_width).sum() }

/// `s` cut to at most `max` cells, ending in `…` when it was cut.
pub fn truncate(s: &str, max: usize) -> String {
    if str_width(s) <= max { return String::from(s); }
    let mut out = String::new();
    if max == 0 { return out; }
    let mut used = 0;
    for c in s.chars() {
        let w = char_width(c);
        if used + w > max - 1 { break; }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}
