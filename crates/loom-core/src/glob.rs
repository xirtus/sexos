// SPDX-License-Identifier: GPL-3.0-only
//! Wildcards over bytes: `*` any run, `?` one byte, `[abc]` / `[a-z]` / `[!x]` one of a set,
//! `\x` the byte itself. Matching is linear-time backtracking to the last `*`, so a pattern
//! can't blow up. Each `*` and `?` also captures what it matched, for rename's `$1`..`$9`.

use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok { Lit(u8), Any, Star, Class { neg: bool, ranges: Vec<(u8, u8)> } }

/// A parsed wildcard pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glob { toks: Vec<Tok>, fold: bool }

fn lower(b: u8) -> u8 { b.to_ascii_lowercase() }

impl Glob {
    /// Parses `pat`; `fold` makes ASCII letters match either case.
    pub fn new(pat: &[u8], fold: bool) -> Glob {
        let mut toks = Vec::new();
        let mut i = 0;
        while i < pat.len() {
            let b = pat[i];
            i += 1;
            match b {
                b'*' => if toks.last() != Some(&Tok::Star) { toks.push(Tok::Star) },
                b'?' => toks.push(Tok::Any),
                b'\\' if i < pat.len() => { toks.push(Tok::Lit(pat[i])); i += 1; }
                b'[' => {
                    // A set runs to the next `]` (a `]` right after `[` or `[!` is literal).
                    let mut j = i;
                    let neg = j < pat.len() && (pat[j] == b'!' || pat[j] == b'^');
                    if neg { j += 1; }
                    let start = j;
                    while j < pat.len() && (pat[j] != b']' || j == start) { j += 1; }
                    if j >= pat.len() { toks.push(Tok::Lit(b'[')); continue; }
                    let body = &pat[start..j];
                    let mut ranges = Vec::new();
                    let mut k = 0;
                    while k < body.len() {
                        if k + 2 < body.len() && body[k + 1] == b'-' { ranges.push((body[k], body[k + 2])); k += 3; }
                        else { ranges.push((body[k], body[k])); k += 1; }
                    }
                    toks.push(Tok::Class { neg, ranges });
                    i = j + 1;
                }
                _ => toks.push(Tok::Lit(b)),
            }
        }
        Glob { toks, fold }
    }

    /// True when the pattern has no wildcard at all.
    pub fn is_literal(&self) -> bool { self.toks.iter().all(|t| matches!(t, Tok::Lit(_))) }

    fn one(&self, t: &Tok, b: u8) -> bool {
        match t {
            Tok::Lit(c) => if self.fold { lower(*c) == lower(b) } else { *c == b },
            Tok::Any => true,
            Tok::Star => false,
            Tok::Class { neg, ranges } => {
                let hit = ranges.iter().any(|&(lo, hi)| {
                    (lo..=hi).contains(&b) || (self.fold && ((lo..=hi).contains(&lower(b)) || (lo..=hi).contains(&b.to_ascii_uppercase())))
                });
                hit != *neg
            }
        }
    }

    /// Matches the whole of `text`.
    pub fn matches(&self, text: &[u8]) -> bool { self.captures(text).is_some() }

    /// Matches the whole of `text`; on a match, the byte range each `*`, `?` and set took, in order.
    pub fn captures(&self, text: &[u8]) -> Option<Vec<(usize, usize)>> {
        let toks = &self.toks;
        let mut caps = alloc::vec![(0usize, 0usize); toks.len()];
        let (mut p, mut t) = (0usize, 0usize);
        let mut star: Option<(usize, usize, usize)> = None; // token, where it started, how far it reaches
        while t < text.len() {
            if p < toks.len() && toks[p] == Tok::Star {
                star = Some((p, t, t));
                caps[p] = (t, t);
                p += 1;
                continue;
            }
            if p < toks.len() && self.one(&toks[p], text[t]) {
                caps[p] = (t, t + 1);
                p += 1;
                t += 1;
                continue;
            }
            match star {
                Some((sp, from, reach)) => {
                    let reach = reach + 1;
                    star = Some((sp, from, reach));
                    caps[sp] = (from, reach);
                    p = sp + 1;
                    t = reach;
                }
                None => return None,
            }
        }
        while p < toks.len() && toks[p] == Tok::Star { caps[p] = (t, t); p += 1; }
        if p != toks.len() { return None; }
        Some(toks.iter().zip(caps).filter(|(k, _)| !matches!(k, Tok::Lit(_))).map(|(_, c)| c).collect())
    }
}

/// One-off match of `pat` against `text`.
pub fn matches(pat: &[u8], text: &[u8], fold: bool) -> bool { Glob::new(pat, fold).matches(text) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn basics() {
        assert!(matches(b"*report*", b"q3-report-final.pdf", false));
        assert!(!matches(b"*report*", b"q3-Report.pdf", false));
        assert!(matches(b"*report*", b"q3-Report.pdf", true));
        assert!(matches(b"a?c", b"abc", false));
        assert!(!matches(b"a?c", b"ac", false));
        assert!(matches(b"*", b"", false));
        assert!(matches(b"", b"", false));
        assert!(!matches(b"", b"x", false));
        assert!(matches(b"img_[0-9][0-9].jpg", b"img_07.jpg", false));
        assert!(!matches(b"img_[!0-9]*", b"img_07.jpg", false));
        assert!(matches(b"[A-Z]*", b"x", true));
        assert!(matches(b"a\\*b", b"a*b", false));
        assert!(!matches(b"a\\*b", b"axb", false));
        assert!(matches(b"*.tar.gz", b"x.tar.gz", false));
        assert!(matches(b"a*b*c", b"aXbYbZc", false));
        assert!(matches(b"[]x]", b"]", false));
        assert!(matches(b"[x", b"[x", false));
    }
    #[test]
    fn caps() {
        let g = Glob::new(b"IMG_*.*", false);
        let c = g.captures(b"IMG_1234.JPG").unwrap();
        assert_eq!(c, alloc::vec![(4, 8), (9, 12)]);
        let g = Glob::new(b"*-?-*", false);
        assert_eq!(g.captures(b"ab-c-de").unwrap(), alloc::vec![(0, 2), (3, 4), (5, 7)]);
    }
    #[test]
    fn no_blowup() {
        let text = [b'a'; 4000];
        let pat = b"*a*a*a*a*a*a*a*a*b";
        assert!(!matches(pat, &text, false));
    }
}
