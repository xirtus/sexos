// SPDX-License-Identifier: GPL-3.0-only
//! Text editing for SexOS programs, apart from any window: a document buffer whose every
//! change goes through an undo tree, find and replace, and syntax colouring for the
//! languages people edit most (Rust, C, JavaScript, Markdown, TOML). Jot draws it; the
//! host tests prove it.
#![no_std]
extern crate alloc;

pub mod find;
pub mod highlight;
pub mod undo;

use alloc::vec::Vec;
use undo::{Edit, UndoTree};

/// A document: UTF-8 bytes, a cursor and an optional selection anchor, all byte offsets
/// on character boundaries.
pub struct Doc {
    text: Vec<u8>,
    pub cursor: usize,
    /// The other end of the selection, when there is one.
    pub anchor: Option<usize>,
    undo: UndoTree,
    /// The undo-tree node that matches what is on disk.
    saved_at: usize,
    /// The column Up and Down try to keep.
    want_col: Option<usize>,
}

impl Doc {
    pub fn new(text: Vec<u8>) -> Doc {
        Doc { text, cursor: 0, anchor: None, undo: UndoTree::new(), saved_at: 0, want_col: None }
    }
    pub fn text(&self) -> &[u8] { &self.text }
    pub fn len(&self) -> usize { self.text.len() }
    pub fn is_empty(&self) -> bool { self.text.is_empty() }
    /// Changed since the last [`Doc::mark_saved`] (undoing back to it counts as unchanged).
    pub fn modified(&self) -> bool { self.undo.current() != self.saved_at }
    pub fn mark_saved(&mut self) { self.saved_at = self.undo.current(); }
    pub fn undo_tree(&self) -> &UndoTree { &self.undo }

    // ---- Lines ----

    pub fn line_start(&self, at: usize) -> usize { self.text[..at.min(self.text.len())].iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1) }
    pub fn line_end(&self, at: usize) -> usize { self.text[at.min(self.text.len())..].iter().position(|b| *b == b'\n').map_or(self.text.len(), |i| at + i) }
    /// Zero-based line number of a byte offset.
    pub fn line_of(&self, at: usize) -> usize { self.text[..at.min(self.text.len())].iter().filter(|b| **b == b'\n').count() }
    pub fn line_count(&self) -> usize { self.text.iter().filter(|b| **b == b'\n').count() + 1 }
    /// Byte offset where line `n` starts (the end of the text past the last line).
    pub fn offset_of_line(&self, n: usize) -> usize {
        if n == 0 { return 0; }
        let mut seen = 0;
        for (i, b) in self.text.iter().enumerate() { if *b == b'\n' { seen += 1; if seen == n { return i + 1; } } }
        self.text.len()
    }
    /// Characters from the line start to `at`.
    pub fn column(&self, at: usize) -> usize { chars(&self.text[self.line_start(at)..at]) }
    fn at_column(&self, start: usize, col: usize) -> usize {
        let end = self.line_end(start);
        let mut at = start;
        for _ in 0..col { if at >= end { break; } at = next_char(&self.text, at); }
        at
    }

    // ---- Selection ----

    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        (a != self.cursor).then(|| (a.min(self.cursor), a.max(self.cursor)))
    }
    pub fn selected(&self) -> &[u8] { self.selection().map_or(&[][..], |(a, b)| &self.text[a..b]) }
    pub fn select_all(&mut self) { self.anchor = Some(0); self.cursor = self.text.len(); }
    pub fn select(&mut self, from: usize, to: usize) { self.anchor = Some(from.min(self.text.len())); self.cursor = to.min(self.text.len()); }

    // ---- Moving ----

    /// Moves the cursor; `extend` keeps (or starts) the selection, as Shift does.
    pub fn move_to(&mut self, motion: Motion, extend: bool) {
        if extend { self.anchor.get_or_insert(self.cursor); } else if let Some((a, b)) = self.selection() {
            // Left or Right on a selection lands on its edge, like every editor.
            self.anchor = None;
            match motion { Motion::Left => { self.cursor = a; return; } Motion::Right => { self.cursor = b; return; } _ => {} }
        } else { self.anchor = None; }
        let keep_col = matches!(motion, Motion::Up | Motion::Down | Motion::PageUp(_) | Motion::PageDown(_));
        let col = if keep_col { *self.want_col.get_or_insert(self.column(self.cursor)) } else { 0 };
        self.cursor = match motion {
            Motion::Left => prev_char(&self.text, self.cursor),
            Motion::Right => next_char(&self.text, self.cursor),
            Motion::WordLeft => word_left(&self.text, self.cursor),
            Motion::WordRight => word_right(&self.text, self.cursor),
            Motion::Home => {
                // Home goes to the first non-blank, then to the very start.
                let start = self.line_start(self.cursor);
                let indent = start + self.text[start..self.line_end(start)].iter().take_while(|b| **b == b' ' || **b == b'\t').count();
                if self.cursor == indent { start } else { indent }
            }
            Motion::End => self.line_end(self.cursor),
            Motion::Up | Motion::PageUp(_) => {
                let n = if let Motion::PageUp(n) = motion { n.max(1) } else { 1 };
                let line = self.line_of(self.cursor);
                if line == 0 { 0 } else { self.at_column(self.offset_of_line(line.saturating_sub(n)), col) }
            }
            Motion::Down | Motion::PageDown(_) => {
                let n = if let Motion::PageDown(n) = motion { n.max(1) } else { 1 };
                let line = self.line_of(self.cursor);
                if line + 1 >= self.line_count() { self.text.len() } else { self.at_column(self.offset_of_line((line + n).min(self.line_count() - 1)), col) }
            }
            Motion::Start => 0,
            Motion::End_ => self.text.len(),
            Motion::To(at) => floor_char(&self.text, at.min(self.text.len())),
        };
        if !keep_col { self.want_col = None; }
    }

    // ---- Changing ----

    /// Replaces `from..to` with `with`, as one undoable step (merged with the step before
    /// when it continues the same typing).
    pub fn replace(&mut self, from: usize, to: usize, with: &[u8]) {
        let removed = self.text[from..to].to_vec();
        self.text.splice(from..to, with.iter().copied());
        self.undo.record(Edit { at: from, removed, inserted: with.to_vec(), cursor_before: self.cursor });
        self.cursor = from + with.len();
        self.anchor = None;
        self.want_col = None;
    }
    /// Typing: replaces the selection, if any, with `s`.
    pub fn insert(&mut self, s: &[u8]) {
        let (a, b) = self.selection().unwrap_or((self.cursor, self.cursor));
        self.replace(a, b, s);
    }
    /// Enter: a newline keeping the current line's indentation.
    pub fn newline(&mut self) {
        let start = self.line_start(self.cursor);
        let indent: Vec<u8> = self.text[start..self.cursor].iter().copied().take_while(|b| *b == b' ' || *b == b'\t').collect();
        let mut s = alloc::vec![b'\n'];
        s.extend_from_slice(&indent);
        self.insert(&s);
        self.undo.seal();
    }
    pub fn backspace(&mut self) {
        if let Some((a, b)) = self.selection() { self.replace(a, b, b""); return; }
        if self.cursor == 0 { return; }
        let p = prev_char(&self.text, self.cursor);
        self.replace(p, self.cursor, b"");
    }
    pub fn delete(&mut self) {
        if let Some((a, b)) = self.selection() { self.replace(a, b, b""); return; }
        if self.cursor >= self.text.len() { return; }
        let n = next_char(&self.text, self.cursor);
        let c = self.cursor;
        self.replace(c, n, b"");
        self.cursor = c;
    }
    /// Cut: the selection removed, its bytes returned.
    pub fn cut(&mut self) -> Vec<u8> {
        let Some((a, b)) = self.selection() else { return Vec::new() };
        let out = self.text[a..b].to_vec();
        self.replace(a, b, b"");
        self.undo.seal();
        out
    }
    /// Ends the current undo step, so the next change starts a new one.
    pub fn seal(&mut self) { self.undo.seal(); }

    /// Steps back one change. False if there was none.
    pub fn undo(&mut self) -> bool {
        let Some(e) = self.undo.undo() else { return false };
        self.text.splice(e.at..e.at + e.inserted.len(), e.removed.iter().copied());
        self.cursor = e.cursor_before.min(self.text.len());
        self.anchor = None;
        true
    }
    /// Steps forward again, along the most recent branch.
    pub fn redo(&mut self) -> bool {
        let Some(e) = self.undo.redo() else { return false };
        self.text.splice(e.at..e.at + e.removed.len(), e.inserted.iter().copied());
        self.cursor = e.at + e.inserted.len();
        self.anchor = None;
        true
    }
    /// Moves to the other branch at this point of the tree (after an undo), as `g-`/`g+`.
    pub fn switch_branch(&mut self) -> bool { self.undo.switch_branch() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion { Left, Right, WordLeft, WordRight, Up, Down, Home, End, PageUp(usize), PageDown(usize), Start, End_, To(usize) }

pub fn chars(s: &[u8]) -> usize { s.iter().filter(|b| (**b & 0xC0) != 0x80).count() }
pub fn next_char(s: &[u8], at: usize) -> usize {
    if at >= s.len() { return s.len(); }
    let mut i = at + 1;
    while i < s.len() && (s[i] & 0xC0) == 0x80 { i += 1; }
    i
}
pub fn prev_char(s: &[u8], at: usize) -> usize {
    if at == 0 { return 0; }
    let mut i = at - 1;
    while i > 0 && (s[i] & 0xC0) == 0x80 { i -= 1; }
    i
}
fn floor_char(s: &[u8], mut at: usize) -> usize { while at > 0 && at < s.len() && (s[at] & 0xC0) == 0x80 { at -= 1; } at }
fn is_word(b: u8) -> bool { b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80 }
fn word_left(s: &[u8], mut at: usize) -> usize {
    while at > 0 && !is_word(s[at - 1]) { at -= 1; }
    while at > 0 && is_word(s[at - 1]) { at -= 1; }
    at
}
fn word_right(s: &[u8], mut at: usize) -> usize {
    while at < s.len() && !is_word(s[at]) { at += 1; }
    while at < s.len() && is_word(s[at]) { at += 1; }
    at
}

#[cfg(test)]
mod tests;
