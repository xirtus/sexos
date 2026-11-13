// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Arabic letters join their neighbours. Drawn a character a cell, each letter takes
//! the presentation form its neighbours call for: medial between two joining
//! letters, initial or final at a word's edges, isolated alone. Harakat and other
//! marks are transparent: letters join across them.
use super::arabic_forms::FORMS;

const TATWEEL: u32 = 0x0640;
const ZWJ: u32 = 0x200D;

fn forms(c: u32) -> Option<(u16, u16, u16, u16)> {
    if !(0x0600..=0x06FF).contains(&c) { return None; }
    FORMS.binary_search_by_key(&(c as u16), |f| f.0).ok().map(|i| { let f = FORMS[i]; (f.1, f.2, f.3, f.4) })
}

/// Marks that do not break joining (the Arabic harakat, superscript alef, Quranic marks).
pub fn transparent(c: u32) -> bool {
    matches!(c, 0x0610..=0x061A | 0x064B..=0x065F | 0x0670 | 0x06D6..=0x06DC | 0x06DF..=0x06E4 | 0x06E7 | 0x06E8 | 0x06EA..=0x06ED)
}
/// Joins to the letter after it (dual-joining letters, tatweel, ZWJ).
fn joins_forward(c: u32) -> bool {
    c == TATWEEL || c == ZWJ || forms(c).is_some_and(|f| f.2 != 0)
}
/// Joins to the letter before it (dual- and right-joining letters, tatweel, ZWJ).
fn joins_back(c: u32) -> bool {
    c == TATWEEL || c == ZWJ || forms(c).is_some_and(|f| f.1 != 0)
}

/// The form of `c` between `before` and `after` (the nearest non-transparent
/// characters, 0 at the line's edges).
pub fn form(before: u32, c: u32, after: u32) -> u32 {
    let Some((iso, fin, ini, med)) = forms(c) else { return c };
    let back = joins_forward(before) && fin != 0;
    let fwd = joins_back(after) && ini != 0;
    let pick = match (back, fwd) {
        (true, true) if med != 0 => med,
        (true, _) => fin,
        (false, true) => ini,
        _ => iso,
    };
    if pick != 0 { pick as u32 } else { c }
}

fn decode(text: &[u8], at: usize) -> Option<(u32, usize)> {
    let n = match *text.get(at)? { 0..=0x7F => 1, 0xC2..=0xDF => 2, 0xE0..=0xEF => 3, 0xF0..=0xF4 => 4, _ => return None };
    let s = core::str::from_utf8(text.get(at..at + n)?).ok()?;
    Some((s.chars().next()? as u32, at + n))
}

/// The form of the character starting at byte `at` of UTF-8 `text`, looking past
/// transparent marks to its neighbours on the same line.
pub fn form_at(text: &[u8], at: usize) -> Option<u32> {
    let (c, end) = decode(text, at)?;
    if forms(c).is_none() { return Some(c); }
    let mut after = 0;
    let mut i = end;
    while let Some((n, e)) = decode(text, i) {
        if n == 10 { break; }
        if !transparent(n) { after = n; break; }
        i = e;
    }
    let mut before = 0;
    let mut j = at;
    while j > 0 {
        let mut k = j - 1;
        while k > 0 && text[k] & 0xC0 == 0x80 { k -= 1; }
        let Some((p, _)) = decode(text, k) else { break };
        if p == 10 { break; }
        if !transparent(p) { before = p; break; }
        j = k;
    }
    Some(form(before, c, after))
}
