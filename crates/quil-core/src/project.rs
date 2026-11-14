// SPDX-License-Identifier: GPL-3.0-only
//! The project folder: walking it, the file tree beside the editor, and search and
//! replace across every file in it. The folder is reached through [`Fs`], so the same
//! code runs over Files on SexOS and over memory in the host tests.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry { pub name: Vec<u8>, pub dir: bool, pub size: u64 }

pub trait Fs {
    fn list(&mut self, path: &[u8]) -> Option<Vec<Entry>>;
    fn read(&mut self, path: &[u8]) -> Option<Vec<u8>>;
    fn write(&mut self, path: &[u8], data: &[u8]) -> bool;
}

/// Folders a walk never enters (build output and version-control stores).
pub const SKIP: &[&[u8]] = &[b".git", b"target", b"node_modules", b".hg", b".svn", b"__pycache__", b".cache"];
/// Files bigger than this are left out of project search.
pub const SEARCH_MAX: u64 = 8 << 20;

pub fn join(dir: &[u8], name: &[u8]) -> Vec<u8> {
    if name.is_empty() { return dir.to_vec(); }
    if dir.is_empty() { return name.to_vec(); }
    let mut p = dir.to_vec();
    if !p.is_empty() && !p.ends_with(b"/") { p.push(b'/'); }
    p.extend_from_slice(name);
    p
}

/// Every file under `root`, as paths relative to it, sorted; at most `limit`.
pub fn walk(fs: &mut dyn Fs, root: &[u8], limit: usize) -> Vec<(Vec<u8>, u64)> {
    let mut out = Vec::new();
    let mut stack: Vec<Vec<u8>> = alloc::vec![Vec::new()];
    while let Some(rel) = stack.pop() {
        let Some(mut entries) = fs.list(&join(root, &rel)) else { continue };
        entries.sort_by(|a, b| b.name.cmp(&a.name));
        for e in entries {
            if e.name.first() == Some(&b'.') && e.name != b".github" && e.dir { continue; }
            let p = if rel.is_empty() { e.name.clone() } else { join(&rel, &e.name) };
            if e.dir { if !SKIP.contains(&&e.name[..]) { stack.push(p); } } else {
                out.push((p, e.size));
                if out.len() >= limit { out.sort(); return out; }
            }
        }
    }
    out.sort();
    out
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query { pub needle: Vec<u8>, pub fold_case: bool, pub whole_word: bool }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit { pub path: Vec<u8>, pub offset: usize, pub line: usize, pub col: usize, pub preview: Vec<u8> }

fn word_byte(b: u8) -> bool { b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80 }

/// Offsets of every match of `q` in `text`.
pub fn matches(text: &[u8], q: &Query) -> Vec<usize> {
    let mut out = Vec::new();
    if q.needle.is_empty() { return out; }
    let lowered;
    let (hay, needle): (&[u8], Vec<u8>) = if q.fold_case {
        lowered = text.to_ascii_lowercase();
        (&lowered, q.needle.to_ascii_lowercase())
    } else { (text, q.needle.clone()) };
    for at in memchr::memmem::find_iter(hay, &needle) {
        if q.whole_word {
            let before = at > 0 && word_byte(hay[at - 1]);
            let after = hay.get(at + needle.len()).is_some_and(|b| word_byte(*b));
            if before || after { continue; }
        }
        if out.last().is_some_and(|l: &usize| at < l + needle.len()) { continue; }
        out.push(at);
    }
    out
}

/// Searches every file under `root`; files that aren't text (a NUL byte in the first
/// 8 KiB) are skipped. Returns the hits (at most `limit`) and how many files were read.
pub fn search(fs: &mut dyn Fs, root: &[u8], q: &Query, limit: usize) -> (Vec<Hit>, usize) {
    let files = walk(fs, root, 200_000);
    let mut hits = Vec::new();
    let mut read = 0;
    for (path, size) in files {
        if size > SEARCH_MAX { continue; }
        let Some(text) = fs.read(&join(root, &path)) else { continue };
        read += 1;
        if memchr::memchr(0, &text[..text.len().min(8192)]).is_some() { continue; }
        let found = matches(&text, q);
        if found.is_empty() { continue; }
        // Line numbers by counting newlines between hits, once through the file.
        let (mut line, mut counted) = (0usize, 0usize);
        for at in found {
            line += memchr::memchr_iter(b'\n', &text[counted..at]).count();
            counted = at;
            let ls = memchr::memrchr(b'\n', &text[..at]).map_or(0, |i| i + 1);
            let le = memchr::memchr(b'\n', &text[at..]).map_or(text.len(), |i| at + i);
            let col = text[ls..at].iter().filter(|c| **c & 0xC0 != 0x80).count();
            let start = ls.max(at.saturating_sub(60));
            hits.push(Hit { path: path.clone(), offset: at, line, col, preview: text[start..le.min(at + 120)].to_vec() });
            if hits.len() >= limit { return (hits, read); }
        }
    }
    (hits, read)
}

/// Replaces every match in every file that has one; returns (files changed, replacements).
pub fn replace_all(fs: &mut dyn Fs, root: &[u8], q: &Query, with: &[u8]) -> (usize, usize) {
    let mut files = 0;
    let mut total = 0;
    for (path, size) in walk(fs, root, 200_000) {
        if size > SEARCH_MAX { continue; }
        let full = join(root, &path);
        let Some(text) = fs.read(&full) else { continue };
        if memchr::memchr(0, &text[..text.len().min(8192)]).is_some() { continue; }
        let found = matches(&text, q);
        if found.is_empty() { continue; }
        let mut out = Vec::with_capacity(text.len());
        let mut last = 0;
        for at in &found { out.extend_from_slice(&text[last..*at]); out.extend_from_slice(with); last = at + q.needle.len(); }
        out.extend_from_slice(&text[last..]);
        if fs.write(&full, &out) { files += 1; total += found.len(); }
    }
    (files, total)
}

/// The file tree: which folders are open, and the rows that makes.
#[derive(Default)]
pub struct Tree {
    pub root: Vec<u8>,
    open: BTreeSet<Vec<u8>>,
    pub rows: Vec<Row>,
    pub selected: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row { pub path: Vec<u8>, pub name: Vec<u8>, pub dir: bool, pub depth: usize, pub open: bool }

impl Tree {
    pub fn new(root: &[u8]) -> Tree { Tree { root: root.to_vec(), ..Tree::default() } }
    pub fn refresh(&mut self, fs: &mut dyn Fs) {
        self.rows.clear();
        let root = self.root.clone();
        self.add(fs, &root, b"", 0);
        self.selected = self.selected.min(self.rows.len().saturating_sub(1));
    }
    fn add(&mut self, fs: &mut dyn Fs, root: &[u8], rel: &[u8], depth: usize) {
        let Some(mut entries) = fs.list(&join(root, rel)) else { return };
        // Folders first, then names, case-insensitively.
        entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase())));
        for e in entries {
            if e.name == b".git" { continue; }
            let p = if rel.is_empty() { e.name.clone() } else { join(rel, &e.name) };
            let open = e.dir && self.open.contains(&p);
            self.rows.push(Row { path: p.clone(), name: e.name, dir: e.dir, depth, open });
            if open { self.add(fs, root, &p, depth + 1); }
        }
    }
    /// Opens or closes the folder at the selected row; returns the file's path when the
    /// row is a file.
    pub fn activate(&mut self, fs: &mut dyn Fs) -> Option<Vec<u8>> {
        let row = self.rows.get(self.selected)?.clone();
        if !row.dir { return Some(row.path); }
        if !self.open.remove(&row.path) { self.open.insert(row.path); }
        self.refresh(fs);
        None
    }
    pub fn up(&mut self) { self.selected = self.selected.saturating_sub(1); }
    pub fn down(&mut self) { if self.selected + 1 < self.rows.len() { self.selected += 1; } }
    /// Opens every folder on the way to `path` and selects it.
    pub fn reveal(&mut self, fs: &mut dyn Fs, path: &[u8]) {
        let mut at = 0;
        while let Some(i) = path[at..].iter().position(|b| *b == b'/') { self.open.insert(path[..at + i].to_vec()); at += i + 1; }
        self.refresh(fs);
        if let Some(i) = self.rows.iter().position(|r| r.path == path) { self.selected = i; }
    }
}
