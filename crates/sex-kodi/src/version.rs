// SPDX-License-Identifier: GPL-3.0-only
//! Kodi's version order. Kodi compares `[epoch:]version` the way Debian does: an epoch
//! first, then runs of digits as numbers and everything else character by character,
//! with letters before other characters and `~` before anything, even the end. So
//! `1.0.0~beta1` < `1.0.0` < `1.0.0+matrix.1` < `1.0.1` < `1:0.1`.

use core::cmp::Ordering;

fn split_epoch(v: &str) -> (u64, &str) {
    let v = v.trim();
    match v.split_once(':') {
        Some((e, rest)) if !e.is_empty() && e.bytes().all(|b| b.is_ascii_digit()) => (e.parse().unwrap_or(0), rest),
        _ => (0, v),
    }
}

fn weight(c: Option<u8>) -> i32 {
    match c {
        None => 0,
        Some(b'~') => -1,
        Some(c) if c.is_ascii_alphabetic() => c as i32,
        Some(c) => c as i32 + 256,
    }
}

fn debian(a: &[u8], b: &[u8]) -> Ordering {
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        // The non-digit run.
        while (i < a.len() && !a[i].is_ascii_digit()) || (j < b.len() && !b[j].is_ascii_digit()) {
            let ca = a.get(i).copied().filter(|c| !c.is_ascii_digit());
            let cb = b.get(j).copied().filter(|c| !c.is_ascii_digit());
            let (wa, wb) = (weight(ca), weight(cb));
            if wa != wb { return wa.cmp(&wb); }
            if ca.is_some() { i += 1; }
            if cb.is_some() { j += 1; }
        }
        // The digit run, as a number (leading zeros ignored, any length).
        while i < a.len() && a[i] == b'0' { i += 1; }
        while j < b.len() && b[j] == b'0' { j += 1; }
        let si = i;
        while i < a.len() && a[i].is_ascii_digit() { i += 1; }
        let sj = j;
        while j < b.len() && b[j].is_ascii_digit() { j += 1; }
        let (da, db) = (&a[si..i], &b[sj..j]);
        let ord = da.len().cmp(&db.len()).then_with(|| da.cmp(db));
        if ord != Ordering::Equal { return ord; }
    }
    Ordering::Equal
}

/// Orders two Kodi version strings.
pub fn compare(a: &str, b: &str) -> Ordering {
    let (ea, va) = split_epoch(a);
    let (eb, vb) = split_epoch(b);
    ea.cmp(&eb).then_with(|| debian(va.as_bytes(), vb.as_bytes()))
}

/// `a >= b`.
pub fn at_least(a: &str, b: &str) -> bool {
    compare(a, b) != Ordering::Less
}
