// SPDX-License-Identifier: GPL-3.0-only
//! The Pinyin dictionary (data/pinyin.txt, built by scripts/gen_pinyin_table.py),
//! searched as sorted text: no index, no allocation.
static TEXT: &str = include_str!("../data/pinyin.txt");

/// The space-separated words for `key` (syllables joined by `-`), most common first.
pub fn lookup(key: &str) -> Option<&'static str> {
    let text = TEXT.as_bytes();
    let (mut lo, mut hi) = (0usize, text.len());
    // Invariant: lo and hi are line starts (or the end).
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let start = text[..mid].iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1).max(lo);
        let end = text[start..].iter().position(|&b| b == b'\n').map_or(text.len(), |i| start + i);
        let line = &TEXT[start..end];
        let (k, words) = line.split_once(' ').unwrap_or((line, ""));
        match k.as_bytes().cmp(key.as_bytes()) {
            core::cmp::Ordering::Equal => return Some(words),
            core::cmp::Ordering::Less => lo = end + 1,
            core::cmp::Ordering::Greater => hi = start,
        }
    }
    None
}
