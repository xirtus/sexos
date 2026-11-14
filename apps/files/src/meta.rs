// SPDX-License-Identifier: GPL-3.0-only
//! Ratings and tags in Files: Ctrl+1..Ctrl+5 rate the selected item, Ctrl+0 clears its
//! rating, Ctrl+Shift+G sets its tags. They are kept in `.loom-meta` at the top of the
//! folder Files was given, by path relative to it (so a stick keeps its own when it is
//! read on another computer), and they follow the item when it is renamed, moved,
//! trashed or restored, and go when it is deleted for good.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use loom_core::tags::Store;
use sex_app::files;
use sex_app::Window;

use crate::{at, join, shown, App, Undo};

/// The file the records are kept in, at the top of what Files was given.
pub const FILE: &[u8] = b".loom-meta";

pub struct Meta { pub store: Store, loaded: bool }
impl Meta { pub fn new() -> Self { Meta { store: Store::new(), loaded: false } } }

/// `abs` relative to `root` (no leading `/`), if it is under it.
pub fn key(root: &[u8], abs: &[u8]) -> Option<Vec<u8>> {
    if root == b"/" { return abs.strip_prefix(b"/").filter(|r| !r.is_empty()).map(|r| r.to_vec()); }
    let rest = abs.strip_prefix(root)?;
    rest.strip_prefix(b"/").filter(|r| !r.is_empty()).map(|r| r.to_vec())
}

impl App {
    fn meta_file(&self) -> Vec<u8> { at(&join(&self.root, FILE)) }
    /// Reads the records once, at start.
    pub(crate) fn meta_load(&mut self, window: &mut Window) {
        if self.loom.meta.loaded { return; }
        self.loom.meta.loaded = true;
        let path = self.meta_file();
        let Ok(st) = files::stat(window, &path) else { sex_pdx::serial_record!("[files.app.meta.loaded] root={} records=0", shown(&self.root)); return };
        let mut raw = vec![0u8; st.size as usize];
        let n = files::read(window, &path, &mut raw).unwrap_or(0);
        raw.truncate(n);
        if let Some(store) = Store::from_bytes(&raw) { self.loom.meta.store = store; }
        sex_pdx::serial_record!("[files.app.meta.loaded] root={} records={}", shown(&self.root), self.loom.meta.store.len());
        for (p, m) in self.loom.meta.store.iter().take(20) {
            let tags: Vec<&str> = m.tags.iter().map(|t| shown(t)).collect();
            sex_pdx::serial_record!("[files.app.meta.record] path={} rating={} tags={}", shown(p), m.rating, tags.join(","));
        }
    }
    fn meta_save(&mut self, window: &mut Window) {
        if !self.loom.meta.store.changed() { return; }
        let bytes = self.loom.meta.store.to_bytes();
        let path = self.meta_file();
        match files::write(window, &path, &bytes) {
            Ok(()) => { self.loom.meta.store.saved(); let _ = files::sync(window); sex_pdx::serial_record!("[files.app.meta.saved] records={}", self.loom.meta.store.len()); }
            Err(e) => sex_pdx::serial_record!("[files.app.meta.save.failed] error={}", e),
        }
    }
    /// Ctrl+0..Ctrl+5 on the selected item.
    pub(crate) fn rate(&mut self, window: &mut Window, stars: u8) {
        let Some(path) = self.selected() else { return };
        let Some(k) = key(&self.root, &path) else { return };
        self.loom.meta.store.rate(&k, stars);
        sex_pdx::serial_record!("[files.app.meta.rated] path={} stars={}", shown(&k), stars);
        self.meta_save(window);
        self.status = if stars == 0 { format!("{}: no rating.", shown(crate::base(&path))) } else { format!("{}: rated {} of 5.", shown(crate::base(&path)), stars) };
    }
    /// The tags of the selected item, as the prompt starts with them.
    pub(crate) fn tags_text(&self, path: &[u8]) -> Vec<u8> {
        let Some(k) = key(&self.root, path) else { return Vec::new() };
        let Some(m) = self.loom.meta.store.get(&k) else { return Vec::new() };
        let mut out = Vec::new();
        for (i, t) in m.tags.iter().enumerate() { if i > 0 { out.extend_from_slice(b", "); } out.extend_from_slice(t); }
        out
    }
    /// The tags prompt was confirmed: the item has exactly these tags (comma separated).
    pub(crate) fn set_tags(&mut self, window: &mut Window, path: &[u8], text: &[u8]) {
        let Some(k) = key(&self.root, path) else { return };
        let wanted: Vec<Vec<u8>> = text.split(|b| *b == b',').map(|t| t.trim_ascii().to_vec()).filter(|t| !t.is_empty()).collect();
        let old: Vec<Vec<u8>> = self.loom.meta.store.get(&k).map(|m| m.tags.iter().cloned().collect()).unwrap_or_default();
        for t in &old { if !wanted.iter().any(|w| w.eq_ignore_ascii_case(t)) { self.loom.meta.store.untag(&k, t); } }
        for t in &wanted { self.loom.meta.store.tag(&k, t); }
        let list: Vec<&str> = wanted.iter().map(|t| shown(t)).collect();
        sex_pdx::serial_record!("[files.app.meta.tagged] path={} tags={}", shown(&k), list.join(","));
        self.meta_save(window);
        self.status = if wanted.is_empty() { format!("{}: no tags.", shown(crate::base(path))) } else { format!("{}: tagged {}.", shown(crate::base(path)), list.join(", ")) };
    }
    /// After a rename or move from `from` to `to`.
    pub(crate) fn meta_moved(&mut self, window: &mut Window, from: &[u8], to: &[u8]) {
        let (Some(f), Some(t)) = (key(&self.root, from), key(&self.root, to)) else { return };
        self.loom.meta.store.moved(&f, &t);
        self.meta_save(window);
    }
    /// After `path` was deleted for good.
    pub(crate) fn meta_removed(&mut self, window: &mut Window, path: &[u8]) {
        let Some(k) = key(&self.root, path) else { return };
        self.loom.meta.store.removed(&k);
        self.meta_save(window);
    }
    /// After Ctrl+Z took `step` back.
    pub(crate) fn meta_undone(&mut self, window: &mut Window, step: &Undo) {
        match step {
            Undo::Moved { from, to } | Undo::Trashed { from, to, .. } => self.meta_moved(window, to, from),
            Undo::Copied { to } => self.meta_removed(window, to),
            Undo::Renamed { dir, moves } => {
                for (f, t) in moves.iter() { self.meta_moved(window, &join(dir, f), &join(dir, t)); }
            }
            Undo::Made { .. } => {}
        }
    }
    /// What the row of `name` in the current folder shows after it: stars and tags.
    pub(crate) fn meta_badge(&self, name: &[u8]) -> String {
        let mut s = String::new();
        let Some(k) = key(&self.root, &join(&self.cwd, name)) else { return s };
        let Some(m) = self.loom.meta.store.get(&k) else { return s };
        if m.rating > 0 { s.push_str("   "); for _ in 0..m.rating { s.push('*'); } }
        if !m.tags.is_empty() {
            s.push_str("   ");
            for (i, t) in m.tags.iter().enumerate() { if i > 0 { s.push(' '); } s.push('#'); s.push_str(shown(t)); }
        }
        s
    }
}
