// SPDX-License-Identifier: GPL-3.0-only
//! Copy mode: selecting text from history and screen with the keyboard, vi style, as
//! tmux, kitty and WezTerm do.
//!
//! A cursor moves over every line the terminal keeps (`h j k l` and the arrows, `w b e`,
//! `0 $`, `g G`, `Ctrl+U`/`Ctrl+D` and page keys); `v`, `V` and `Ctrl+V` start a
//! character, line or block selection; `y` (or Enter) yanks it, returning the text;
//! `/` and `?` search forward and backward (with [`Screen::search`]), `n`/`N` go to the
//! next and previous match; `Esc` or `q` leaves. The screen shows the selection, the
//! matches and the copy cursor; this type keeps the rest.

use crate::keys::{Key, Mods};
use crate::{Match, Point, Screen, SelectionMode};
use alloc::string::String;
use alloc::vec::Vec;

/// What a key did, for the terminal window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing changed.
    None,
    /// The cursor, selection, search or view changed: draw again.
    Redraw,
    /// The selection was yanked (copy mode has ended): put it on the clipboard.
    Yank(String),
    /// Copy mode has ended without a yank.
    Exit,
}

pub struct CopyMode {
    cursor: Point,
    anchor: Option<Point>,
    mode: SelectionMode,
    /// `Screen::rows_dropped_total` when the rows above were last brought up to date.
    seen: usize,
    /// The search being typed, and whether it goes forward.
    typing: Option<(String, bool)>,
    last: Option<(String, bool)>,
    matches: Vec<Match>,
    current: Option<usize>,
    /// Search patterns are regular expressions (they are plain text otherwise).
    pub regex: bool,
}

impl CopyMode {
    /// Starts copy mode with its cursor on the terminal's cursor.
    pub fn new(screen: &mut Screen) -> CopyMode {
        let (x, y) = screen.cursor();
        let cursor = Point { row: screen.history_len() + y, col: x };
        let mut cm = CopyMode { cursor, anchor: None, mode: SelectionMode::Char, seen: screen.rows_dropped_total(), typing: None, last: None, matches: Vec::new(), current: None, regex: false };
        // Start where the user is looking when the view is scrolled back.
        if screen.view_offset() != 0 { cm.cursor = screen.point_at(0, screen.rows() / 2); }
        screen.clear_selection();
        screen.set_copy_cursor(Some(cm.cursor));
        cm
    }
    pub fn cursor(&self) -> Point { self.cursor }
    /// The search being typed and whether it goes forward (`/`) or backward (`?`), for the
    /// window to show in a prompt line.
    pub fn search_prompt(&self) -> Option<(&str, bool)> { self.typing.as_ref().map(|(s, f)| (s.as_str(), *f)) }
    /// The matches of the last search and which one the cursor is on.
    pub fn matches(&self) -> (&[Match], Option<usize>) { (&self.matches, self.current) }

    /// Leaves copy mode: selection, highlights and copy cursor are cleared.
    pub fn exit(&mut self, screen: &mut Screen) {
        screen.clear_selection();
        screen.clear_highlights();
        screen.set_copy_cursor(None);
        self.anchor = None;
        self.typing = None;
    }

    /// Handles one key.
    pub fn key(&mut self, screen: &mut Screen, key: Key, mods: Mods) -> Action {
        self.sync(screen);
        if let Some((text, forward)) = self.typing.as_mut() {
            match key {
                Key::Escape => { self.typing = None; }
                Key::Enter => {
                    let (pat, fwd) = (core::mem::take(text), *forward);
                    self.typing = None;
                    self.run_search(screen, pat, fwd);
                }
                Key::Backspace => { if text.pop().is_none() { self.typing = None; } }
                Key::Char(c) if !mods.control && !mods.alt => text.push(c),
                _ => return Action::None,
            }
            return Action::Redraw;
        }
        let rows = screen.rows();
        let total = screen.history_len() + rows;
        let p = self.cursor;
        let to = |row: usize, col: usize| Point { row: row.min(total - 1), col: col.min(screen.cols() - 1) };
        let next = match (key, mods.control) {
            (Key::Char('h'), false) | (Key::Left, _) => Some(to(p.row, p.col.saturating_sub(1))),
            (Key::Char('l'), false) | (Key::Right, _) => Some(to(p.row, p.col + 1)),
            (Key::Char('k'), false) | (Key::Up, _) => Some(to(p.row.saturating_sub(1), p.col)),
            (Key::Char('j'), false) | (Key::Down, _) => Some(to(p.row + 1, p.col)),
            (Key::Char('u'), true) => Some(to(p.row.saturating_sub(rows / 2), p.col)),
            (Key::Char('d'), true) => Some(to(p.row + rows / 2, p.col)),
            (Key::Char('b'), true) | (Key::PageUp, _) => Some(to(p.row.saturating_sub(rows), p.col)),
            (Key::Char('f'), true) | (Key::PageDown, _) => Some(to(p.row + rows, p.col)),
            (Key::Char('0'), false) | (Key::Home, _) => Some(to(p.row, 0)),
            (Key::Char('$'), false) | (Key::End, _) => {
                let len = screen.line_text(p.row).chars().count();
                Some(to(p.row, len.saturating_sub(1)))
            }
            (Key::Char('^'), false) => {
                let t = screen.line_text(p.row);
                Some(to(p.row, t.chars().position(|c| c != ' ').unwrap_or(0)))
            }
            (Key::Char('g'), false) => Some(Point { row: 0, col: 0 }),
            (Key::Char('G'), false) => Some(to(total - 1, 0)),
            (Key::Char('w'), false) => Some(word_forward(screen, p)),
            (Key::Char('b'), false) => Some(word_back(screen, p)),
            (Key::Char('e'), false) => Some(word_end(screen, p)),
            _ => None,
        };
        if let Some(n) = next { self.move_to(screen, n); return Action::Redraw; }
        match (key, mods.control) {
            (Key::Char('v'), false) => self.toggle(screen, SelectionMode::Char),
            (Key::Char('V'), false) => self.toggle(screen, SelectionMode::Line),
            (Key::Char('v'), true) => self.toggle(screen, SelectionMode::Block),
            (Key::Char('y'), false) | (Key::Enter, _) => {
                let text = if self.anchor.is_some() { screen.selection_text() } else { screen.line_text(self.cursor.row) };
                self.exit(screen);
                return Action::Yank(text);
            }
            (Key::Char('/'), false) => self.typing = Some((String::new(), true)),
            (Key::Char('?'), false) => self.typing = Some((String::new(), false)),
            (Key::Char('n'), false) => self.step(screen, false),
            (Key::Char('N'), false) => self.step(screen, true),
            (Key::Escape, _) | (Key::Char('q'), false) | (Key::Char('c'), true) => { self.exit(screen); return Action::Exit; }
            _ => return Action::None,
        }
        Action::Redraw
    }

    /// Moves the rows held here up by the lines that left the history since last time.
    fn sync(&mut self, screen: &Screen) {
        let now = screen.rows_dropped_total();
        let gone = now.wrapping_sub(self.seen);
        self.seen = now;
        if gone == 0 { return; }
        self.cursor.row = self.cursor.row.saturating_sub(gone);
        if let Some(a) = self.anchor.as_mut() { a.row = a.row.saturating_sub(gone); }
        self.matches.clear();
        self.current = None;
    }

    fn move_to(&mut self, screen: &mut Screen, p: Point) {
        self.cursor = p;
        screen.set_copy_cursor(Some(p));
        if let Some(a) = self.anchor { screen.select_mode(a, p, self.mode); }
        // Keep the cursor in view.
        let top = screen.point_at(0, 0).row;
        let rows = screen.rows();
        if p.row < top { screen.view_top(p.row); } else if p.row >= top + rows { screen.view_top(p.row + 1 - rows); }
    }

    fn toggle(&mut self, screen: &mut Screen, mode: SelectionMode) {
        if self.anchor.is_some() && self.mode == mode {
            self.anchor = None;
            screen.clear_selection();
            return;
        }
        let a = *self.anchor.get_or_insert(self.cursor);
        self.mode = mode;
        screen.select_mode(a, self.cursor, mode);
    }

    fn run_search(&mut self, screen: &mut Screen, pattern: String, forward: bool) {
        if pattern.is_empty() { return; }
        // Smart case: an upper-case letter makes the search case-sensitive.
        let case = pattern.chars().any(char::is_uppercase);
        self.matches = screen.search(&pattern, self.regex, case);
        self.current = None;
        self.last = Some((pattern, forward));
        screen.set_highlights(self.matches.clone(), None);
        self.step(screen, false);
    }

    /// Goes to the next match in the search's direction (`reverse`: the other way),
    /// wrapping around.
    fn step(&mut self, screen: &mut Screen, reverse: bool) {
        let Some((pat, forward)) = self.last.as_ref() else { return };
        let forward = *forward != reverse;
        if self.matches.is_empty() {
            // The matches went with the history lines they were on: search again.
            let case = pat.chars().any(char::is_uppercase);
            self.matches = screen.search(pat, self.regex, case);
            if self.matches.is_empty() { return; }
        }
        let p = self.cursor;
        let n = self.matches.len();
        let i = if forward {
            self.matches.iter().position(|m| m.start > p).unwrap_or(0)
        } else {
            self.matches.iter().rposition(|m| m.start < p).unwrap_or(n - 1)
        };
        self.current = Some(i);
        screen.set_highlights(self.matches.clone(), Some(i));
        let m = self.matches[i];
        self.move_to(screen, m.start);
        screen.scroll_to(m.start.row);
    }
}

// ---- Word motions over history and screen, as one run of cells ----

fn class(c: char) -> u8 { if c == ' ' || c == '\t' { 0 } else if c.is_alphanumeric() || c == '_' { 1 } else { 2 } }

struct Cells<'a> { screen: &'a Screen, cols: usize, total: usize }
impl Cells<'_> {
    fn new(screen: &Screen) -> Cells<'_> { Cells { screen, cols: screen.cols(), total: (screen.history_len() + screen.rows()) * screen.cols() } }
    fn idx(&self, p: Point) -> usize { p.row * self.cols + p.col }
    fn point(&self, i: usize) -> Point { Point { row: i / self.cols, col: i % self.cols } }
    /// The character class at a position; the end of a row that does not wrap counts as a blank.
    fn class(&self, i: usize) -> u8 {
        let p = self.point(i);
        match self.screen.abs_line(p.row) {
            Some(l) => class(l.cells.get(p.col).map_or(' ', |c| c.ch)),
            None => 0,
        }
    }
    fn row_break(&self, i: usize) -> bool {
        // Moving from i to i + 1 crosses a row end that is a real line end.
        (i + 1) % self.cols == 0 && !self.screen.abs_line(i / self.cols).is_some_and(|l| l.wrapped)
    }
}

fn word_forward(screen: &Screen, p: Point) -> Point {
    let c = Cells::new(screen);
    let mut i = c.idx(p);
    let start = c.class(i);
    if start != 0 { while i + 1 < c.total && c.class(i + 1) == start && !c.row_break(i) { i += 1; } }
    i += 1;
    while i < c.total && c.class(i) == 0 { i += 1; }
    c.point(i.min(c.total - 1))
}
fn word_end(screen: &Screen, p: Point) -> Point {
    let c = Cells::new(screen);
    let mut i = c.idx(p) + 1;
    while i < c.total && c.class(i) == 0 { i += 1; }
    if i >= c.total { return c.point(c.total - 1); }
    let k = c.class(i);
    while i + 1 < c.total && c.class(i + 1) == k && !c.row_break(i) { i += 1; }
    c.point(i)
}
fn word_back(screen: &Screen, p: Point) -> Point {
    let c = Cells::new(screen);
    let mut i = c.idx(p);
    if i == 0 { return p; }
    i -= 1;
    while i > 0 && c.class(i) == 0 { i -= 1; }
    let k = c.class(i);
    while i > 0 && c.class(i - 1) == k && !c.row_break(i - 1) { i -= 1; }
    c.point(i)
}
