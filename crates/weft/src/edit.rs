// SPDX-License-Identifier: GPL-3.0-only
//! A single line of editable text: cursor, selection, word moves, undo and redo.
//! Positions are byte offsets on character boundaries.
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LineEdit {
    pub text: String,
    pub cursor: usize,
    /// The other end of the selection; equal to `cursor` when nothing is selected.
    pub anchor: usize,
    undo: Vec<(String, usize)>,
    redo: Vec<(String, usize)>,
    /// Whether the last change was typing, so a run of typed characters undoes at once.
    typing: bool,
    pub max_len: Option<usize>,
}

fn prev(s: &str, at: usize) -> usize { s[..at].char_indices().next_back().map_or(0, |(i, _)| i) }
fn next(s: &str, at: usize) -> usize { s[at..].chars().next().map_or(at, |c| at + c.len_utf8()) }
/// Where the edit that turned `old` into `new` ends in `new`: the cursor goes there.
fn change_end(old: &str, new: &str) -> usize {
    let same = old.bytes().rev().zip(new.bytes().rev()).take_while(|(a, b)| a == b).count();
    let mut at = new.len() - same.min(new.len());
    while !new.is_char_boundary(at) { at += 1; }
    at
}
fn is_word(c: char) -> bool { c.is_alphanumeric() || c == '_' }

impl LineEdit {
    pub fn new(text: &str) -> LineEdit { LineEdit { text: text.into(), cursor: text.len(), anchor: text.len(), ..Default::default() } }
    pub fn selection(&self) -> (usize, usize) { (self.cursor.min(self.anchor), self.cursor.max(self.anchor)) }
    pub fn has_selection(&self) -> bool { self.cursor != self.anchor }
    pub fn selected(&self) -> &str { let (a, b) = self.selection(); &self.text[a..b] }

    fn remember(&mut self, typing: bool) {
        if !(typing && self.typing) {
            self.undo.push((self.text.clone(), self.cursor));
            if self.undo.len() > 100 { self.undo.remove(0); }
        }
        self.redo.clear();
        self.typing = typing;
    }

    /// Replaces everything (not undoable: the program set it).
    pub fn set_text(&mut self, text: &str) {
        self.text.clear();
        self.text.push_str(text);
        self.cursor = self.text.len();
        self.anchor = self.cursor;
        self.undo.clear();
        self.redo.clear();
    }

    fn delete_selection(&mut self) -> bool {
        if !self.has_selection() { return false; }
        let (a, b) = self.selection();
        self.text.replace_range(a..b, "");
        self.cursor = a;
        self.anchor = a;
        true
    }

    /// Types `s` over the selection.
    pub fn insert(&mut self, s: &str) -> bool {
        let s: String = s.chars().filter(|c| *c != '\n' && *c != '\r').collect();
        if s.is_empty() && !self.has_selection() { return false; }
        if let Some(max) = self.max_len {
            let room = max.saturating_sub(self.text.chars().count() - self.selected().chars().count());
            if s.chars().count() > room { return false; }
        }
        self.remember(s.chars().count() == 1 && !self.has_selection());
        self.delete_selection();
        self.text.insert_str(self.cursor, &s);
        self.cursor += s.len();
        self.anchor = self.cursor;
        true
    }

    pub fn backspace(&mut self, word: bool) -> bool {
        if self.has_selection() { self.remember(false); return self.delete_selection(); }
        if self.cursor == 0 { return false; }
        self.remember(false);
        let from = if word { self.word_left(self.cursor) } else { prev(&self.text, self.cursor) };
        self.text.replace_range(from..self.cursor, "");
        self.cursor = from;
        self.anchor = from;
        true
    }

    pub fn delete(&mut self, word: bool) -> bool {
        if self.has_selection() { self.remember(false); return self.delete_selection(); }
        if self.cursor >= self.text.len() { return false; }
        self.remember(false);
        let to = if word { self.word_right(self.cursor) } else { next(&self.text, self.cursor) };
        self.text.replace_range(self.cursor..to, "");
        true
    }

    fn word_left(&self, mut at: usize) -> usize {
        while at > 0 && !self.text[..at].chars().next_back().map_or(false, is_word) { at = prev(&self.text, at); }
        while at > 0 && self.text[..at].chars().next_back().map_or(false, is_word) { at = prev(&self.text, at); }
        at
    }
    fn word_right(&self, mut at: usize) -> usize {
        let n = self.text.len();
        while at < n && !self.text[at..].chars().next().map_or(false, is_word) { at = next(&self.text, at); }
        while at < n && self.text[at..].chars().next().map_or(false, is_word) { at = next(&self.text, at); }
        at
    }

    fn place(&mut self, to: usize, extend: bool) {
        self.cursor = to;
        if !extend { self.anchor = to; }
        self.typing = false;
    }
    pub fn left(&mut self, word: bool, extend: bool) {
        if self.has_selection() && !extend { let (a, _) = self.selection(); return self.place(a, false); }
        let to = if word { self.word_left(self.cursor) } else { prev(&self.text, self.cursor) };
        self.place(to, extend);
    }
    pub fn right(&mut self, word: bool, extend: bool) {
        if self.has_selection() && !extend { let (_, b) = self.selection(); return self.place(b, false); }
        let to = if word { self.word_right(self.cursor) } else { next(&self.text, self.cursor) };
        self.place(to, extend);
    }
    pub fn home(&mut self, extend: bool) { self.place(0, extend); }
    pub fn end(&mut self, extend: bool) { let n = self.text.len(); self.place(n, extend); }
    pub fn select_all(&mut self) { self.anchor = 0; self.cursor = self.text.len(); }
    /// Puts the cursor at `at` (snapped to a character), extending the selection or not.
    pub fn set_cursor(&mut self, mut at: usize, extend: bool) {
        at = at.min(self.text.len());
        while !self.text.is_char_boundary(at) { at -= 1; }
        self.place(at, extend);
    }
    /// Selects the word around `at`.
    pub fn select_word(&mut self, at: usize) {
        let at = at.min(self.text.len());
        let mut a = at;
        while a > 0 && self.text[..a].chars().next_back().map_or(false, is_word) { a = prev(&self.text, a); }
        let mut b = at;
        while b < self.text.len() && self.text[b..].chars().next().map_or(false, is_word) { b = next(&self.text, b); }
        self.anchor = a;
        self.cursor = b;
    }
    pub fn undo(&mut self) -> bool {
        let Some((t, _)) = self.undo.pop() else { return false };
        let old = core::mem::replace(&mut self.text, t);
        self.cursor = change_end(&old, &self.text);
        self.redo.push((old, self.cursor));
        self.anchor = self.cursor;
        self.typing = false;
        true
    }
    pub fn redo(&mut self) -> bool {
        let Some((t, _)) = self.redo.pop() else { return false };
        let old = core::mem::replace(&mut self.text, t);
        self.cursor = change_end(&old, &self.text);
        self.undo.push((old, self.cursor));
        self.anchor = self.cursor;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typing_moving_selecting_and_undo() {
        let mut e = LineEdit::new("");
        for c in "hello world".chars() { e.insert(c.encode_utf8(&mut [0; 4])); }
        assert_eq!(e.text, "hello world");
        e.left(true, false);
        assert_eq!(e.cursor, 6);
        e.left(true, true);
        assert_eq!(e.selected(), "hello ");
        e.insert("bye ");
        assert_eq!(e.text, "bye world");
        e.undo();
        assert_eq!(e.text, "hello world");
        e.undo();
        assert_eq!(e.text, "");
        e.redo();
        assert_eq!(e.text, "hello world");
        e.backspace(true);
        assert_eq!(e.text, "hello ");
        e.select_word(2);
        assert_eq!(e.selected(), "hello");
    }
    #[test]
    fn multibyte_characters_move_whole() {
        let mut e = LineEdit::new("aü€");
        e.left(false, false);
        assert_eq!(e.cursor, 3);
        e.backspace(false);
        assert_eq!(e.text, "a€");
        e.set_cursor(2, false);
        assert_eq!(e.cursor, 1);
    }
}
