// SPDX-License-Identifier: GPL-3.0-only
//! Editing with any number of selections at once. Every selection is an anchor and a
//! head (byte offsets on character boundaries); an empty one is a cursor. Each command
//! acts on all of them, and selections that come to overlap merge, as in VS Code and Zed.

use crate::buffer::{Buffer, Snapshot};
use alloc::vec;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sel {
    pub anchor: usize,
    pub head: usize,
    /// The column Up and Down try to keep.
    pub col: Option<usize>,
}
impl Sel {
    pub fn cursor(at: usize) -> Sel { Sel { anchor: at, head: at, col: None } }
    pub fn start(&self) -> usize { self.anchor.min(self.head) }
    pub fn end(&self) -> usize { self.anchor.max(self.head) }
    pub fn is_empty(&self) -> bool { self.anchor == self.head }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion { Left, Right, Up, Down, WordLeft, WordRight, Home, End, DocStart, DocEnd, PageUp(usize), PageDown(usize) }

/// Where a selection lands after its edit, relative to the inserted text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After { End, Start, At(usize), Select }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group { Typing, Deleting, Other }

struct Step { snap: Snapshot, sels: Vec<Sel>, primary: usize, version: u64 }

const UNDO_LIMIT: usize = 2000;

pub struct Editor {
    pub buf: Buffer,
    pub sels: Vec<Sel>,
    /// The selection the view follows and that single-cursor commands use.
    pub primary: usize,
    undo: Vec<Step>,
    redo: Vec<Step>,
    group: Option<Group>,
    version: u64,
    next_version: u64,
    saved: u64,
    /// Indent unit typed by Tab.
    pub indent: Vec<u8>,
    /// Inside [`Editor::begin_change`]: every edit until `end_change` undoes as one.
    in_change: bool,
    change_marked: bool,
}

pub fn is_word(c: char) -> bool { c.is_alphanumeric() || c == '_' }

impl Editor {
    pub fn new(buf: Buffer) -> Editor {
        Editor { buf, sels: vec![Sel::cursor(0)], primary: 0, undo: Vec::new(), redo: Vec::new(), group: None, version: 0, next_version: 1, saved: 0, indent: b"    ".to_vec(), in_change: false, change_marked: false }
    }
    pub fn from_bytes(text: &[u8]) -> Editor { Editor::new(Buffer::from_bytes(text)) }
    pub fn text(&self) -> Vec<u8> { self.buf.to_vec() }
    pub fn modified(&self) -> bool { self.version != self.saved }
    pub fn mark_saved(&mut self) { self.saved = self.version; }
    /// After a save that rebased the buffer on the new file, old steps point at text
    /// that is gone.
    pub fn forget_history(&mut self) { self.undo.clear(); self.redo.clear(); self.group = None; }
    pub fn cursor(&self) -> usize { self.sels[self.primary].head }
    pub fn primary_sel(&self) -> Sel { self.sels[self.primary] }
    pub fn set_cursor(&mut self, at: usize) { self.sels = vec![Sel::cursor(at.min(self.buf.len()))]; self.primary = 0; }
    pub fn set_selection(&mut self, anchor: usize, head: usize) {
        let n = self.buf.len();
        self.sels = vec![Sel { anchor: anchor.min(n), head: head.min(n), col: None }];
        self.primary = 0;
    }
    pub fn selected(&self, s: &Sel) -> Vec<u8> { self.buf.slice(s.start()..s.end()) }

    // ---- History ----

    /// Starts a compound change (a Vim `cw` and the typing after it): one undo step.
    pub fn begin_change(&mut self) { self.in_change = true; self.change_marked = false; self.group = None; }
    pub fn in_change(&self) -> bool { self.in_change }
    pub fn end_change(&mut self) { self.in_change = false; self.change_marked = false; self.group = None; }
    fn checkpoint(&mut self, g: Group) {
        if self.in_change {
            if self.change_marked { return; }
            self.change_marked = true;
        } else if self.group == Some(g) && g != Group::Other { return; }
        self.undo.push(Step { snap: self.buf.snapshot(), sels: self.sels.clone(), primary: self.primary, version: self.version });
        if self.undo.len() > UNDO_LIMIT { self.undo.remove(0); }
        self.redo.clear();
        self.group = Some(g);
    }
    /// Ends the current typing group, so the next edit undoes on its own.
    pub fn break_group(&mut self) { self.group = None; }
    pub fn undo(&mut self) -> bool { self.swap(true) }
    pub fn redo(&mut self) -> bool { self.swap(false) }
    fn swap(&mut self, undo: bool) -> bool {
        let step = match if undo { self.undo.pop() } else { self.redo.pop() } { Some(s) => s, None => return false };
        let here = Step { snap: self.buf.snapshot(), sels: self.sels.clone(), primary: self.primary, version: self.version };
        if undo { self.redo.push(here) } else { self.undo.push(here) }
        self.buf.restore(&step.snap);
        self.sels = step.sels;
        self.primary = step.primary.min(self.sels.len() - 1);
        self.version = step.version;
        self.group = None;
        true
    }

    // ---- The edit primitive ----

    /// Applies one replacement per selection (`f` returns the range to replace, the new
    /// text and where the selection goes). Ranges are applied together, so offsets from
    /// `f` all refer to the text before any of them.
    pub fn edit(&mut self, mut f: impl FnMut(&Buffer, &Sel) -> (core::ops::Range<usize>, Vec<u8>, After)) {
        self.edit_grouped(Group::Other, &mut f);
    }
    fn edit_grouped(&mut self, g: Group, f: &mut dyn FnMut(&Buffer, &Sel) -> (core::ops::Range<usize>, Vec<u8>, After)) {
        let mut plan: Vec<(usize, core::ops::Range<usize>, Vec<u8>, After)> =
            self.sels.iter().enumerate().map(|(i, s)| { let (r, t, a) = f(&self.buf, s); (i, r, t, a) }).collect();
        plan.sort_by_key(|p| (p.1.start, p.1.end));
        // Overlapping ranges (two cursors deleting the same character) collapse into one.
        let mut kept: Vec<(usize, core::ops::Range<usize>, Vec<u8>, After)> = Vec::new();
        let mut dropped: Vec<(usize, usize)> = Vec::new();
        for p in plan {
            if let Some(last) = kept.last_mut() {
                if p.1.start < last.1.end || p.1 == last.1 {
                    last.1.end = last.1.end.max(p.1.end);
                    dropped.push((p.0, last.0));
                    continue;
                }
            }
            kept.push(p);
        }
        if kept.iter().all(|p| p.1.is_empty() && p.2.is_empty()) {
            return;
        }
        self.checkpoint(g);
        self.version = self.next_version;
        self.next_version += 1;
        for p in kept.iter().rev() { self.buf.replace(p.1.clone(), &p.2); }
        let mut delta: isize = 0;
        let mut new = vec![Sel::cursor(0); self.sels.len()];
        for p in &kept {
            let start = (p.1.start as isize + delta) as usize;
            let n = p.2.len();
            new[p.0] = match p.3 {
                After::End => Sel::cursor(start + n),
                After::Start => Sel::cursor(start),
                After::At(k) => Sel::cursor(start + k.min(n)),
                After::Select => Sel { anchor: start, head: start + n, col: None },
            };
            delta += n as isize - (p.1.end - p.1.start) as isize;
        }
        for (gone, into) in dropped { new[gone] = new[into]; }
        self.sels = new;
        self.normalize();
    }

    /// Sorts selections and merges those that touch or overlap.
    pub fn normalize(&mut self) {
        let len = self.buf.len();
        for s in &mut self.sels { s.anchor = s.anchor.min(len); s.head = s.head.min(len); }
        let primary = self.sels[self.primary.min(self.sels.len() - 1)];
        let mut v = core::mem::take(&mut self.sels);
        v.sort_by_key(|s| (s.start(), s.end()));
        let mut out: Vec<Sel> = Vec::with_capacity(v.len());
        let mut primary_at = 0;
        for s in v {
            let is_primary = s == primary;
            if let Some(last) = out.last_mut() {
                if s.start() < last.end() || (s.start() == last.start() && s.end() == last.end()) {
                    let (a, b) = (last.start().min(s.start()), last.end().max(s.end()));
                    let forward = last.head >= last.anchor;
                    *last = if forward { Sel { anchor: a, head: b, col: None } } else { Sel { anchor: b, head: a, col: None } };
                    if is_primary { primary_at = out.len() - 1; }
                    continue;
                }
            }
            if is_primary { primary_at = out.len(); }
            out.push(s);
        }
        self.sels = out;
        self.primary = primary_at.min(self.sels.len() - 1);
    }

    // ---- Typing ----

    /// Replaces every selection with `text` (a cursor inserts it). A newline keeps the
    /// line's indentation.
    pub fn type_text(&mut self, text: &[u8]) {
        let newline = text == b"\n";
        self.edit_grouped(if newline { Group::Other } else { Group::Typing }, &mut |b, s| {
            let mut t = text.to_vec();
            if newline {
                let ls = b.line_start(s.start());
                let line = b.slice(ls..s.start());
                t.extend(line.iter().take_while(|c| **c == b' ' || **c == b'\t'));
            }
            (s.start()..s.end(), t, After::End)
        });
        if newline { self.break_group(); }
    }
    pub fn backspace(&mut self) {
        self.edit_grouped(Group::Deleting, &mut |b, s| {
            if !s.is_empty() { return (s.start()..s.end(), vec![], After::Start); }
            (b.prev_char(s.head)..s.head, vec![], After::Start)
        });
    }
    pub fn delete_forward(&mut self) {
        self.edit_grouped(Group::Deleting, &mut |b, s| {
            if !s.is_empty() { return (s.start()..s.end(), vec![], After::Start); }
            (s.head..b.next_char(s.head), vec![], After::Start)
        });
    }
    /// Deletes back to the start of the word (Ctrl+Backspace).
    pub fn delete_word_back(&mut self) {
        self.edit_grouped(Group::Other, &mut |b, s| {
            if !s.is_empty() { return (s.start()..s.end(), vec![], After::Start); }
            (word_left(b, s.head)..s.head, vec![], After::Start)
        });
    }

    // ---- Moving ----

    pub fn motion_target(&self, s: &Sel, m: Motion) -> (usize, Option<usize>) {
        let b = &self.buf;
        let h = s.head;
        match m {
            Motion::Left => (b.prev_char(h), None),
            Motion::Right => (b.next_char(h), None),
            Motion::Up | Motion::Down | Motion::PageUp(_) | Motion::PageDown(_) => {
                let col = s.col.unwrap_or_else(|| b.column(h));
                let n = match m { Motion::PageUp(n) | Motion::PageDown(n) => n.max(1), _ => 1 };
                let up = matches!(m, Motion::Up | Motion::PageUp(_));
                let mut line = b.line_start(h);
                for _ in 0..n {
                    if up { match b.prev_line(line) { Some(p) => line = p, None => return (0, Some(col)) } } else {
                        let e = b.line_end(line);
                        if e >= b.len() { return (b.len(), Some(col)); }
                        line = e + 1;
                    }
                }
                (b.at_column(line, col), Some(col))
            }
            Motion::WordLeft => (word_left(b, h), None),
            Motion::WordRight => (word_right(b, h), None),
            Motion::Home => {
                // Smart Home: first non-blank, then the true line start.
                let ls = b.line_start(h);
                let mut first = ls;
                while matches!(b.byte(first), Some(b' ') | Some(b'\t')) { first += 1; }
                (if h == first { ls } else { first }, None)
            }
            Motion::End => (b.line_end(h), None),
            Motion::DocStart => (0, None),
            Motion::DocEnd => (b.len(), None),
        }
    }
    pub fn move_all(&mut self, m: Motion, extend: bool) {
        self.break_group();
        let sels = self.sels.clone();
        for (i, s) in sels.iter().enumerate() {
            // Left/Right on a selection without Shift lands on its edge.
            if !extend && !s.is_empty() && matches!(m, Motion::Left | Motion::Right) {
                let at = if m == Motion::Left { s.start() } else { s.end() };
                self.sels[i] = Sel::cursor(at);
                continue;
            }
            let (to, col) = self.motion_target(s, m);
            self.sels[i] = Sel { anchor: if extend { s.anchor } else { to }, head: to, col };
        }
        self.normalize();
    }
    pub fn select_all(&mut self) { self.sels = vec![Sel { anchor: 0, head: self.buf.len(), col: None }]; self.primary = 0; }

    // ---- Multiple cursors ----

    /// Adds a cursor on the line above or below the topmost or bottommost one, at the
    /// same column (Ctrl+Shift+Up/Down; repeated, it makes a column selection).
    pub fn add_cursor(&mut self, below: bool) {
        let edge = if below { *self.sels.iter().max_by_key(|s| s.head).unwrap() } else { *self.sels.iter().min_by_key(|s| s.head).unwrap() };
        let col = edge.col.unwrap_or_else(|| self.buf.column(edge.head));
        let acol = self.buf.column(edge.anchor);
        let line = self.buf.line_start(edge.head);
        let target = if below { let e = self.buf.line_end(line); (e < self.buf.len()).then_some(e + 1) } else { self.buf.prev_line(line) };
        if let Some(t) = target {
            let head = self.buf.at_column(t, col);
            let anchor = if edge.is_empty() { head } else { self.buf.at_column(t, acol) };
            self.sels.push(Sel { anchor, head, col: Some(col) });
            self.primary = self.sels.len() - 1;
            let p = self.sels[self.primary];
            self.normalize();
            self.primary = self.sels.iter().position(|s| *s == p).unwrap_or(self.primary);
        }
    }
    /// The word around `at`, if the cursor touches one.
    pub fn word_at(&self, at: usize) -> Option<(usize, usize)> {
        let b = &self.buf;
        let touches = b.char_at(at).is_some_and(is_word) || (at > 0 && b.char_at(b.prev_char(at)).is_some_and(is_word));
        if !touches { return None; }
        let mut a = at;
        while a > 0 { let p = b.prev_char(a); if b.char_at(p).is_some_and(is_word) { a = p } else { break } }
        let mut e = at;
        while b.char_at(e).is_some_and(is_word) { e = b.next_char(e); }
        Some((a, e))
    }
    /// Ctrl+D: selects the word at the cursor, then each next occurrence of the selection.
    pub fn add_next_occurrence(&mut self) -> bool {
        let p = self.sels[self.primary];
        if p.is_empty() {
            if let Some((a, e)) = self.word_at(p.head) { self.sels[self.primary] = Sel { anchor: a, head: e, col: None }; return true; }
            return false;
        }
        let needle = self.selected(&p);
        let last = self.sels.iter().map(|s| s.end()).max().unwrap_or(0);
        let hit = self.buf.find(&needle, last, false).or_else(|| self.buf.find(&needle, 0, false));
        match hit {
            Some(at) if !self.sels.iter().any(|s| s.start() == at) => {
                self.sels.push(Sel { anchor: at, head: at + needle.len(), col: None });
                self.primary = self.sels.len() - 1;
                let pr = self.sels[self.primary];
                self.normalize();
                self.primary = self.sels.iter().position(|s| *s == pr).unwrap_or(0);
                true
            }
            _ => false,
        }
    }
    /// Ctrl+Shift+L: a selection on every occurrence of the selection (or the word).
    pub fn select_all_occurrences(&mut self) -> usize {
        let mut p = self.sels[self.primary];
        if p.is_empty() { match self.word_at(p.head) { Some((a, e)) => p = Sel { anchor: a, head: e, col: None }, None => return 0 } }
        let needle = self.selected(&p);
        let mut v = Vec::new();
        let mut at = 0;
        while let Some(i) = self.buf.find(&needle, at, false) { v.push(Sel { anchor: i, head: i + needle.len(), col: None }); at = i + needle.len(); }
        if v.is_empty() { return 0; }
        self.primary = v.iter().position(|s| s.start() == p.start()).unwrap_or(0);
        self.sels = v;
        self.sels.len()
    }
    /// Turns each multi-line selection into one cursor at the end of every line it spans.
    pub fn split_into_lines(&mut self) {
        let mut v = Vec::new();
        for s in &self.sels {
            if s.is_empty() { v.push(*s); continue; }
            let mut line = self.buf.line_start(s.start());
            loop {
                let e = self.buf.line_end(line).min(s.end());
                let a = line.max(s.start());
                v.push(Sel { anchor: a, head: e, col: None });
                if self.buf.line_end(line) >= s.end() || self.buf.line_end(line) >= self.buf.len() { break; }
                line = self.buf.line_end(line) + 1;
            }
        }
        self.sels = v;
        self.primary = self.sels.len() - 1;
        self.normalize();
    }
    /// Escape: back to the one primary cursor.
    pub fn collapse(&mut self) {
        let p = self.sels[self.primary];
        self.sels = vec![if self.sels.len() > 1 { p } else { Sel::cursor(p.head) }];
        self.primary = 0;
    }

    // ---- Line commands ----

    /// The distinct line starts touched by the selections.
    fn line_starts(&self) -> Vec<usize> {
        let mut v = Vec::new();
        for s in &self.sels {
            let mut l = self.buf.line_start(s.start());
            let last = if s.end() > s.start() && self.buf.line_start(s.end()) == s.end() { s.end() - 1 } else { s.end() };
            loop {
                v.push(l);
                let e = self.buf.line_end(l);
                if e >= last || e >= self.buf.len() { break; }
                l = e + 1;
            }
        }
        v.sort_unstable();
        v.dedup();
        v
    }
    /// Applies edits at several points, keeping selections in place around them.
    fn edit_points(&mut self, edits: Vec<(core::ops::Range<usize>, Vec<u8>)>) {
        if edits.is_empty() { return; }
        self.checkpoint(Group::Other);
        self.version = self.next_version;
        self.next_version += 1;
        let mut edits = edits;
        edits.sort_by_key(|e| e.0.start);
        for e in edits.iter().rev() { self.buf.replace(e.0.clone(), &e.1); }
        let map = |p: usize| -> usize {
            let mut d: isize = 0;
            for e in &edits {
                if e.0.end <= p { d += e.1.len() as isize - e.0.len() as isize; }
                else if e.0.start < p { return (e.0.start as isize + d) as usize + e.1.len().min(p - e.0.start); }
                else { break; }
            }
            (p as isize + d).max(0) as usize
        };
        for s in &mut self.sels { s.anchor = map(s.anchor); s.head = map(s.head); s.col = None; }
        self.normalize();
    }
    pub fn indent_lines(&mut self) {
        let ind = self.indent.clone();
        let e = self.line_starts().into_iter().map(|l| (l..l, ind.clone())).collect();
        self.edit_points(e);
    }
    pub fn outdent_lines(&mut self) {
        let width = self.indent.len().max(1);
        let mut e = Vec::new();
        for l in self.line_starts() {
            let head = self.buf.slice(l..(l + width).min(self.buf.len()));
            let n = if head.first() == Some(&b'\t') { 1 } else { head.iter().take_while(|c| **c == b' ').count() };
            if n > 0 { e.push((l..l + n, vec![])); }
        }
        self.edit_points(e);
    }
    /// Tab: indents lines under a multi-line selection, otherwise types the indent.
    pub fn tab(&mut self) {
        if self.sels.iter().any(|s| !s.is_empty() && self.buf.line_start(s.start()) != self.buf.line_start(s.end())) { self.indent_lines(); }
        else { let i = self.indent.clone(); self.type_text(&i); }
    }
    /// Comments or uncomments the selected lines with `token` (`//`, `#`, ...).
    pub fn toggle_comment(&mut self, token: &[u8]) {
        let lines = self.line_starts();
        let lead = |l: usize| { let mut a = l; while matches!(self.buf.byte(a), Some(b' ') | Some(b'\t')) { a += 1; } a };
        let all = lines.iter().all(|&l| { let a = lead(l); a == self.buf.line_end(l) || self.buf.slice(a..a + token.len()) == token });
        let mut e = Vec::new();
        for &l in &lines {
            let a = lead(l);
            if a == self.buf.line_end(l) { continue; }
            if all {
                let n = token.len() + (self.buf.byte(a + token.len()) == Some(b' ')) as usize;
                e.push((a..a + n, vec![]));
            } else {
                let mut t = token.to_vec(); t.push(b' ');
                e.push((a..a, t));
            }
        }
        self.edit_points(e);
    }
    pub fn duplicate_lines(&mut self) {
        let e = self.line_starts().into_iter().map(|l| {
            let end = self.buf.line_end(l);
            let mut t = self.buf.slice(l..end); t.push(b'\n');
            (l..l, t)
        }).collect();
        self.edit_points(e);
    }
    pub fn delete_lines(&mut self) {
        let lines = self.line_starts();
        let e: Vec<_> = lines.iter().map(|&l| {
            let n = self.buf.next_line(l);
            if n == self.buf.len() && self.buf.line_end(l) == n && l > 0 { (l - 1..n, vec![]) } else { (l..n, vec![]) }
        }).collect();
        self.edit_points(e);
        for s in &mut self.sels { *s = Sel::cursor(s.head); }
        self.normalize();
    }
    /// Moves the selected lines up or down by one, keeping the selection on them.
    pub fn move_lines(&mut self, down: bool) {
        let lines = self.line_starts();
        let (first, last) = (lines[0], *lines.last().unwrap());
        let block_end = self.buf.line_end(last);
        if down {
            if block_end >= self.buf.len() { return; }
            let next_end = self.buf.line_end(block_end + 1);
            let block = self.buf.slice(first..block_end);
            let next = self.buf.slice(block_end + 1..next_end);
            let mut t = next.clone(); t.push(b'\n'); t.extend_from_slice(&block);
            let shift = next.len() + 1;
            let sels: Vec<Sel> = self.sels.iter().map(|s| Sel { anchor: s.anchor + shift, head: s.head + shift, col: None }).collect();
            self.edit_points(vec![(first..next_end, t)]);
            self.sels = sels;
        } else {
            let Some(prev) = self.buf.prev_line(first) else { return };
            let block = self.buf.slice(first..block_end);
            let above = self.buf.slice(prev..first - 1);
            let mut t = block.clone(); t.push(b'\n'); t.extend_from_slice(&above);
            let shift = above.len() + 1;
            let sels: Vec<Sel> = self.sels.iter().map(|s| Sel { anchor: s.anchor - shift, head: s.head - shift, col: None }).collect();
            self.edit_points(vec![(prev..block_end, t)]);
            self.sels = sels;
        }
        self.normalize();
    }
    /// Joins each selected line with the next, leaving one space.
    pub fn join_lines(&mut self) {
        let mut lines = self.line_starts();
        if lines.len() > 1 { lines.pop(); }
        let mut e = Vec::new();
        for l in lines {
            let end = self.buf.line_end(l);
            if end >= self.buf.len() { continue; }
            let mut b = end + 1;
            while matches!(self.buf.byte(b), Some(b' ') | Some(b'\t')) { b += 1; }
            let mut a = end;
            while a > l && matches!(self.buf.byte(a - 1), Some(b' ') | Some(b'\t')) { a -= 1; }
            let sep = if a == l || b == self.buf.line_end(b) { vec![] } else { vec![b' '] };
            e.push((a..b, sep));
        }
        self.edit_points(e);
    }

    // ---- Clipboard ----

    /// The selected text, one selection per line; with no selection, the cursor's line.
    pub fn copy(&self) -> (Vec<u8>, bool) {
        if self.sels.iter().all(|s| s.is_empty()) {
            let mut out = Vec::new();
            for s in &self.sels { let l = self.buf.line_start(s.head); out.extend(self.buf.slice(l..self.buf.line_end(l))); out.push(b'\n'); }
            return (out, true);
        }
        let parts: Vec<Vec<u8>> = self.sels.iter().map(|s| self.selected(s)).collect();
        (parts.join(&b'\n'), false)
    }
    pub fn cut(&mut self) -> (Vec<u8>, bool) {
        let c = self.copy();
        if c.1 { self.delete_lines(); } else { self.edit(|_, s| (s.start()..s.end(), vec![], After::Start)); }
        c
    }
    /// Pastes `text`; with as many lines as cursors, each cursor gets its own line.
    /// Whole lines (copied with no selection) go above the cursor's line.
    pub fn paste(&mut self, text: &[u8], whole_lines: bool) {
        let n = self.sels.len();
        let parts: Vec<&[u8]> = text.split(|b| *b == b'\n').collect();
        let spread = n > 1 && (parts.len() == n || (parts.len() == n + 1 && parts[n].is_empty() && whole_lines));
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by_key(|i| self.sels[*i].start());
        let rank: Vec<usize> = { let mut r = vec![0; n]; for (k, i) in order.iter().enumerate() { r[*i] = k; } r };
        let mut i = 0;
        let sels = self.sels.clone();
        self.edit(|b, s| {
            let idx = sels.iter().position(|x| x == s).unwrap_or(i);
            i += 1;
            let piece: Vec<u8> = if spread { let mut p = parts[rank[idx]].to_vec(); if whole_lines { p.push(b'\n'); } p } else { text.to_vec() };
            if whole_lines && s.is_empty() {
                let l = b.line_start(s.head);
                let col = s.head - l;
                (l..l, piece.clone(), After::At(piece.len() + col))
            } else { (s.start()..s.end(), piece, After::End) }
        });
    }
}

pub fn word_left(b: &Buffer, at: usize) -> usize {
    let mut a = at;
    while a > 0 { let p = b.prev_char(a); if b.char_at(p).is_some_and(|c| c.is_whitespace()) { a = p } else { break } }
    if a == 0 { return 0; }
    let w = b.char_at(b.prev_char(a)).is_some_and(is_word);
    while a > 0 { let p = b.prev_char(a); let c = b.char_at(p).unwrap_or(' '); if !c.is_whitespace() && is_word(c) == w { a = p } else { break } }
    a
}
pub fn word_right(b: &Buffer, at: usize) -> usize {
    let mut a = at;
    let n = b.len();
    if a >= n { return n; }
    let w = b.char_at(a).is_some_and(is_word);
    let ws = b.char_at(a).is_some_and(|c| c.is_whitespace());
    if !ws { while let Some(c) = b.char_at(a) { if !c.is_whitespace() && is_word(c) == w { a = b.next_char(a) } else { break } } }
    while let Some(c) = b.char_at(a) { if c.is_whitespace() && c != '\n' { a = b.next_char(a) } else { break } }
    if a == at { a = b.next_char(a); }
    a
}
