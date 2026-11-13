// SPDX-License-Identifier: GPL-3.0-only
//! `localStorage`: one string map per origin, kept by the browser in Files so it
//! lasts across runs. Pages reach it through op `STORAGE`; each page only sees its own
//! origin's map. The browser saves it with [`WebStorage::save`] and reads it back with
//! [`WebStorage::load`].

use alloc::collections::BTreeMap;
use alloc::string::String;

/// Bytes (keys plus values, as UTF-8) one origin may keep, as browsers allow.
pub const QUOTA: usize = 5 << 20;
/// Bytes of IndexedDB one origin may keep.
pub const IDB_QUOTA: usize = 64 << 20;
/// IndexedDB databases are kept as areas named this prefix and the origin.
pub const IDB_PREFIX: &str = "idb ";
/// Origins kept; the least recently changed goes first beyond this.
const MAX_ORIGINS: usize = 512;

#[derive(Default, Debug, Clone)]
pub struct Area {
    pub items: BTreeMap<String, String>,
    bytes: usize,
    /// When it last changed, by [`WebStorage::changes`]; picks what goes first.
    touched: u64,
}

impl Area {
    pub fn bytes(&self) -> usize { self.bytes }
}

#[derive(Default, Debug, Clone)]
pub struct WebStorage {
    areas: BTreeMap<String, Area>,
    /// Grows with every change: the browser saves when it moved.
    pub changes: u64,
}

/// Escapes for the saved form: one record per line, fields split by tabs.
fn escape(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' { out.push(c); continue; }
        match it.next() { Some('t') => out.push('\t'), Some('n') => out.push('\n'), Some('r') => out.push('\r'), Some(o) => out.push(o), None => {} }
    }
    out
}

impl WebStorage {
    pub fn area(&self, origin: &str) -> Option<&Area> { self.areas.get(origin) }
    pub fn origins(&self) -> impl Iterator<Item = &String> { self.areas.keys() }

    pub fn get(&self, origin: &str, key: &str) -> Option<&str> { self.areas.get(origin)?.items.get(key).map(String::as_str) }

    /// False when it would take the origin over its quota (nothing changes then).
    pub fn set(&mut self, origin: &str, key: &str, value: &str) -> bool { self.set_within(origin, key, value, QUOTA) }

    /// Like [`set`](Self::set), with the area's own quota in bytes.
    pub fn set_within(&mut self, origin: &str, key: &str, value: &str, quota: usize) -> bool {
        let changes = self.changes + 1;
        let area = self.areas.entry(String::from(origin)).or_default();
        let old = area.items.get(key).map_or(0, |v| key.len() + v.len());
        let bytes = area.bytes - old + key.len() + value.len();
        if bytes > quota { return false; }
        if area.items.get(key).is_some_and(|v| v == value) { return true; }
        area.items.insert(String::from(key), String::from(value));
        area.bytes = bytes;
        area.touched = changes;
        self.changes = changes;
        if self.areas.len() > MAX_ORIGINS {
            let oldest = self.areas.iter().filter(|(o, _)| *o != origin).min_by_key(|(_, a)| a.touched).map(|(o, _)| o.clone());
            if let Some(o) = oldest { self.areas.remove(&o); }
        }
        true
    }

    pub fn remove(&mut self, origin: &str, key: &str) {
        let Some(area) = self.areas.get_mut(origin) else { return };
        if let Some(v) = area.items.remove(key) {
            area.bytes -= key.len() + v.len();
            self.changes += 1;
            area.touched = self.changes;
        }
        if area.items.is_empty() { self.areas.remove(origin); }
    }

    /// Forgets everything `origin` kept (`clear()`, or clearing a site's data).
    pub fn clear(&mut self, origin: &str) {
        if self.areas.remove(origin).is_some() { self.changes += 1; }
    }

    pub fn len(&self, origin: &str) -> usize { self.areas.get(origin).map_or(0, |a| a.items.len()) }

    pub fn key(&self, origin: &str, i: usize) -> Option<&str> { self.areas.get(origin)?.items.keys().nth(i).map(String::as_str) }

    /// The saved form: `origin\tkey\tvalue` lines, escaped.
    pub fn save(&self) -> String {
        let mut out = String::new();
        for (origin, area) in &self.areas {
            for (k, v) in &area.items {
                escape(origin, &mut out);
                out.push('\t');
                escape(k, &mut out);
                out.push('\t');
                escape(v, &mut out);
                out.push('\n');
            }
        }
        out
    }

    /// Reads back what [`save`](Self::save) wrote; lines that don't fit are skipped.
    pub fn load(text: &str) -> WebStorage {
        let mut s = WebStorage::default();
        for line in text.lines() {
            let mut f = line.split('\t');
            let (Some(o), Some(k), Some(v), None) = (f.next(), f.next(), f.next(), f.next()) else { continue };
            s.set_within(&unescape(o), &unescape(k), &unescape(v), usize::MAX);
        }
        s.changes = 0;
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_each_origin_apart_and_round_trips() {
        let mut s = WebStorage::default();
        assert!(s.set("https://a.example", "n", "1"));
        assert!(s.set("https://a.example", "tab\tkey", "line\nbreak\\"));
        assert!(s.set("https://b.example", "n", "2"));
        assert_eq!(s.get("https://a.example", "n"), Some("1"));
        assert_eq!(s.get("https://b.example", "n"), Some("2"));
        let back = WebStorage::load(&s.save());
        assert_eq!(back.get("https://a.example", "tab\tkey"), Some("line\nbreak\\"));
        assert_eq!(back.len("https://a.example"), 2);
        let mut back = back;
        back.clear("https://a.example");
        assert_eq!(back.len("https://a.example"), 0);
        assert_eq!(back.get("https://b.example", "n"), Some("2"));
    }
    #[test]
    fn refuses_past_the_quota() {
        let mut s = WebStorage::default();
        let big = "x".repeat(QUOTA - 1);
        assert!(s.set("o", "k", &big));
        assert!(!s.set("o", "k2", "yy"));
        assert!(s.set("o", "k", "small"));
        assert!(s.set("o", "k2", "yy"));
    }
}
