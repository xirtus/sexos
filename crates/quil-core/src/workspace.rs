// SPDX-License-Identifier: GPL-3.0-only
//! Open files as tabs. Each tab owns its editor, its language and where its view is.
//! (The tab bar is drawn by the program; when the Weft toolkit's tab widget lands it
//! draws this same list.)

use crate::editor::Editor;
use crate::highlight::{self, Lang, State};
use alloc::vec::Vec;

pub struct Tab {
    pub ed: Editor,
    /// Empty for a new file never saved.
    pub path: Vec<u8>,
    /// Stable recovery slot for an untitled tab; absent for named files.
    pub untitled_slot: Option<u8>,
    pub lang: Lang,
    /// Byte offset of the first line shown, and the first column.
    pub top: usize,
    pub left: usize,
    syntax_version: u64,
    syntax_checkpoints: Vec<(usize, State)>,
    pub syntax_pending: bool,
}

/// State at the first visible line. A distant jump is scanned over idle draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxAt { Ready(State), Pending, Unavailable }

pub const SYNTAX_LINE_MAX: usize = 1 << 20;

impl Tab {
    pub fn new(path: &[u8], ed: Editor) -> Tab { Tab { lang: Lang::from_path(path), ed, path: path.to_vec(), untitled_slot: None, top: 0, left: 0, syntax_version: 0, syntax_checkpoints: alloc::vec![(0, State::Normal)], syntax_pending: false } }
    /// Find a line boundary without walking an arbitrarily large lazy file line.
    pub fn syntax_line_end(&self, at: usize) -> Option<usize> {
        let mut pos = at;
        let mut bytes = [0u8; 4096];
        while pos - at <= SYNTAX_LINE_MAX {
            let want = bytes.len().min(SYNTAX_LINE_MAX + 1 - (pos - at));
            let n = self.ed.buf.read(pos, &mut bytes[..want]);
            if let Some(i) = memchr::memchr(b'\n', &bytes[..n]) { return Some(pos + i); }
            pos += n;
            if n == 0 { return Some(pos); }
        }
        None
    }
    fn remember_syntax(&mut self, at: usize, state: State) {
        if let Some(slot) = self.syntax_checkpoints.iter_mut().find(|(offset, _)| *offset == at) {
            slot.1 = state;
        } else {
            self.syntax_checkpoints.push((at, state));
            if self.syntax_checkpoints.len() > 128 { self.syntax_checkpoints.remove(1); }
        }
    }
    /// Scan at most 64 KiB of ordinary lines per call. A far jump shows plain
    /// text with a visible indexing label until the exact state is known.
    pub fn syntax_state_at_top(&mut self) -> SyntaxAt {
        if self.syntax_version != self.ed.version() {
            self.syntax_version = self.ed.version();
            self.syntax_checkpoints.clear();
            self.syntax_checkpoints.push((0, State::Normal));
        }
        let (mut at, mut state) = self.syntax_checkpoints.iter().copied()
            .filter(|(offset, _)| *offset <= self.top).max_by_key(|(offset, _)| *offset)
            .unwrap_or((0, State::Normal));
        let mut scanned = 0;
        let mut spans = Vec::new();
        while at < self.top {
            let Some(end) = self.syntax_line_end(at) else { self.syntax_pending = false; return SyntaxAt::Unavailable; };
            if scanned > 0 && scanned + end - at > 64 * 1024 { break; }
            let bytes = self.ed.buf.slice(at..end);
            state = highlight::line_state(self.lang, &bytes, state, &mut spans);
            let next = if end < self.ed.buf.len() { end + 1 } else { end };
            scanned += next - at;
            at = next;
        }
        self.remember_syntax(at, state);
        self.syntax_pending = at < self.top;
        if self.syntax_pending { SyntaxAt::Pending } else { SyntaxAt::Ready(state) }
    }
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
