// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The order a line is shown in. A line with right-to-left letters is laid out by
//! the Unicode Bidirectional Algorithm for one line (UAX #9, rules P2-P3, W1-W7,
//! N1-N2, I1-I2, L1-L2), its direction taken from its first strong letter.
//! Explicit embeddings and isolates are ignored (treated as BN). No allocation: a
//! line of up to [`MAX`] characters; longer lines are shown as stored.
//! Tests check it against the `unicode-bidi` crate.
use super::bidi_classes::RUNS;

pub const MAX: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Class { L, R, AL, EN, ES, ET, AN, CS, NSM, BN, B, S, WS, ON }
use Class::*;

pub fn class(c: char) -> Class {
    const ALL: [Class; 14] = [L, R, AL, EN, ES, ET, AN, CS, NSM, BN, B, S, WS, ON];
    let i = RUNS.partition_point(|r| r.0 <= c as u32).saturating_sub(1);
    ALL[RUNS[i].1 as usize]
}

/// True if any character could need reordering (Hebrew, Arabic, Syriac, Thaana,
/// NKo, their presentation forms, other right-to-left scripts, direction marks).
pub fn needs_reorder(text: &str) -> bool {
    text.chars().any(|c| matches!(class(c), R | AL) || c == '\u{200F}')
}

/// The paragraph is right-to-left (its first strong letter is).
pub fn is_rtl(line: &str) -> bool {
    line.chars().map(class).find(|c| matches!(c, L | R | AL)).is_some_and(|c| c != L)
}

/// The embedding level of each character of `line` (odd: right to left) into
/// `out`; returns how many, or `None` if the line is longer than `out`.
pub fn levels_into(line: &str, out: &mut [u8]) -> Option<usize> {
    let mut t = [ON; MAX];
    let mut n = 0;
    for c in line.chars() { if n == MAX || n == out.len() { return None; } t[n] = class(c); n += 1; }
    let orig = t;
    let para: u8 = if is_rtl(line) { 1 } else { 0 };
    let e = if para == 1 { R } else { L };
    // BN are removed from the run (X9): later rules skip them.
    let live = |i: usize, t: &[Class; MAX]| t[i] != BN;
    // W1: NSM takes the previous type (sos at start).
    let mut prev = e;
    for i in 0..n { if !live(i, &t) { continue; } if t[i] == NSM { t[i] = prev; } prev = t[i]; }
    // W2: EN after AL is AN. W3: AL is R.
    let mut strong = e;
    for i in 0..n {
        match t[i] { L | R | AL => strong = t[i], EN if strong == AL => t[i] = AN, _ => {} }
    }
    for i in 0..n { if t[i] == AL { t[i] = R; } }
    // W4: one separator between two numbers of the same kind joins them.
    let neighbours = |i: usize, t: &[Class; MAX]| -> (Option<Class>, Option<Class>) {
        let p = (0..i).rev().find(|&j| live(j, t)).map(|j| t[j]);
        let q = (i + 1..n).find(|&j| live(j, t)).map(|j| t[j]);
        (p, q)
    };
    for i in 0..n {
        let (p, q) = neighbours(i, &t);
        match t[i] {
            ES if p == Some(EN) && q == Some(EN) => t[i] = EN,
            CS if p == Some(EN) && q == Some(EN) => t[i] = EN,
            CS if p == Some(AN) && q == Some(AN) => t[i] = AN,
            _ => {}
        }
    }
    // W5: a run of ET (and BN) next to EN is EN.
    let mut i = 0;
    while i < n {
        if t[i] == ET || (t[i] == BN && i > 0 && t[i - 1] == ET) {
            let start = i;
            while i < n && (t[i] == ET || t[i] == BN) { i += 1; }
            let before = (0..start).rev().find(|&j| live(j, &t)).map(|j| t[j]) == Some(EN);
            let after = i < n && t[i] == EN;
            if before || after { for k in start..i { if t[k] == ET { t[k] = EN; } } }
        } else { i += 1; }
    }
    // W6: other separators and terminators are neutral.
    for i in 0..n { if matches!(t[i], ES | ET | CS) { t[i] = ON; } }
    // W7: EN after L (or an L paragraph start) is L.
    let mut strong = e;
    for i in 0..n { match t[i] { L | R => strong = t[i], EN if strong == L => t[i] = L, _ => {} } }
    let dir = |c: Class| match c { L => Some(L), R | EN | AN => Some(R), _ => None };
    // N0: paired brackets take the direction of what they enclose (BD16, up to 63 deep).
    let chars = {
        let mut a = ['\0'; MAX];
        for (slot, c) in a.iter_mut().zip(line.chars()) { *slot = c; }
        a
    };
    let opening = |c: char| matches!(c, '(' | '[' | '{');
    let closing_for = |c: char| match c { '(' => ')', '[' => ']', '{' => '}', _ => '\0' };
    let mut stack = [(0usize, '\0'); 63];
    let mut depth = 0;
    let mut pairs = [(0usize, 0usize); 64];
    let mut npairs = 0;
    for i in 0..n {
        if t[i] != ON { continue; }
        let c = chars[i];
        if opening(c) {
            if depth == stack.len() { break; }
            stack[depth] = (i, closing_for(c)); depth += 1;
        } else if let Some(k) = (0..depth).rev().find(|&k| stack[k].1 == c) {
            if npairs < pairs.len() { pairs[npairs] = (stack[k].0, i); npairs += 1; }
            depth = k;
        }
    }
    pairs[..npairs].sort_unstable();
    for &(open, close) in &pairs[..npairs] {
        let inside = (open + 1..close).filter_map(|j| dir(t[j]));
        let mut found = None;
        for d in inside { if d == e { found = Some(e); break; } found = Some(d); }
        let to = match found {
            Some(d) if d == e => Some(e),
            Some(d) => {
                let before = (0..open).rev().find_map(|j| dir(t[j])).unwrap_or(e);
                Some(if before == d { d } else { e })
            }
            None => None,
        };
        if let Some(d) = to {
            t[open] = d; t[close] = d;
            for k in [open, close] {
                let mut j = k + 1;
                while j < n && orig[j] == NSM { t[j] = d; j += 1; }
            }
        }
    }
    // N1/N2: neutrals between two strong types of one direction take it, else the paragraph's.
    let neutral = |c: Class| matches!(c, B | S | WS | ON | BN);
    let mut i = 0;
    while i < n {
        if !neutral(t[i]) { i += 1; continue; }
        let start = i;
        while i < n && neutral(t[i]) { i += 1; }
        let before = if start == 0 { e } else { dir(t[start - 1]).unwrap_or(e) };
        let after = if i < n { dir(t[i]).unwrap_or(e) } else { e };
        let to = if before == after { before } else { e };
        for k in start..i { t[k] = to; }
    }
    // I1/I2.
    for i in 0..n {
        out[i] = match (para, t[i]) {
            (0, R) => 1, (0, AN | EN) => 2, (0, _) => 0,
            (_, L | EN | AN) => 2, _ => 1,
        };
    }
    // L1: segment separators, and whitespace before them or at the line's end, take
    // the paragraph level (by original type).
    let mut trailing = true;
    for i in (0..n).rev() {
        match orig[i] {
            S | B => { out[i] = para; trailing = true; }
            WS | BN if trailing => out[i] = para,
            _ => trailing = false,
        }
    }
    Some(n)
}

/// For a line (no newline), the logical index of the character shown at each
/// visual position, left to right, into `order`; returns how many. `None` when the
/// line needs no reordering or is too long.
pub fn visual_order_into(line: &str, order: &mut [u16]) -> Option<usize> {
    if !needs_reorder(line) { return None; }
    let mut levels = [0u8; MAX];
    let n = levels_into(line, &mut levels)?;
    if order.len() < n { return None; }
    for (i, o) in order[..n].iter_mut().enumerate() { *o = i as u16; }
    // L2: from the highest level to the lowest odd one, reverse every run at or above it.
    let high = levels[..n].iter().copied().max().unwrap_or(0);
    let low_odd = levels[..n].iter().copied().filter(|l| l % 2 == 1).min().unwrap_or(high + 1);
    let mut level = high;
    while level >= low_odd && level > 0 {
        let mut i = 0;
        while i < n {
            if levels[order[i] as usize] >= level {
                let start = i;
                while i < n && levels[order[i] as usize] >= level { i += 1; }
                order[start..i].reverse();
            } else { i += 1; }
        }
        level -= 1;
    }
    Some(n)
}

/// The mirrored glyph a character takes when shown right to left.
pub fn mirror(c: char) -> char {
    match c {
        '(' => ')', ')' => '(', '[' => ']', ']' => '[', '{' => '}', '}' => '{', '<' => '>', '>' => '<',
        '«' => '»', '»' => '«', '‹' => '›', '›' => '‹', '≤' => '≥', '≥' => '≤',
        _ => c,
    }
}
