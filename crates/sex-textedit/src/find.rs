// SPDX-License-Identifier: GPL-3.0-only
//! Find and replace over a [`Doc`](crate::Doc): plain text, optionally ignoring ASCII
//! case, wrapping around the end. Replace-all is one undo step.

use crate::Doc;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options { pub ignore_case: bool, pub whole_word: bool }

fn eq(a: u8, b: u8, o: Options) -> bool { if o.ignore_case { a.eq_ignore_ascii_case(&b) } else { a == b } }
fn word(b: u8) -> bool { b.is_ascii_alphanumeric() || b == b'_' }

/// Every match of `needle` in `hay`, left to right, not overlapping.
pub fn all(hay: &[u8], needle: &[u8], o: Options) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() || needle.len() > hay.len() { return out; }
    let mut i = 0;
    while i + needle.len() <= hay.len() {
        if hay[i..i + needle.len()].iter().zip(needle).all(|(a, b)| eq(*a, *b, o))
            && (!o.whole_word || ((i == 0 || !word(hay[i - 1])) && hay.get(i + needle.len()).is_none_or(|b| !word(*b))))
        {
            out.push((i, i + needle.len()));
            i += needle.len();
        } else { i += 1; }
    }
    out
}

/// The next match at or after `from` (wrapping), or before it when `back`.
pub fn next(hay: &[u8], needle: &[u8], from: usize, back: bool, o: Options) -> Option<(usize, usize)> {
    let hits = all(hay, needle, o);
    if back { hits.iter().rev().find(|h| h.0 < from).or(hits.last()).copied() }
    else { hits.iter().find(|h| h.0 >= from).or(hits.first()).copied() }
}

/// Selects the next match after the cursor. False if there is none.
pub fn find_next(doc: &mut Doc, needle: &[u8], back: bool, o: Options) -> bool {
    let from = if back { doc.selection().map_or(doc.cursor, |s| s.0) } else { doc.selection().map_or(doc.cursor, |s| s.0 + 1) };
    match next(doc.text(), needle, from, back, o) {
        Some((a, b)) => { doc.select(a, b); true }
        None => false,
    }
}

/// Replaces the selected match (if the selection is one) and selects the next.
pub fn replace_next(doc: &mut Doc, needle: &[u8], with: &[u8], o: Options) -> bool {
    if let Some((a, b)) = doc.selection() {
        let sel = &doc.text()[a..b];
        if sel.len() == needle.len() && sel.iter().zip(needle).all(|(x, y)| eq(*x, *y, o)) {
            doc.replace(a, b, with);
            doc.seal();
        }
    }
    find_next(doc, needle, false, o)
}

/// Replaces every match; one undo step. Returns how many.
pub fn replace_all(doc: &mut Doc, needle: &[u8], with: &[u8], o: Options) -> usize {
    let hits = all(doc.text(), needle, o);
    if hits.is_empty() { return 0; }
    let (first, last) = (hits[0].0, hits[hits.len() - 1].1);
    let mut out = Vec::with_capacity(last - first);
    let mut at = first;
    for (a, b) in &hits { out.extend_from_slice(&doc.text()[at..*a]); out.extend_from_slice(with); at = *b; }
    doc.seal();
    doc.replace(first, last, &out);
    doc.seal();
    hits.len()
}
