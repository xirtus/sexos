// SPDX-License-Identifier: GPL-3.0-only
//! Batch rename with a live preview, in the manner of a multi-rename tool.
//!
//! A new name is built from two masks, one for the name and one for the extension, then
//! search-and-replace rules run over the result, then a case change. The masks know:
//!
//! | token | gives |
//! |---|---|
//! | `[N]` / `[E]` | the old name without its extension / the old extension |
//! | `[N2-5]` `[N3]` `[N2-]` `[N-3-]` `[N2,4]` | characters 2 to 5, the 3rd, 2nd to last, the last 3, 4 from the 2nd (counted from 1; negative counts from the end) |
//! | `[C]` | the counter with the rules' start, step and width; `[C10+5:3]` sets them inline |
//! | `[Y]` `[y]` `[M]` `[D]` `[h]` `[m]` `[s]` | the modification year (4 or 2 digits), month, day, hour, minute, second |
//! | `[U]` `[L]` `[F]` `[n]` | what follows in upper case, lower case, Title Case, or as it is |
//! | `[[` `]]` | a literal bracket |
//!
//! [`preview`] shows each item's new name with its problems (empty, `/` or NUL inside,
//! `.`/`..`, too long, two items aiming at one name, a name already taken by something
//! not being renamed). [`plan`] turns a clean preview into moves in an order that never
//! overwrites: a chain `a->b, b->c` runs back to front and a cycle such as the swap
//! `a->b, b->a` goes through a temporary name.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use crate::date;
use crate::glob::Glob;

/// One thing to rename: its name within its folder, and its modification time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item { pub name: Vec<u8>, pub mtime: u64, pub dir: bool }

impl Item {
    pub fn file(name: &[u8], mtime: u64) -> Item { Item { name: name.to_vec(), mtime, dir: false } }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Case { #[default] Keep, Upper, Lower, Title }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counter { pub start: i64, pub step: i64, pub width: usize }
impl Default for Counter { fn default() -> Self { Counter { start: 1, step: 1, width: 1 } } }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Find {
    /// Every occurrence of the text.
    #[default]
    Plain,
    /// A wildcard pattern over the whole name; `$1`..`$9` in the replacement are what its
    /// `*`, `?` and sets matched, `$0` the whole name, `$$` a dollar sign.
    Wildcard,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Replace { pub find: Vec<u8>, pub with: Vec<u8>, pub kind: Find, pub ignore_case: bool }

/// Everything the rename dialog holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rules {
    pub name: Vec<u8>,
    pub ext: Vec<u8>,
    pub counter: Counter,
    pub replaces: Vec<Replace>,
    /// Case change for the name part and for the extension, last of all.
    pub case: Case,
    pub ext_case: Case,
    /// Longest name allowed, in bytes.
    pub max_len: usize,
}

impl Default for Rules {
    fn default() -> Self {
        Rules { name: b"[N]".to_vec(), ext: b"[E]".to_vec(), counter: Counter::default(), replaces: Vec::new(), case: Case::Keep, ext_case: Case::Keep, max_len: 255 }
    }
}

/// A mask that can't be read: where (byte offset in the mask) and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternError { pub in_ext: bool, pub at: usize, pub why: &'static str }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part { Name, Ext }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Range { All, From(i64), FromTo(i64, i64), FromLen(i64, usize) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field { Year, Year2, Month, Day, Hour, Minute, Second }

#[derive(Clone, Debug, PartialEq, Eq)]
enum Piece { Text(Vec<u8>), Sub(Part, Range), Counter(Option<i64>, Option<i64>, Option<usize>), Date(Field), Case(Case) }

fn num(m: &[u8], i: &mut usize) -> Option<i64> {
    let neg = *i < m.len() && m[*i] == b'-' && *i + 1 < m.len() && m[*i + 1].is_ascii_digit();
    if neg { *i += 1; }
    let start = *i;
    let mut v: i64 = 0;
    while *i < m.len() && m[*i].is_ascii_digit() {
        v = v.saturating_mul(10).saturating_add((m[*i] - b'0') as i64);
        *i += 1;
    }
    if *i == start { if neg { *i -= 1; } return None; }
    Some(if neg { -v } else { v })
}

fn compile(m: &[u8], in_ext: bool) -> Result<Vec<Piece>, PatternError> {
    let err = |at, why| PatternError { in_ext, at, why };
    let mut out: Vec<Piece> = Vec::new();
    let mut text = Vec::new();
    let mut i = 0;
    while i < m.len() {
        let b = m[i];
        if b == b']' && m.get(i + 1) == Some(&b']') { text.push(b']'); i += 2; continue; }
        if b != b'[' { text.push(b); i += 1; continue; }
        if m.get(i + 1) == Some(&b'[') { text.push(b'['); i += 2; continue; }
        let start = i;
        let close = m[i..].iter().position(|c| *c == b']').map(|p| p + i).ok_or(err(start, "no closing ]"))?;
        let body = &m[i + 1..close];
        i = close + 1;
        if body.is_empty() { return Err(err(start, "empty []")); }
        if !text.is_empty() { out.push(Piece::Text(core::mem::take(&mut text))); }
        let piece = match body[0] {
            b'N' | b'E' => {
                let part = if body[0] == b'N' { Part::Name } else { Part::Ext };
                let mut j = 1;
                let range = if j == body.len() { Range::All } else {
                    let a = num(body, &mut j).ok_or(err(start, "expected a position"))?;
                    if a == 0 { return Err(err(start, "positions count from 1")); }
                    if j == body.len() { Range::FromTo(a, a) }
                    else if body[j] == b',' {
                        j += 1;
                        let n = num(body, &mut j).filter(|n| *n >= 0).ok_or(err(start, "expected a length"))?;
                        Range::FromLen(a, n as usize)
                    } else if body[j] == b'-' {
                        j += 1;
                        if j == body.len() { Range::From(a) } else {
                            let b = num(body, &mut j).ok_or(err(start, "expected an end position"))?;
                            if b == 0 { return Err(err(start, "positions count from 1")); }
                            Range::FromTo(a, b)
                        }
                    } else { return Err(err(start, "unexpected character in range")) }
                };
                if j != body.len() { return Err(err(start, "unexpected character in range")); }
                Piece::Sub(part, range)
            }
            b'C' => {
                let mut j = 1;
                let start_v = num(body, &mut j);
                let mut step = None;
                let mut width = None;
                if j < body.len() && body[j] == b'+' { j += 1; step = Some(num(body, &mut j).ok_or(err(start, "expected a step"))?); }
                if j < body.len() && body[j] == b':' {
                    j += 1;
                    width = Some(num(body, &mut j).filter(|w| (0..=32).contains(w)).ok_or(err(start, "expected a width up to 32"))? as usize);
                }
                if j != body.len() { return Err(err(start, "counter is [C], [C<start>+<step>:<width>]")); }
                Piece::Counter(start_v, step, width)
            }
            _ if body.len() == 1 => match body[0] {
                b'Y' => Piece::Date(Field::Year),
                b'y' => Piece::Date(Field::Year2),
                b'M' => Piece::Date(Field::Month),
                b'D' => Piece::Date(Field::Day),
                b'h' => Piece::Date(Field::Hour),
                b'm' => Piece::Date(Field::Minute),
                b's' => Piece::Date(Field::Second),
                b'U' => Piece::Case(Case::Upper),
                b'L' => Piece::Case(Case::Lower),
                b'F' => Piece::Case(Case::Title),
                b'n' => Piece::Case(Case::Keep),
                _ => return Err(err(start, "unknown token")),
            },
            _ => return Err(err(start, "unknown token")),
        };
        out.push(piece);
    }
    if !text.is_empty() { out.push(Piece::Text(text)); }
    Ok(out)
}

/// Splits a name into name part and extension at the last dot; a leading dot (`.profile`)
/// and folders have no extension.
pub fn split_ext(name: &[u8], dir: bool) -> (&[u8], &[u8]) {
    if dir { return (name, &[]); }
    match name.iter().rposition(|b| *b == b'.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name, &[]),
    }
}

/// Character boundaries: UTF-8 characters when the bytes are UTF-8, else single bytes.
fn bounds(s: &[u8]) -> Vec<usize> {
    match core::str::from_utf8(s) {
        Ok(t) => t.char_indices().map(|(i, _)| i).chain(core::iter::once(s.len())).collect(),
        Err(_) => (0..=s.len()).collect(),
    }
}

fn sub(s: &[u8], r: Range) -> &[u8] {
    let b = bounds(s);
    let n = b.len() as i64 - 1;
    // 1-based inclusive positions; negative counts from the end (-1 is the last).
    let pos = |p: i64| if p < 0 { n + p + 1 } else { p };
    let (from, to) = match r {
        Range::All => (1, n),
        Range::From(a) => (pos(a), n),
        Range::FromTo(a, z) => (pos(a), pos(z)),
        Range::FromLen(a, len) => (pos(a), pos(a) + len as i64 - 1),
    };
    let from = from.max(1);
    let to = to.min(n);
    if from > to { return &[]; }
    &s[b[(from - 1) as usize]..b[to as usize]]
}

fn pad(out: &mut Vec<u8>, v: i64, width: usize) {
    let digits = alloc::format!("{}", v.unsigned_abs());
    if v < 0 { out.push(b'-'); }
    for _ in digits.len()..width { out.push(b'0'); }
    out.extend_from_slice(digits.as_bytes());
}

/// Changes case; `word_start` carries Title Case across pieces.
fn apply_case(s: &[u8], case: Case, word_start: &mut bool) -> Vec<u8> {
    let track = |ws: &mut bool, alnum: bool| *ws = !alnum;
    match (case, core::str::from_utf8(s)) {
        (Case::Keep, _) => {
            if let Some(&last) = s.last() { track(word_start, last.is_ascii_alphanumeric() || last >= 0x80); }
            s.to_vec()
        }
        (_, Ok(t)) => {
            let mut out = String::with_capacity(t.len());
            for c in t.chars() {
                let up = match case { Case::Upper => true, Case::Lower => false, _ => *word_start };
                if up { out.extend(c.to_uppercase()) } else { out.extend(c.to_lowercase()) }
                track(word_start, c.is_alphanumeric());
            }
            out.into_bytes()
        }
        (_, Err(_)) => s.iter().map(|&b| {
            let up = match case { Case::Upper => true, Case::Lower => false, _ => *word_start };
            track(word_start, b.is_ascii_alphanumeric() || b >= 0x80);
            if up { b.to_ascii_uppercase() } else { b.to_ascii_lowercase() }
        }).collect(),
    }
}

fn find_plain(hay: &[u8], needle: &[u8], fold: bool, from: usize) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() { return None; }
    (from..=hay.len() - needle.len()).find(|&i| {
        let w = &hay[i..i + needle.len()];
        if fold { w.eq_ignore_ascii_case(needle) } else { w == needle }
    })
}

fn replace(name: &[u8], r: &Replace) -> Vec<u8> {
    match r.kind {
        Find::Plain => {
            let mut out = Vec::new();
            let mut at = 0;
            while let Some(i) = find_plain(name, &r.find, r.ignore_case, at) {
                out.extend_from_slice(&name[at..i]);
                out.extend_from_slice(&r.with);
                at = i + r.find.len();
            }
            out.extend_from_slice(&name[at..]);
            out
        }
        Find::Wildcard => {
            let Some(caps) = Glob::new(&r.find, r.ignore_case).captures(name) else { return name.to_vec() };
            let mut out = Vec::new();
            let w = &r.with;
            let mut i = 0;
            while i < w.len() {
                if w[i] == b'$' && i + 1 < w.len() {
                    match w[i + 1] {
                        b'$' => { out.push(b'$'); i += 2; continue; }
                        b'0' => { out.extend_from_slice(name); i += 2; continue; }
                        d @ b'1'..=b'9' => {
                            if let Some(&(s, e)) = caps.get((d - b'1') as usize) { out.extend_from_slice(&name[s..e]); }
                            i += 2;
                            continue;
                        }
                        _ => {}
                    }
                }
                out.push(w[i]);
                i += 1;
            }
            out
        }
    }
}

/// Compiled rules, ready to name many items.
pub struct Renamer { name: Vec<Piece>, ext: Vec<Piece>, rules: Rules }

impl Renamer {
    pub fn new(rules: &Rules) -> Result<Renamer, PatternError> {
        Ok(Renamer { name: compile(&rules.name, false)?, ext: compile(&rules.ext, true)?, rules: rules.clone() })
    }

    fn render(&self, pieces: &[Piece], item: &Item, index: usize) -> Vec<u8> {
        let (n, e) = split_ext(&item.name, item.dir);
        let dt = date::from_secs(item.mtime);
        let mut out = Vec::new();
        let mut case = Case::Keep;
        let mut ws = true;
        for p in pieces {
            let mut seg = Vec::new();
            match p {
                Piece::Text(t) => seg.extend_from_slice(t),
                Piece::Sub(part, r) => seg.extend_from_slice(sub(if *part == Part::Name { n } else { e }, *r)),
                Piece::Counter(s, st, w) => {
                    let c = &self.rules.counter;
                    let v = s.unwrap_or(c.start).saturating_add(st.unwrap_or(c.step).saturating_mul(index as i64));
                    pad(&mut seg, v, w.unwrap_or(c.width));
                }
                Piece::Date(f) => match f {
                    Field::Year => pad(&mut seg, dt.year, 4),
                    Field::Year2 => pad(&mut seg, dt.year.rem_euclid(100), 2),
                    Field::Month => pad(&mut seg, dt.month as i64, 2),
                    Field::Day => pad(&mut seg, dt.day as i64, 2),
                    Field::Hour => pad(&mut seg, dt.hour as i64, 2),
                    Field::Minute => pad(&mut seg, dt.minute as i64, 2),
                    Field::Second => pad(&mut seg, dt.second as i64, 2),
                },
                Piece::Case(c) => { case = *c; continue; }
            }
            out.extend(apply_case(&seg, case, &mut ws));
        }
        out
    }

    /// The new name of the `index`th selected item.
    pub fn name(&self, item: &Item, index: usize) -> Vec<u8> {
        let base = self.render(&self.name, item, index);
        let ext = self.render(&self.ext, item, index);
        let mut full = base;
        if !ext.is_empty() { full.push(b'.'); full.extend_from_slice(&ext); }
        for r in &self.rules.replaces { full = replace(&full, r); }
        if self.rules.case == Case::Keep && self.rules.ext_case == Case::Keep { return full; }
        let (n, e) = split_ext(&full, item.dir);
        let mut out = apply_case(n, self.rules.case, &mut true);
        if !e.is_empty() || full.len() > n.len() {
            out.push(b'.');
            out.extend(apply_case(e, self.rules.ext_case, &mut true));
        }
        out
    }
}

/// Why a new name can't be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    Empty,
    /// Contains `/` or NUL.
    BadChar(u8),
    /// `.` or `..`.
    Dots,
    TooLong,
    /// Another item in the batch gets the same name.
    Duplicate,
    /// Something in the folder that isn't being renamed already has this name.
    Exists,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line { pub from: Vec<u8>, pub to: Vec<u8>, pub problems: Vec<Problem> }
impl Line {
    pub fn changed(&self) -> bool { self.from != self.to }
    pub fn ok(&self) -> bool { self.problems.is_empty() }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview { pub lines: Vec<Line> }
impl Preview {
    pub fn ok(&self) -> bool { self.lines.iter().all(Line::ok) }
    pub fn problems(&self) -> usize { self.lines.iter().filter(|l| !l.ok()).count() }
    pub fn changes(&self) -> usize { self.lines.iter().filter(|l| l.changed()).count() }
}

/// New names for `items` (in selection order, which the counter follows), checked against
/// `existing`: every name in the folder, the items' own included.
pub fn preview(rules: &Rules, items: &[Item], existing: &[Vec<u8>]) -> Result<Preview, PatternError> {
    let r = Renamer::new(rules)?;
    let sources: BTreeSet<&[u8]> = items.iter().map(|i| &i.name[..]).collect();
    let others: BTreeSet<&[u8]> = existing.iter().map(|e| &e[..]).filter(|e| !sources.contains(e)).collect();
    let mut lines: Vec<Line> = items.iter().enumerate().map(|(k, it)| Line { from: it.name.clone(), to: r.name(it, k), problems: Vec::new() }).collect();
    let mut count: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
    for l in &lines { *count.entry(l.to.clone()).or_default() += 1; }
    for l in &mut lines {
        let t = &l.to;
        if t.is_empty() { l.problems.push(Problem::Empty); }
        if let Some(&b) = t.iter().find(|b| **b == b'/' || **b == 0) { l.problems.push(Problem::BadChar(b)); }
        if t == b"." || t == b".." { l.problems.push(Problem::Dots); }
        if t.len() > rules.max_len { l.problems.push(Problem::TooLong); }
        if count[t] > 1 && !t.is_empty() { l.problems.push(Problem::Duplicate); }
        if l.changed() && others.contains(&t[..]) { l.problems.push(Problem::Exists); }
    }
    Ok(Preview { lines })
}

/// One rename, within the folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Move { pub from: Vec<u8>, pub to: Vec<u8> }

/// The preview still has problems; nothing should be renamed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Blocked { pub problems: usize }

/// The moves for a clean preview, in a safe order. `existing` is every name in the folder,
/// used to pick temporary names that collide with nothing.
pub fn plan(preview: &Preview, existing: &[Vec<u8>]) -> Result<Vec<Move>, Blocked> {
    if !preview.ok() { return Err(Blocked { problems: preview.problems() }); }
    let pairs: Vec<(Vec<u8>, Vec<u8>)> = preview.lines.iter().filter(|l| l.changed()).map(|l| (l.from.clone(), l.to.clone())).collect();
    Ok(order_moves(&pairs, existing))
}

/// Orders simultaneous renames `from -> to` (distinct sources, distinct targets) so that no
/// move lands on a name still in use: chains run back to front, cycles go through a
/// temporary `.loom-rename-N` name that collides with nothing in `taken` or the batch.
pub fn order_moves(pairs: &[(Vec<u8>, Vec<u8>)], taken: &[Vec<u8>]) -> Vec<Move> {
    // source -> target, still to do.
    let mut pending: BTreeMap<Vec<u8>, Vec<u8>> = pairs.iter().filter(|(f, t)| f != t).cloned().collect();
    let mut used: BTreeSet<Vec<u8>> = taken.iter().cloned().collect();
    for (f, t) in pairs { used.insert(f.clone()); used.insert(t.clone()); }
    let mut out = Vec::new();
    let mut tmp_n = 0usize;
    // The targets still blocked by a pending source.
    while !pending.is_empty() {
        let ready: Vec<Vec<u8>> = pending.iter().filter(|(_, t)| !pending.contains_key(*t)).map(|(f, _)| f.clone()).collect();
        if !ready.is_empty() {
            for f in ready {
                let t = pending.remove(&f).unwrap_or_default();
                out.push(Move { from: f, to: t });
            }
            continue;
        }
        // Everything left is in cycles: park one source under a temporary name.
        let (f, t) = pending.pop_first().unwrap_or_default();
        let tmp = loop {
            let mut c = b".loom-rename-".to_vec();
            crate::text::push_u64(&mut c, tmp_n as u64);
            tmp_n += 1;
            if !used.contains(&c) { break c; }
        };
        used.insert(tmp.clone());
        out.push(Move { from: f, to: tmp.clone() });
        pending.insert(tmp, t);
    }
    out
}

/// The moves that undo `moves`.
pub fn undo(moves: &[Move]) -> Vec<Move> {
    moves.iter().rev().map(|m| Move { from: m.to.clone(), to: m.from.clone() }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn names(v: &[&str]) -> Vec<Vec<u8>> { v.iter().map(|s| s.as_bytes().to_vec()).collect() }
    fn items(v: &[&str]) -> Vec<Item> { v.iter().map(|s| Item::file(s.as_bytes(), 0)).collect() }
    fn s(b: &[u8]) -> &str { core::str::from_utf8(b).unwrap() }
    fn rules(name: &str, ext: &str) -> Rules { Rules { name: name.into(), ext: ext.into(), ..Rules::default() } }
    fn one(r: &Rules, name: &str, mtime: u64, index: usize) -> String {
        String::from(s(&Renamer::new(r).unwrap().name(&Item::file(name.as_bytes(), mtime), index)))
    }

    /// Runs moves over a folder and fails on any overwrite or missing source.
    fn apply(folder: &mut BTreeSet<Vec<u8>>, moves: &[Move]) {
        for m in moves {
            assert!(folder.remove(&m.from), "missing source {}", s(&m.from));
            assert!(folder.insert(m.to.clone()), "overwrote {}", s(&m.to));
        }
    }

    #[test]
    fn tokens() {
        let r = rules("[N]", "[E]");
        assert_eq!(one(&r, "photo.jpg", 0, 0), "photo.jpg");
        assert_eq!(one(&r, ".profile", 0, 0), ".profile");
        assert_eq!(one(&r, "archive.tar.gz", 0, 0), "archive.tar.gz");
        assert_eq!(one(&rules("[N2-4]", "[E]"), "abcdef.txt", 0, 0), "bcd.txt");
        assert_eq!(one(&rules("[N3]", ""), "abcdef.txt", 0, 0), "c");
        assert_eq!(one(&rules("[N3-]", "[E1]"), "abcdef.txt", 0, 0), "cdef.t");
        assert_eq!(one(&rules("[N-2-]", "[E]"), "abcdef.txt", 0, 0), "ef.txt");
        assert_eq!(one(&rules("[N2,3]", "[E]"), "abcdef.txt", 0, 0), "bcd.txt");
        assert_eq!(one(&rules("[N1--2]", "[E]"), "abcdef.txt", 0, 0), "abcde.txt");
        assert_eq!(one(&rules("[N5-20]", "[E]"), "abc.txt", 0, 0), ".txt");
        // UTF-8 names count characters, not bytes.
        assert_eq!(one(&rules("[N2-3]", "[E]"), "äöüß.txt", 0, 0), "öü.txt");
        assert_eq!(one(&rules("[[[N]]]", "[E]"), "a.b", 0, 0), "[a].b");
    }

    #[test]
    fn counter_and_dates() {
        let mut r = rules("pic_[C]", "[E]");
        r.counter = Counter { start: 7, step: 3, width: 3 };
        assert_eq!(one(&r, "x.png", 0, 0), "pic_007.png");
        assert_eq!(one(&r, "x.png", 0, 2), "pic_013.png");
        assert_eq!(one(&rules("[C10+5:4]", "[E]"), "x.png", 0, 3), "0025.png");
        assert_eq!(one(&rules("[C5+-2:2]", ""), "x", 0, 4), "-03");
        // 2023-11-14 22:13:20 UTC
        assert_eq!(one(&rules("[Y]-[M]-[D]_[h][m][s]_[N]", "[E]"), "a.jpg", 1_700_000_000, 0), "2023-11-14_221320_a.jpg");
        assert_eq!(one(&rules("[y][M]", ""), "a", 1_700_000_000, 0), "2311");
    }

    #[test]
    fn cases() {
        let mut r = rules("[N]", "[E]");
        r.case = Case::Title;
        r.ext_case = Case::Lower;
        assert_eq!(one(&r, "hello WORLD-foo_bar.JPG", 0, 0), "Hello World-Foo_Bar.jpg");
        r.case = Case::Upper;
        assert_eq!(one(&r, "straße.TXT", 0, 0), "STRASSE.txt");
        assert_eq!(one(&rules("[U][N1][L][N2-]", "[E]"), "mIXED.txt", 0, 0), "Mixed.txt");
        assert_eq!(one(&rules("[F]my file [N]", "[E]"), "the END.txt", 0, 0), "My File The End.txt");
        r.case = Case::Lower;
        r.ext_case = Case::Keep;
        assert_eq!(one(&r, "README", 0, 0), "readme");
    }

    #[test]
    fn replaces() {
        let mut r = rules("[N]", "[E]");
        r.replaces.push(Replace { find: b" ".to_vec(), with: b"_".to_vec(), ..Replace::default() });
        assert_eq!(one(&r, "my holiday photo.jpg", 0, 0), "my_holiday_photo.jpg");
        r.replaces = vec![Replace { find: b"IMG".to_vec(), with: b"Img".to_vec(), kind: Find::Plain, ignore_case: true }];
        assert_eq!(one(&r, "img_1 IMG_2.jpg", 0, 0), "Img_1 Img_2.jpg");
        r.replaces = vec![Replace { find: b"IMG_*.*".to_vec(), with: b"photo-$1.$2 ($0) $$".to_vec(), kind: Find::Wildcard, ignore_case: false }];
        assert_eq!(one(&r, "IMG_0042.jpg", 0, 0), "photo-0042.jpg (IMG_0042.jpg) $");
        assert_eq!(one(&r, "DSC_1.jpg", 0, 0), "DSC_1.jpg");
        // Replaces run before the case change.
        r.replaces = vec![Replace { find: b"*-*".to_vec(), with: b"$2 $1".to_vec(), kind: Find::Wildcard, ignore_case: false }];
        r.case = Case::Title;
        assert_eq!(one(&r, "artist-song", 0, 0), "Song Artist");
    }

    #[test]
    fn bad_masks() {
        assert_eq!(Renamer::new(&rules("[N", "")).err().map(|e| e.why), Some("no closing ]"));
        assert_eq!(Renamer::new(&rules("[Q]", "")).err().map(|e| e.at), Some(0));
        assert!(Renamer::new(&rules("[N0]", "")).is_err());
        assert!(Renamer::new(&rules("[N2x]", "")).is_err());
        let e = Renamer::new(&rules("[N]", "x[]")).err().unwrap();
        assert!(e.in_ext && e.at == 1);
        assert!(Renamer::new(&rules("[C:99]", "")).is_err());
    }

    #[test]
    fn twenty_files_by_pattern() {
        let list: Vec<Item> = (0..20).map(|i| Item::file(alloc::format!("DSC{:04}.JPG", 100 + i * 7).as_bytes(), 1_700_000_000 + i as u64 * 86_400)).collect();
        let existing: Vec<Vec<u8>> = list.iter().map(|i| i.name.clone()).chain([b"notes.txt".to_vec()]).collect();
        let mut r = rules("Holiday [Y]-[M]-[D] [C]", "[E]");
        r.counter = Counter { start: 1, step: 1, width: 2 };
        r.ext_case = Case::Lower;
        let p = preview(&r, &list, &existing).unwrap();
        assert!(p.ok());
        assert_eq!(p.changes(), 20);
        assert_eq!(s(&p.lines[0].to), "Holiday 2023-11-14 01.jpg");
        assert_eq!(s(&p.lines[19].to), "Holiday 2023-12-03 20.jpg");
        let moves = plan(&p, &existing).unwrap();
        assert_eq!(moves.len(), 20);
        let mut folder: BTreeSet<Vec<u8>> = existing.iter().cloned().collect();
        apply(&mut folder, &moves);
        assert_eq!(folder.len(), 21);
        assert!(folder.contains(&b"Holiday 2023-11-20 07.jpg"[..].to_vec()));
        apply(&mut folder, &undo(&moves));
        assert_eq!(folder, existing.iter().cloned().collect());
    }

    #[test]
    fn conflicts() {
        let list = items(&["a.txt", "b.txt", "c.txt"]);
        let existing = names(&["a.txt", "b.txt", "c.txt", "x.txt", "keep"]);
        // Everything to one name.
        let p = preview(&rules("same", "[E]"), &list, &existing).unwrap();
        assert!(p.lines.iter().all(|l| l.problems == [Problem::Duplicate]));
        assert_eq!(plan(&p, &existing), Err(Blocked { problems: 3 }));
        // Onto a name that is taken and not being renamed.
        let mut r = rules("[N]", "[E]");
        r.replaces = vec![Replace { find: b"a".to_vec(), with: b"x".to_vec(), ..Replace::default() }];
        let p = preview(&r, &list, &existing).unwrap();
        assert_eq!(p.lines[0].problems, [Problem::Exists]);
        assert!(p.lines[1].ok() && !p.lines[1].changed());
        // Bad characters, empty and dots.
        let p = preview(&rules("a/[N]", "[E]"), &items(&["z"]), &[]).unwrap();
        assert_eq!(p.lines[0].problems, [Problem::BadChar(b'/')]);
        let p = preview(&rules("[N]\0", ""), &items(&["z"]), &[]).unwrap();
        assert_eq!(p.lines[0].problems, [Problem::BadChar(0)]);
        let p = preview(&rules("", ""), &items(&["z"]), &[]).unwrap();
        assert_eq!(p.lines[0].problems, [Problem::Empty]);
        let p = preview(&rules("..", ""), &items(&["z"]), &[]).unwrap();
        assert_eq!(p.lines[0].problems, [Problem::Dots]);
        let mut long = rules("[N][N][N]", "");
        long.max_len = 8;
        let p = preview(&long, &items(&["abc"]), &[]).unwrap();
        assert_eq!(p.lines[0].problems, [Problem::TooLong]);
        // Renaming onto a name another selected item is leaving is fine.
        let p = preview(&rules("[N]x", "[E]"), &items(&["a.t", "ax.t"]), &names(&["a.t", "ax.t"])).unwrap();
        assert!(p.ok());
    }

    #[test]
    fn swap_goes_through_a_temporary_name() {
        let existing = names(&["a", "b", ".loom-rename-0"]);
        let moves = order_moves(&[(b"a".to_vec(), b"b".to_vec()), (b"b".to_vec(), b"a".to_vec())], &existing);
        assert_eq!(moves.len(), 3);
        assert_eq!(s(&moves[0].to), ".loom-rename-1", "skips a temporary name already taken");
        let mut folder: BTreeSet<Vec<u8>> = existing.iter().cloned().collect();
        let marked: BTreeMap<Vec<u8>, &str> = [(b"a".to_vec(), "A"), (b"b".to_vec(), "B")].into_iter().collect();
        // Track contents to prove the swap really swapped.
        let mut content = marked.clone();
        for m in &moves {
            let c = content.remove(&m.from).unwrap();
            assert!(content.insert(m.to.clone(), c).is_none());
        }
        assert_eq!(content[&b"a".to_vec()], "B");
        assert_eq!(content[&b"b".to_vec()], "A");
        apply(&mut folder, &moves);
        assert_eq!(folder, existing.iter().cloned().collect());
    }

    #[test]
    fn chains_and_rotations() {
        // a->b->c->d chain: d first.
        let pairs: Vec<(Vec<u8>, Vec<u8>)> = vec![(b"a".to_vec(), b"b".to_vec()), (b"b".to_vec(), b"c".to_vec()), (b"c".to_vec(), b"d".to_vec())];
        let moves = order_moves(&pairs, &names(&["a", "b", "c"]));
        assert_eq!(moves.len(), 3);
        assert_eq!(s(&moves[0].from), "c");
        let mut folder: BTreeSet<Vec<u8>> = names(&["a", "b", "c"]).into_iter().collect();
        apply(&mut folder, &moves);
        assert_eq!(folder, names(&["b", "c", "d"]).into_iter().collect());
        // A rotation of 1..=5 by one, plus a separate swap, as a counter rename would cause.
        let list = items(&["1", "2", "3", "4", "5", "x", "y"]);
        let to = ["2", "3", "4", "5", "1", "y", "x"];
        let pairs: Vec<(Vec<u8>, Vec<u8>)> = list.iter().zip(to).map(|(i, t)| (i.name.clone(), t.as_bytes().to_vec())).collect();
        let existing: Vec<Vec<u8>> = list.iter().map(|i| i.name.clone()).collect();
        let moves = order_moves(&pairs, &existing);
        assert_eq!(moves.len(), 9, "one temporary per cycle");
        let mut content: BTreeMap<Vec<u8>, Vec<u8>> = existing.iter().map(|n| (n.clone(), n.clone())).collect();
        for m in &moves {
            let c = content.remove(&m.from).unwrap();
            assert!(content.insert(m.to.clone(), c).is_none());
        }
        for (f, t) in &pairs { assert_eq!(&content[t], f); }
    }

    #[test]
    fn counter_renumbering_shifts_safely() {
        // 01..10 renamed to 02..11 by [C2:2]: every target but the last is a source.
        let list: Vec<Item> = (1..=10).map(|i| Item::file(alloc::format!("{:02}", i).as_bytes(), 0)).collect();
        let existing: Vec<Vec<u8>> = list.iter().map(|i| i.name.clone()).collect();
        let p = preview(&rules("[C2:2]", ""), &list, &existing).unwrap();
        assert!(p.ok());
        let moves = plan(&p, &existing).unwrap();
        assert_eq!(moves.len(), 10);
        assert_eq!(s(&moves[0].from), "10");
        let mut folder: BTreeSet<Vec<u8>> = existing.into_iter().collect();
        apply(&mut folder, &moves);
        assert!(folder.contains(&b"11"[..].to_vec()) && !folder.contains(&b"01"[..].to_vec()));
    }
}
