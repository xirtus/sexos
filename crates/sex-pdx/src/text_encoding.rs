// SPDX-License-Identifier: GPL-3.0-only
//! Allocation-free UTF-8 data helpers. Offsets and limits always count bytes;
//! presentation uses one cell per scalar, without normalization or shaping.
//! These functions convey no authority and retain no document state.

pub const REPLACEMENT: u32 = 0xfffd;

/// Editable text admits UTF-8 scalars, LF and TAB, but no other C0/C1 controls.
/// An unsupported glyph is still valid text and its original bytes survive.
pub const fn is_text_scalar(scalar: u32) -> bool {
    scalar == 9 || scalar == 10 || (scalar >= 32 && scalar != 127
        && !(scalar >= 128 && scalar <= 159) && scalar <= 0x10ffff
        && !(scalar >= 0xd800 && scalar <= 0xdfff))
}
pub fn valid_text(bytes: &[u8]) -> bool {
    core::str::from_utf8(bytes).is_ok_and(|s| s.chars().all(|c| is_text_scalar(c as u32)))
}

/// Decode one scalar and its exclusive byte end. A malformed byte consumes one
/// byte as a visible replacement cell; stored bytes are never repaired here.
pub fn cell(bytes: &[u8], at: usize) -> Option<(u32, usize)> {
    let first = *bytes.get(at)?;
    let count = match first { 0..=127 => 1, 0xc2..=0xdf => 2,
        0xe0..=0xef => 3, 0xf0..=0xf4 => 4, _ => 0 };
    if count != 0 {
        if let Some(part) = bytes.get(at..at.saturating_add(count)) {
            if let Ok(text) = core::str::from_utf8(part) {
                return Some((text.chars().next().unwrap() as u32, at + count));
            }
        }
    }
    Some((REPLACEMENT, at + 1))
}
pub fn next(bytes: &[u8], at: usize) -> usize {
    cell(bytes, at).map_or(bytes.len(), |(_, end)| end)
}
/// Positions supplied by editable text must be scalar boundaries. The bounded
/// backward scan also makes malformed read-only byte streams navigable.
pub fn previous(bytes: &[u8], at: usize) -> usize {
    let at = at.min(bytes.len());
    if at == 0 { return 0; }
    for start in (at.saturating_sub(4)..at).rev() {
        if let Some((scalar, end)) = cell(bytes, start) {
            if end == at && (scalar != REPLACEMENT || bytes[start] == 0xef) { return start; }
        }
    }
    at - 1
}
pub fn is_boundary(bytes: &[u8], at: usize) -> bool {
    at <= bytes.len() && (at == bytes.len() || bytes[at] & 0xc0 != 0x80)
}
/// Largest complete scalar prefix within a byte budget. Invalid bytes remain
/// single replacement cells; callers admit editable text with valid_text.
pub fn prefix(bytes: &[u8], budget: usize) -> usize {
    let mut at = 0;
    while let Some((_, end)) = cell(bytes, at) {
        if end > budget { break; }
        at = end;
    }
    at
}
pub fn cells(bytes: &[u8]) -> usize {
    let mut at = 0; let mut count = 0;
    while let Some((_, end)) = cell(bytes, at) { at = end; count += 1; }
    count
}
pub fn offset(bytes: &[u8], columns: usize) -> usize {
    let mut at = 0;
    for _ in 0..columns {
        let Some((_, end)) = cell(bytes, at) else { break };
        at = end;
    }
    at
}
pub fn encode(scalar: u32, out: &mut [u8; 4]) -> Option<usize> {
    let c = char::from_u32(scalar)?;
    Some(c.encode_utf8(out).len())
}
