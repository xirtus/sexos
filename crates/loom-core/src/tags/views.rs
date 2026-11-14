// SPDX-License-Identifier: GPL-3.0-only
//! Per-folder view memory: each folder opens sorted and shown the way it was left. A view
//! marked `subfolders` also applies below its folder unless a subfolder has its own.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::text::{escape, lines, split_unescaped, unescape};
use crate::{is_within, parent, rebase};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortKey { #[default] Name, Size, Modified, Type }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Mode { #[default] List, Details, Icons, Columns }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct View { pub sort: SortKey, pub descending: bool, pub mode: Mode, pub hidden: bool, pub folders_first: bool, pub subfolders: bool }

impl Default for View {
    fn default() -> Self { View { sort: SortKey::Name, descending: false, mode: Mode::List, hidden: false, folders_first: true, subfolders: false } }
}

const SORTS: [(SortKey, &[u8]); 4] = [(SortKey::Name, b"name"), (SortKey::Size, b"size"), (SortKey::Modified, b"modified"), (SortKey::Type, b"type")];
const MODES: [(Mode, &[u8]); 4] = [(Mode::List, b"list"), (Mode::Details, b"details"), (Mode::Icons, b"icons"), (Mode::Columns, b"columns")];

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Views { map: BTreeMap<Vec<u8>, View>, pub default: View }

const HEADER: &[u8] = b"loom-views 1";

impl Views {
    /// How `folder` should open.
    pub fn get(&self, folder: &[u8]) -> View {
        if let Some(v) = self.map.get(folder) { return *v; }
        let mut at = folder;
        while !at.is_empty() {
            at = parent(at);
            if let Some(v) = self.map.get(at).filter(|v| v.subfolders) { return *v; }
        }
        self.default
    }
    /// Remembers `view` for `folder`; the default view is not stored.
    pub fn set(&mut self, folder: &[u8], view: View) {
        if view == self.default && !self.map.keys().any(|k| k != folder && is_within(folder, k)) { self.map.remove(folder); }
        else { self.map.insert(folder.to_vec(), view); }
    }
    pub fn forget(&mut self, folder: &[u8]) { self.map.remove(folder); }
    pub fn len(&self) -> usize { self.map.len() }
    pub fn is_empty(&self) -> bool { self.map.is_empty() }
    pub fn moved(&mut self, from: &[u8], to: &[u8]) {
        let keys: Vec<Vec<u8>> = self.map.keys().filter(|k| is_within(k, from)).cloned().collect();
        for k in keys { if let Some(v) = self.map.remove(&k) { self.map.insert(rebase(&k, from, to), v); } }
    }
    pub fn removed(&mut self, folder: &[u8]) { self.map.retain(|k, _| !is_within(k, folder)); }

    fn line(out: &mut Vec<u8>, path: &[u8], v: &View) {
        escape(path, out);
        out.push(b'\t');
        out.extend_from_slice(SORTS.iter().find(|s| s.0 == v.sort).map_or(&b"name"[..], |s| s.1));
        out.extend_from_slice(if v.descending { b"\tdesc\t" } else { b"\tasc\t" });
        out.extend_from_slice(MODES.iter().find(|s| s.0 == v.mode).map_or(&b"list"[..], |s| s.1));
        out.push(b'\t');
        if v.hidden { out.push(b'h'); }
        if v.folders_first { out.push(b'f'); }
        if v.subfolders { out.push(b's'); }
        out.push(b'\n');
    }

    /// `path<TAB>sort<TAB>asc|desc<TAB>mode<TAB>flags`; the first record (path `*`) is the default.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = HEADER.to_vec();
        out.push(b'\n');
        Views::line(&mut out, b"*", &self.default);
        for (p, v) in &self.map { Views::line(&mut out, p, v); }
        out
    }

    pub fn from_bytes(raw: &[u8]) -> Option<Views> {
        let mut it = lines(raw);
        if it.next()? != HEADER { return None; }
        let mut out = Views::default();
        for l in it {
            let f = split_unescaped(l, b'\t');
            if f.len() != 5 { continue; }
            let Some(sort) = SORTS.iter().find(|s| s.1 == f[1]).map(|s| s.0) else { continue };
            let Some(mode) = MODES.iter().find(|s| s.1 == f[3]).map(|s| s.0) else { continue };
            let v = View { sort, descending: f[2] == b"desc", mode, hidden: f[4].contains(&b'h'), folders_first: f[4].contains(&b'f'), subfolders: f[4].contains(&b's') };
            if f[0] == b"*" { out.default = v; } else { out.map.insert(unescape(f[0]), v); }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_and_inherits() {
        let mut v = Views::default();
        let photos = View { sort: SortKey::Modified, descending: true, mode: Mode::Icons, subfolders: true, ..View::default() };
        let code = View { sort: SortKey::Type, hidden: true, ..View::default() };
        v.set(b"home/photos", photos);
        v.set(b"home/code", code);
        assert_eq!(v.get(b"home/photos"), photos);
        assert_eq!(v.get(b"home/photos/2026/may"), photos, "inherited from a subfolders view");
        assert_eq!(v.get(b"home/code/sexos"), View::default(), "not inherited");
        assert_eq!(v.get(b"home"), View::default());
        let mine = View { mode: Mode::Details, ..View::default() };
        v.set(b"home/photos/2026", mine);
        assert_eq!(v.get(b"home/photos/2026/may"), photos, "own view of 2026 doesn't apply below it");
        assert_eq!(v.get(b"home/photos/2026"), mine);
        // Setting the default view under an inheriting folder must still be stored.
        v.set(b"home/photos/2025", View::default());
        assert_eq!(v.get(b"home/photos/2025"), View::default());
        v.set(b"elsewhere", View::default());
        assert_eq!(v.len(), 4);
        v.default.hidden = true;
        let back = Views::from_bytes(&v.to_bytes()).unwrap();
        assert_eq!(back, v);
        v.moved(b"home/photos", b"pics");
        assert_eq!(v.get(b"pics/2026"), mine);
        assert_eq!(v.get(b"pics/x"), photos);
        v.removed(b"pics");
        assert_eq!(v.len(), 1);
    }
}
