// SPDX-License-Identifier: GPL-3.0-only
//! Open files as tabs. Each tab owns its editor, its language and where its view is.
//! (The tab bar is drawn by the program; when the Weft toolkit's tab widget lands it
//! draws this same list.)

use crate::editor::Editor;
use crate::highlight::Lang;
use alloc::vec::Vec;

pub struct Tab {
    pub ed: Editor,
    /// Empty for a new file never saved.
    pub path: Vec<u8>,
    pub lang: Lang,
    /// Byte offset of the first line shown, and the first column.
    pub top: usize,
    pub left: usize,
}

impl Tab {
    pub fn new(path: &[u8], ed: Editor) -> Tab { Tab { lang: Lang::from_path(path), ed, path: path.to_vec(), top: 0, left: 0 } }
    pub fn name(&self) -> &[u8] {
        if self.path.is_empty() { return b"untitled"; }
        self.path.rsplit(|b| *b == b'/').next().unwrap_or(&self.path)
    }
    /// Keeps the primary cursor within `rows` lines of the view's top.
    pub fn follow(&mut self, rows: usize) {
        let b = &self.ed.buf;
        let c = self.ed.cursor();
        let cl = b.line_start(c);
        if cl < self.top { self.top = cl; return; }
        // Count lines from top to the cursor, stopping once past the view.
        let mut l = self.top;
        let mut n = 0;
        while l < cl && n < rows { l = b.next_line(l); n += 1; }
        if n >= rows && l <= cl {
            // Put the cursor's line on the last row.
            let mut t = cl;
            for _ in 1..rows { match b.prev_line(t) { Some(p) => t = p, None => break } }
            self.top = t;
        }
    }
    pub fn scroll(&mut self, lines: isize) {
        let b = &self.ed.buf;
        if lines > 0 { for _ in 0..lines { let n = b.next_line(self.top); if n >= b.len() && b.line_end(self.top) >= b.len() { break; } self.top = n; } }
        else { for _ in 0..-lines { match b.prev_line(self.top) { Some(p) => self.top = p, None => break } } }
    }
}

#[derive(Default)]
pub struct Tabs { pub list: Vec<Tab>, pub active: usize }

impl Tabs {
    pub fn is_empty(&self) -> bool { self.list.is_empty() }
    pub fn len(&self) -> usize { self.list.len() }
    pub fn current(&mut self) -> Option<&mut Tab> { self.list.get_mut(self.active) }
    pub fn find(&self, path: &[u8]) -> Option<usize> { if path.is_empty() { None } else { self.list.iter().position(|t| t.path == path) } }
    /// Adds a tab after the active one and switches to it.
    pub fn add(&mut self, tab: Tab) -> usize {
        let at = if self.list.is_empty() { 0 } else { self.active + 1 };
        self.list.insert(at, tab);
        self.active = at;
        at
    }
    /// Closes tab `i`; the tab to its left (or right) becomes active.
    pub fn close(&mut self, i: usize) -> Option<Tab> {
        if i >= self.list.len() { return None; }
        let t = self.list.remove(i);
        if self.active > i || self.active >= self.list.len() { self.active = self.active.saturating_sub(1); }
        Some(t)
    }
    pub fn next(&mut self) { if !self.list.is_empty() { self.active = (self.active + 1) % self.list.len(); } }
    pub fn prev(&mut self) { if !self.list.is_empty() { self.active = (self.active + self.list.len() - 1) % self.list.len(); } }
    /// Moves the active tab one place left or right.
    pub fn shift(&mut self, right: bool) {
        let n = self.list.len();
        if n < 2 { return; }
        let to = if right { (self.active + 1).min(n - 1) } else { self.active.saturating_sub(1) };
        self.list.swap(self.active, to);
        self.active = to;
    }
}
