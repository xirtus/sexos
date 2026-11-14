// SPDX-License-Identifier: GPL-3.0-only
//! Fuzzy matching for Go to File and the command palette, scored the way fzf and VS Code
//! do it: every pattern character must appear in order; matches at word starts, after
//! path separators, in a row and in the file name score higher; shorter candidates win
//! ties.

use alloc::vec::Vec;

fn boundary(prev: Option<u8>, c: u8) -> bool {
    match prev { None => true, Some(p) => matches!(p, b'/' | b'_' | b'-' | b'.' | b' ' | b':') || (p.is_ascii_lowercase() && c.is_ascii_uppercase()) }
}

/// The score of `pattern` against `cand`, with the matched byte positions; `None` when
/// it does not match. Case-insensitive unless the pattern has a capital.
pub fn score(pattern: &str, cand: &[u8]) -> Option<(i32, Vec<usize>)> {
    let pat: Vec<u8> = pattern.bytes().filter(|b| *b != b' ').collect();
    if pat.is_empty() { return Some((0, Vec::new())); }
    let smart = pat.iter().any(|b| b.is_ascii_uppercase());
    let eq = |a: u8, b: u8| if smart { a == b } else { a.to_ascii_lowercase() == b.to_ascii_lowercase() };
    // Quick reject, and the last position the match may start from.
    let mut j = 0;
    for &c in cand { if j < pat.len() && eq(pat[j], c) { j += 1; } }
    if j < pat.len() { return None; }
    let name_at = cand.iter().rposition(|b| *b == b'/').map_or(0, |i| i + 1);
    // Dynamic programming over (pattern index, candidate index): best score ending there.
    let (n, m) = (pat.len(), cand.len());
    const NEG: i32 = i32::MIN / 4;
    let mut best = alloc::vec![NEG; n * m];
    let mut from = alloc::vec![usize::MAX; n * m];
    for i in 0..n {
        // Best score of the previous row at any earlier position, less one per byte skipped.
        let mut gap = NEG;
        let mut gap_at = usize::MAX;
        for jx in 0..m {
            if i > 0 && jx > 0 {
                if gap > NEG { gap -= 1; }
                let p = best[(i - 1) * m + jx - 1];
                if p > NEG && p - 1 > gap { gap = p - 1; gap_at = jx - 1; }
            }
            if !eq(pat[i], cand[jx]) { continue; }
            let c = cand[jx];
            let mut bonus = 16;
            if boundary(if jx == 0 { None } else { Some(cand[jx - 1]) }, c) { bonus += 24; }
            if jx >= name_at { bonus += 8; }
            if c == pat[i] { bonus += 1; }
            let s = if i == 0 { bonus - (jx as i32).min(20) } else {
                let mut s = NEG;
                let mut at = usize::MAX;
                if jx > 0 && best[(i - 1) * m + jx - 1] > NEG { s = best[(i - 1) * m + jx - 1] + bonus + 20; at = jx - 1; }
                if gap > NEG && gap + bonus > s { s = gap + bonus; at = gap_at; }
                if at == usize::MAX { continue; }
                from[i * m + jx] = at;
                s
            };
            best[i * m + jx] = s;
        }
    }
    let (end, s) = (0..m).map(|jx| (jx, best[(n - 1) * m + jx])).max_by_key(|x| x.1)?;
    if s <= NEG { return None; }
    let mut pos = alloc::vec![0; n];
    let mut jx = end;
    for i in (0..n).rev() { pos[i] = jx; if i > 0 { jx = from[i * m + jx]; if jx == usize::MAX { return None; } } }
    Some((s - (m as i32) / 8, pos))
}

/// Indices of `cands` that match, best first; at most `limit`.
pub fn rank<T: AsRef<[u8]>>(pattern: &str, cands: &[T], limit: usize) -> Vec<(usize, i32)> {
    let mut v: Vec<(usize, i32)> = cands.iter().enumerate().filter_map(|(i, c)| score(pattern, c.as_ref()).map(|(s, _)| (i, s))).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(cands[a.0].as_ref().len().cmp(&cands[b.0].as_ref().len())));
    v.truncate(limit);
    v
}
