// SPDX-License-Identifier: GPL-3.0-only
//! Escaping for Loom's line-based dot files: one record per line, fields split by tabs.

use alloc::vec::Vec;

/// Escapes `\`, tab, newline, carriage return and `,` so a field fits on a line.
pub fn escape(s: &[u8], out: &mut Vec<u8>) {
    for &b in s {
        match b {
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\t' => out.extend_from_slice(b"\\t"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b',' => out.extend_from_slice(b"\\,"),
            _ => out.push(b),
        }
    }
}

pub fn unescape(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\\' && i + 1 < s.len() {
            out.push(match s[i + 1] { b't' => b'\t', b'n' => b'\n', b'r' => b'\r', c => c });
            i += 2;
        } else {
            out.push(s[i]);
            i += 1;
        }
    }
    out
}

/// Splits on unescaped `sep`.
pub fn split_unescaped(s: &[u8], sep: u8) -> Vec<&[u8]> {
    let mut parts = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < s.len() {
        if s[i] == b'\\' { i += 2; continue; }
        if s[i] == sep { parts.push(&s[start..i]); start = i + 1; }
        i += 1;
    }
    parts.push(&s[start.min(s.len())..]);
    parts
}

pub fn parse_u64(s: &[u8]) -> Option<u64> {
    if s.is_empty() || s.len() > 20 { return None; }
    s.iter().try_fold(0u64, |a, b| if b.is_ascii_digit() { a.checked_mul(10)?.checked_add((b - b'0') as u64) } else { None })
}

pub fn push_u64(out: &mut Vec<u8>, mut v: u64) {
    let mut buf = [0u8; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 { break; }
    }
    out.extend_from_slice(&buf[i..]);
}

/// Non-empty lines of a file, without `\r`.
pub fn lines(raw: &[u8]) -> impl Iterator<Item = &[u8]> {
    raw.split(|b| *b == b'\n').map(|l| l.strip_suffix(b"\r").unwrap_or(l)).filter(|l| !l.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip() {
        let s = b"a\\b\tc\nd,e\r";
        let mut e = Vec::new();
        escape(s, &mut e);
        assert!(!e.contains(&b'\t') && !e.contains(&b'\n'));
        assert_eq!(unescape(&e), s);
        assert_eq!(split_unescaped(b"a\\,b,c,,", b','), alloc::vec![&b"a\\,b"[..], b"c", b"", b""]);
        let mut n = Vec::new();
        push_u64(&mut n, 0);
        push_u64(&mut n, 18_446_744_073_709_551_615);
        assert_eq!(n, b"018446744073709551615");
        assert_eq!(parse_u64(b"18446744073709551616"), None);
    }
}
