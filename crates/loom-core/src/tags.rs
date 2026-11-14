// SPDX-License-Identifier: GPL-3.0-only
//! What Loom remembers about files beyond the disk: tags, a rating (0-5), a colour label and
//! a comment per path ([`Store`]), smart folders ([`query`]), recent files by frecency
//! ([`recent`]) and each folder's view ([`views`]).
//!
//! Everything is keyed by path and saved as small line-based dot files (one record per line,
//! tab-separated, escaped with [`crate::text`] rules), so a damaged line loses one record and
//! nothing else. When Loom moves or renames something it calls `moved`, and the records go
//! with it, including everything under a moved folder.

pub mod query;
pub mod recent;
pub mod views;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use crate::text::{escape, lines, parse_u64, push_u64, split_unescaped, unescape};
use crate::{is_within, rebase};

/// A colour label, as most file managers offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Color { #[default] None, Red, Orange, Yellow, Green, Blue, Purple, Gray }

impl Color {
    pub const ALL: [Color; 8] = [Color::None, Color::Red, Color::Orange, Color::Yellow, Color::Green, Color::Blue, Color::Purple, Color::Gray];
    pub fn name(self) -> &'static str {
        ["none", "red", "orange", "yellow", "green", "blue", "purple", "gray"][self as usize]
    }
    pub fn parse(s: &[u8]) -> Option<Color> {
        if s.eq_ignore_ascii_case(b"grey") { return Some(Color::Gray); }
        Color::ALL.into_iter().find(|c| s.eq_ignore_ascii_case(c.name().as_bytes()))
    }
}

/// What Loom knows about one file.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Meta { pub tags: BTreeSet<Vec<u8>>, pub rating: u8, pub color: Color, pub comment: Vec<u8> }

impl Meta {
    pub fn is_empty(&self) -> bool { self.tags.is_empty() && self.rating == 0 && self.color == Color::None && self.comment.is_empty() }
    pub fn has_tag(&self, tag: &[u8]) -> bool { self.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) }
}

/// Metadata for many files, by path.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Store { map: BTreeMap<Vec<u8>, Meta>, changed: bool }

const HEADER: &[u8] = b"loom-meta 1";

impl Store {
    pub fn new() -> Store { Store::default() }
    pub fn get(&self, path: &[u8]) -> Option<&Meta> { self.map.get(path) }
    pub fn len(&self) -> usize { self.map.len() }
    pub fn is_empty(&self) -> bool { self.map.is_empty() }
    /// True once something changed since the last [`Store::to_bytes`]/[`Store::saved`].
    pub fn changed(&self) -> bool { self.changed }
    pub fn saved(&mut self) { self.changed = false; }
    pub fn iter(&self) -> impl Iterator<Item = (&Vec<u8>, &Meta)> { self.map.iter() }

    /// Changes a file's record; records left empty are dropped.
    pub fn edit(&mut self, path: &[u8], f: impl FnOnce(&mut Meta)) {
        let mut m = self.map.remove(path).unwrap_or_default();
        let before = m.clone();
        f(&mut m);
        m.rating = m.rating.min(5);
        if m != before { self.changed = true; }
        if !m.is_empty() { self.map.insert(path.to_vec(), m); }
    }
    pub fn tag(&mut self, path: &[u8], tag: &[u8]) {
        let tag = tag.trim_ascii();
        if !tag.is_empty() { self.edit(path, |m| { m.tags.insert(tag.to_vec()); }); }
    }
    pub fn untag(&mut self, path: &[u8], tag: &[u8]) { self.edit(path, |m| { m.tags.retain(|t| !t.eq_ignore_ascii_case(tag)); }); }
    pub fn rate(&mut self, path: &[u8], stars: u8) { self.edit(path, |m| m.rating = stars.min(5)); }
    pub fn label(&mut self, path: &[u8], color: Color) { self.edit(path, |m| m.color = color); }
    pub fn comment(&mut self, path: &[u8], text: &[u8]) { self.edit(path, |m| m.comment = text.to_vec()); }

    /// `from` (a file or a folder) is now at `to`: its records, and those under it, follow.
    pub fn moved(&mut self, from: &[u8], to: &[u8]) {
        let keys: Vec<Vec<u8>> = self.map.range(from.to_vec()..).take_while(|(k, _)| k.starts_with(from)).filter(|(k, _)| is_within(k, from)).map(|(k, _)| k.clone()).collect();
        let moving: Vec<(Vec<u8>, Meta)> = keys.into_iter().filter_map(|k| self.map.remove(&k).map(|m| (k, m))).collect();
        for (k, m) in moving { self.map.insert(rebase(&k, from, to), m); self.changed = true; }
    }
    /// `from` was copied to `to`: the copy gets the same records.
    pub fn copied(&mut self, from: &[u8], to: &[u8]) {
        let copies: Vec<(Vec<u8>, Meta)> = self.map.iter().filter(|(k, _)| is_within(k, from)).map(|(k, m)| (rebase(k, from, to), m.clone())).collect();
        for (k, m) in copies { self.map.insert(k, m); self.changed = true; }
    }
    /// `path` (and anything under it) is gone.
    pub fn removed(&mut self, path: &[u8]) {
        let n = self.map.len();
        self.map.retain(|k, _| !is_within(k, path));
        if self.map.len() != n { self.changed = true; }
    }
    /// Every tag in use, with how many files carry it.
    pub fn all_tags(&self) -> BTreeMap<Vec<u8>, usize> {
        let mut out = BTreeMap::new();
        for m in self.map.values() { for t in &m.tags { *out.entry(t.clone()).or_default() += 1; } }
        out
    }

    /// `loom-meta 1`, then per file: `path<TAB>rating<TAB>colour<TAB>tag,tag<TAB>comment`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = HEADER.to_vec();
        out.push(b'\n');
        for (p, m) in &self.map {
            escape(p, &mut out);
            out.push(b'\t');
            push_u64(&mut out, m.rating as u64);
            out.push(b'\t');
            out.extend_from_slice(m.color.name().as_bytes());
            out.push(b'\t');
            for (i, t) in m.tags.iter().enumerate() {
                if i > 0 { out.push(b','); }
                escape(t, &mut out);
            }
            out.push(b'\t');
            escape(&m.comment, &mut out);
            out.push(b'\n');
        }
        out
    }

    /// Reads a store back; lines it can't read are skipped. `None` if it isn't one.
    pub fn from_bytes(raw: &[u8]) -> Option<Store> {
        let mut it = lines(raw);
        if it.next()? != HEADER { return None; }
        let mut map = BTreeMap::new();
        for l in it {
            let f = split_unescaped(l, b'\t');
            if f.len() != 5 { continue; }
            let (Some(rating), Some(color)) = (parse_u64(f[1]), Color::parse(f[2])) else { continue };
            let tags = if f[3].is_empty() { BTreeSet::new() } else { split_unescaped(f[3], b',').into_iter().map(unescape).collect() };
            let m = Meta { tags, rating: rating.min(5) as u8, color, comment: unescape(f[4]) };
            if !m.is_empty() { map.insert(unescape(f[0]), m); }
        }
        Some(Store { map, changed: false })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_and_round_trip() {
        let mut s = Store::new();
        s.tag(b"docs/a.pdf", b"work");
        s.tag(b"docs/a.pdf", b"tax,2026");
        s.rate(b"docs/a.pdf", 9);
        s.label(b"docs/a.pdf", Color::Red);
        s.comment(b"docs/a.pdf", b"line one\nline\ttwo \\ end");
        s.tag(b"odd\tname\n", b"x");
        s.rate(b"plain", 2);
        assert!(s.changed());
        assert_eq!(s.get(b"docs/a.pdf").unwrap().rating, 5);
        let bytes = s.to_bytes();
        assert_eq!(bytes.iter().filter(|b| **b == b'\n').count(), 4);
        let back = Store::from_bytes(&bytes).unwrap();
        assert_eq!(back.map, s.map);
        assert!(back.get(b"docs/a.pdf").unwrap().has_tag(b"TAX,2026"));
        // Clearing everything drops the record.
        s.rate(b"plain", 0);
        assert!(s.get(b"plain").is_none());
        assert_eq!(Store::from_bytes(b"something else\n"), None);
        // A damaged line costs only itself.
        let mut damaged = bytes.clone();
        damaged.extend_from_slice(b"junk line\nok\t3\tblue\t\t\n");
        let d = Store::from_bytes(&damaged).unwrap();
        assert_eq!(d.len(), 4);
        assert_eq!(d.get(b"ok").unwrap().color, Color::Blue);
    }

    #[test]
    fn follows_moves() {
        let mut s = Store::new();
        s.tag(b"photos/2026/a.jpg", b"trip");
        s.tag(b"photos/2026/b.jpg", b"trip");
        s.tag(b"photos/2026b.jpg", b"other");
        s.rate(b"photos", 3);
        s.moved(b"photos/2026", b"archive/2026");
        assert!(s.get(b"archive/2026/a.jpg").is_some() && s.get(b"archive/2026/b.jpg").is_some());
        assert!(s.get(b"photos/2026/a.jpg").is_none());
        assert!(s.get(b"photos/2026b.jpg").is_some(), "sibling with a shared prefix stays");
        s.moved(b"archive/2026/a.jpg", b"archive/2026/renamed.jpg");
        assert!(s.get(b"archive/2026/renamed.jpg").unwrap().has_tag(b"trip"));
        s.copied(b"archive", b"backup");
        assert!(s.get(b"backup/2026/b.jpg").is_some() && s.get(b"archive/2026/b.jpg").is_some());
        s.removed(b"archive");
        assert!(s.get(b"archive/2026/b.jpg").is_none());
        assert_eq!(s.all_tags()[&b"trip".to_vec()], 2);
        assert_eq!(Color::parse(b"Grey"), Some(Color::Gray));
    }
}
