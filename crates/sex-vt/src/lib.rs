// SPDX-License-Identifier: GPL-3.0-only
//! A terminal screen: the bytes a command-line program writes become a grid of cells.
//!
//! This is the part of a terminal that does not care where the bytes come from. Term
//! feeds it what its guests and programs print; `sex-std` feeds it a program's own
//! stdout. The escape sequences are the VT100/xterm subset that ordinary programs use:
//! cursor movement, erasing, insert and delete, scroll regions, 16, 256 and 24-bit
//! colours, bold, underline and inverse, the alternate screen, bracketed paste and the
//! cursor and status reports. `vte` (MIT/Apache-2.0) does the parsing; everything a
//! sequence means is here.
//!
//! What the program's side needs back (answers to `ESC [ 6 n` and friends) collects in
//! [`Screen::take_replies`]; keys typed into the terminal become bytes with [`keys`].
#![no_std]
extern crate alloc;

pub mod images;
mod kitty;
pub mod keys;
pub mod theme;
mod png;
mod sixel;
#[cfg(feature = "render")]
pub mod render;
pub mod copy_mode;
pub mod hints;
mod osc;
mod search;

pub use osc::{CommandMark, Event, Progress};
pub use search::{Highlight, Match};

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// A cell's colour: the terminal's own default, one of the 256 indexed colours, or RGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

pub const BOLD: u8 = 1;
pub const DIM: u8 = 2;
pub const ITALIC: u8 = 4;
pub const UNDERLINE: u8 = 8;
pub const INVERSE: u8 = 16;
pub const HIDDEN: u8 = 32;
pub const STRIKE: u8 = 64;

/// One character cell: 16 bytes, so a long scrollback stays affordable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: u8,
    /// The OSC 8 hyperlink this cell belongs to: 0 for none, otherwise an index into the
    /// screen's link table ([`Screen::link`]).
    pub link: u16,
}
impl Cell {
    pub const BLANK: Cell = Cell { ch: ' ', fg: Color::Default, bg: Color::Default, flags: 0, link: 0 };
    /// A blank in the current background, as erasing leaves (xterm's "background colour erase").
    fn erased(pen: &Pen) -> Cell { Cell { ch: ' ', fg: Color::Default, bg: pen.bg, flags: 0, link: 0 } }
}

/// The attributes new text is written with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pen { fg: Color, bg: Color, flags: u8 }
const PLAIN: Pen = Pen { fg: Color::Default, bg: Color::Default, flags: 0 };

#[derive(Clone, Copy, Debug)]
struct Saved { x: usize, y: usize, pen: Pen, origin: bool }

/// One row, and whether it continues on the next one (a soft wrap), which copying and
/// resizing use to keep long lines whole.
#[derive(Clone, Debug)]
struct Line { cells: Vec<Cell>, wrapped: bool }
impl Line {
    fn new(cols: usize) -> Self { Line { cells: vec![Cell::BLANK; cols], wrapped: false } }
    fn text(&self) -> String {
        let mut s: String = self.cells.iter().map(|c| c.ch).collect();
        let keep = s.trim_end_matches(' ').len();
        s.truncate(keep);
        s
    }
}

/// A point on the whole history: `row` counts from the oldest scrollback line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point { pub row: usize, pub col: usize }

/// How a selection's two corners are read: a run of text, whole lines, or a rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionMode {
    #[default]
    Char,
    Line,
    Block,
}

/// What the text protocols, search and copy mode keep on a screen. Rows that must survive
/// the history being trimmed are stored as "global" rows (absolute row plus `dropped`),
/// so trimming is O(1) and they are converted when read.
#[derive(Default)]
struct Extras {
    /// Lines ever dropped off the top of the history.
    dropped: usize,
    /// The hyperlink new text is written with (0: none). Not part of the pen: SGR 0 keeps it.
    link: u16,
    links: Vec<Option<osc::Link>>,
    link_gc_at: usize,
    events: Vec<Event>,
    commands: VecDeque<osc::Mark>,
    highlights: Vec<Match>,
    /// `dropped` when the highlights were set.
    highlights_base: usize,
    current: Option<usize>,
    /// Copy mode's cursor, as a global row and column.
    copy_cursor: Option<(usize, usize)>,
    selection_mode: SelectionMode,
    cwd: Option<String>,
}

pub struct Screen {
    cols: usize,
    rows: usize,
    lines: Vec<Line>,
    /// The primary screen while the alternate one is shown (full-screen programs).
    saved_primary: Option<(Vec<Line>, Saved)>,
    history: VecDeque<Line>,
    history_max: usize,
    x: usize,
    y: usize,
    /// The cursor sits past the last column: the next printed character wraps first.
    pending_wrap: bool,
    pen: Pen,
    saved: Saved,
    top: usize,
    bottom: usize,
    tabs: Vec<bool>,
    pub cursor_visible: bool,
    pub app_cursor: bool,
    pub app_keypad: bool,
    pub bracketed_paste: bool,
    autowrap: bool,
    origin: bool,
    insert: bool,
    /// Report mouse clicks (1000) and drags (1002), in SGR form (1006) when asked.
    pub mouse: u16,
    pub mouse_sgr: bool,
    title: Vec<u8>,
    title_changed: bool,
    replies: Vec<u8>,
    /// Lines scrolled back from the bottom in the view; 0 follows the output.
    view: usize,
    /// Rows changed since the last [`Screen::take_dirty`], by screen row.
    dirty: Vec<bool>,
    all_dirty: bool,
    bells: u32,
    last_char: char,
    selection: Option<(Point, Point)>,
    ext: Extras,
    /// The colours this screen is drawn in.
    theme: theme::Theme,
    /// Inline images (kitty graphics, sixel) and their placements.
    images: images::ImageStore,
}

impl Screen {
    pub fn new(cols: usize, rows: usize) -> Self {
        let (cols, rows) = (cols.max(2), rows.max(1));
        let saved = Saved { x: 0, y: 0, pen: PLAIN, origin: false };
        Screen {
            cols, rows, lines: (0..rows).map(|_| Line::new(cols)).collect(), saved_primary: None,
            history: VecDeque::new(), history_max: 5000, x: 0, y: 0, pending_wrap: false, pen: PLAIN, saved,
            top: 0, bottom: rows - 1, tabs: (0..cols).map(|c| c % 8 == 0).collect(), cursor_visible: true,
            app_cursor: false, app_keypad: false, bracketed_paste: false, autowrap: true, origin: false, insert: false,
            mouse: 0, mouse_sgr: false, title: Vec::new(), title_changed: false, replies: Vec::new(), view: 0,
            dirty: vec![true; rows], all_dirty: true, bells: 0, last_char: ' ', selection: None,
            ext: Extras::default(),
            theme: theme::MOCHA,
            images: images::ImageStore::new(),
        }
    }
    pub fn cols(&self) -> usize { self.cols }
    pub fn theme(&self) -> &theme::Theme { &self.theme }
    /// New colours: every row is drawn again.
    pub fn set_theme(&mut self, theme: theme::Theme) { if theme != self.theme { self.theme = theme; self.all_dirty = true; } }
    /// A cell's final colours in this screen's theme.
    pub fn colors(&self, cell: &Cell) -> (u32, u32) { colors_in(&self.theme, cell) }
    pub fn rows(&self) -> usize { self.rows }
    /// Where the cursor is, as (column, row) on the screen.
    pub fn cursor(&self) -> (usize, usize) { (self.x.min(self.cols - 1), self.y) }
    pub fn set_history_max(&mut self, lines: usize) { self.history_max = lines; self.trim_history(); }
    pub fn history_len(&self) -> usize { self.history.len() }
    pub fn alternate(&self) -> bool { self.saved_primary.is_some() }

    /// Feeds bytes the program wrote. Partial sequences and UTF-8 carry over to the next call.
    pub fn feed(&mut self, parser: &mut Parser, bytes: &[u8]) {
        // New output brings the view back to the bottom, as every terminal does.
        if self.view != 0 && !bytes.is_empty() { self.view = 0; self.all_dirty = true; }
        // kitty graphics arrive as APC strings, which `vte` drops: lift them out first.
        let mut rest = bytes;
        loop {
            let (used, step) = parser.1.step(rest);
            match step {
                kitty::Step::Vt(n) => parser.0.advance(&mut Performer(self), &rest[..n]),
                kitty::Step::Esc => parser.0.advance(&mut Performer(self), b"\x1b"),
                kitty::Step::Apc => { let apc = parser.1.take(); self.kitty_apc(&apc); }
                kitty::Step::Nothing => {}
            }
            rest = &rest[used..];
            if rest.is_empty() { break; }
        }
    }

    /// What the terminal answers the program (cursor reports, device attributes).
    pub fn take_replies(&mut self) -> Vec<u8> { core::mem::take(&mut self.replies) }
    /// The window title the program set, once, after it changes.
    pub fn take_title(&mut self) -> Option<&[u8]> {
        if !core::mem::take(&mut self.title_changed) { return None; }
        Some(&self.title)
    }
    pub fn take_bells(&mut self) -> u32 { core::mem::take(&mut self.bells) }

    /// The cell shown at a screen position, taking the scrolled-back view into account.
    pub fn visible(&self, col: usize, row: usize) -> Cell {
        self.visible_line(row).and_then(|l| l.cells.get(col)).copied().unwrap_or(Cell::BLANK)
    }
    fn visible_line(&self, row: usize) -> Option<&Line> {
        let h = self.history.len();
        let abs = h - self.view.min(h) + row;
        if abs < h { self.history.get(abs) } else { self.lines.get(abs - h) }
    }
    /// The text of one visible row, trailing blanks dropped.
    pub fn row_text(&self, row: usize) -> String { self.visible_line(row).map(Line::text).unwrap_or_default() }
    /// Lines of history and screen together, oldest first: `history_len() + rows()` of them.
    pub fn line_text(&self, row: usize) -> String { self.abs_line(row).map(Line::text).unwrap_or_default() }
    /// Every visible row, joined by newlines (tests and screen readers).
    pub fn text(&self) -> String {
        let mut out = String::new();
        for r in 0..self.rows { if r > 0 { out.push('\n'); } out.push_str(&self.row_text(r)); }
        out
    }
    /// Lines scrolled back from the bottom.
    pub fn view_offset(&self) -> usize { self.view }
    /// Scrolls the view: positive goes back into the history.
    pub fn scroll_view(&mut self, lines: isize) {
        let max = if self.alternate() { 0 } else { self.history.len() } as isize;
        let next = (self.view as isize + lines).clamp(0, max) as usize;
        if next != self.view { self.view = next; self.all_dirty = true; }
    }
    /// Which screen rows need drawing again; clears the marks.
    pub fn take_dirty(&mut self) -> Vec<bool> {
        let all = core::mem::take(&mut self.all_dirty);
        let out = self.dirty.iter().map(|d| *d || all).collect();
        self.dirty.iter_mut().for_each(|d| *d = false);
        out
    }
    pub fn mark_all_dirty(&mut self) { self.all_dirty = true; }

    /// The absolute position of a screen cell, for selections that survive scrolling.
    pub fn point_at(&self, col: usize, row: usize) -> Point {
        let h = self.history.len();
        Point { row: h - self.view.min(h) + row.min(self.rows - 1), col: col.min(self.cols - 1) }
    }
    pub fn select(&mut self, from: Point, to: Point) { self.select_mode(from, to, SelectionMode::Char); }
    /// A selection read as text, whole lines or a rectangle between two corners.
    pub fn select_mode(&mut self, from: Point, to: Point, mode: SelectionMode) {
        let (a, b) = if from <= to { (from, to) } else { (to, from) };
        self.selection = Some((a, b));
        self.ext.selection_mode = mode;
        self.all_dirty = true;
    }
    pub fn clear_selection(&mut self) { if self.selection.take().is_some() { self.all_dirty = true; } }
    /// Copy mode's cursor, drawn as a hollow box (absolute rows; `None` hides it).
    pub fn set_copy_cursor(&mut self, p: Option<Point>) {
        self.ext.copy_cursor = p.map(|p| (p.row + self.ext.dropped, p.col));
        self.all_dirty = true;
    }
    pub fn copy_cursor(&self) -> Option<Point> {
        let (row, col) = self.ext.copy_cursor?;
        Some(Point { row: row.checked_sub(self.ext.dropped)?, col })
    }
    /// The selection's corners (ordered) and mode.
    pub fn selection(&self) -> Option<(Point, Point, SelectionMode)> { self.selection.map(|(a, b)| (a, b, self.ext.selection_mode)) }
    pub fn selected(&self, col: usize, row: usize) -> bool {
        let Some((a, b)) = self.selection else { return false };
        let p = self.point_at(col, row);
        match self.ext.selection_mode {
            SelectionMode::Char => p >= a && p <= b,
            SelectionMode::Line => p.row >= a.row && p.row <= b.row,
            SelectionMode::Block => p.row >= a.row && p.row <= b.row && p.col >= a.col.min(b.col) && p.col <= a.col.max(b.col),
        }
    }
    /// The selected text, soft-wrapped rows joined, trailing blanks of each line dropped.
    /// A block selection is one line per row; a line selection takes whole rows.
    pub fn selection_text(&self) -> String {
        let Some((mut a, mut b)) = self.selection else { return String::new() };
        let mut out = String::new();
        match self.ext.selection_mode {
            SelectionMode::Char => {}
            SelectionMode::Line => { a.col = 0; b.col = self.cols - 1; }
            SelectionMode::Block => {
                let (l, r) = (a.col.min(b.col), a.col.max(b.col));
                for row in a.row..=b.row {
                    let Some(line) = self.abs_line(row) else { break };
                    let to = (r + 1).min(line.cells.len());
                    let piece: String = line.cells[l.min(to)..to].iter().map(|c| c.ch).collect();
                    out.push_str(piece.trim_end_matches(' '));
                    if row != b.row { out.push('\n'); }
                }
                return out;
            }
        }
        for row in a.row..=b.row {
            let Some(line) = self.abs_line(row) else { break };
            let from = if row == a.row { a.col } else { 0 };
            let to = if row == b.row { (b.col + 1).min(line.cells.len()) } else { line.cells.len() };
            let piece: String = line.cells[from.min(to)..to].iter().map(|c| c.ch).collect();
            if line.wrapped && row != b.row { out.push_str(&piece); } else {
                out.push_str(piece.trim_end_matches(' '));
                if row != b.row { out.push('\n'); }
            }
        }
        out
    }
    pub(crate) fn abs_line(&self, row: usize) -> Option<&Line> {
        let h = self.history.len();
        if row < h { self.history.get(row) } else { self.lines.get(row - h) }
    }
    /// A word around a point, for double-click selection.
    pub fn word_at(&self, p: Point) -> (Point, Point) {
        let Some(line) = self.abs_line(p.row) else { return (p, p) };
        let word = |c: char| c.is_alphanumeric() || "-_./~:@%+".contains(c);
        if !line.cells.get(p.col).is_some_and(|c| word(c.ch)) { return (p, p); }
        let mut a = p.col;
        while a > 0 && word(line.cells[a - 1].ch) { a -= 1; }
        let mut b = p.col;
        while b + 1 < line.cells.len() && word(line.cells[b + 1].ch) { b += 1; }
        (Point { row: p.row, col: a }, Point { row: p.row, col: b })
    }

    /// New size. Rows leaving the top go into the history; the cursor stays on its line.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let (cols, rows) = (cols.max(2), rows.max(1));
        if cols == self.cols && rows == self.rows { return; }
        for line in self.lines.iter_mut().chain(self.history.iter_mut()) { line.cells.resize(cols, Cell::BLANK); }
        if let Some((lines, _)) = self.saved_primary.as_mut() { for l in lines.iter_mut() { l.cells.resize(cols, Cell::BLANK); } }
        while self.lines.len() > rows {
            // Drop blank rows below the cursor first, then push the top into the history.
            if self.y + 1 < self.lines.len() && self.lines.last().is_some_and(|l| l.text().is_empty()) { self.lines.pop(); continue; }
            let top = self.lines.remove(0);
            if !self.alternate() { self.push_history(top); }
            self.y = self.y.saturating_sub(1);
        }
        while self.lines.len() < rows {
            // Growing pulls lines back out of the history, as xterm does.
            if !self.alternate() { if let Some(l) = self.history.pop_back() { self.lines.insert(0, l); self.y += 1; continue; } }
            self.lines.push(Line::new(cols));
        }
        if let Some((lines, _)) = self.saved_primary.as_mut() { lines.resize(rows, Line::new(cols)); }
        self.cols = cols;
        self.rows = rows;
        self.x = self.x.min(cols - 1);
        self.y = self.y.min(rows - 1);
        self.pending_wrap = false;
        self.top = 0;
        self.bottom = rows - 1;
        self.tabs = (0..cols).map(|c| c % 8 == 0).collect();
        self.dirty = vec![true; rows];
        self.all_dirty = true;
        self.view = 0;
        self.selection = None;
    }

    // ---- What the escape sequences do ----

    fn touch(&mut self, row: usize) { if let Some(d) = self.dirty.get_mut(row) { *d = true; } }
    fn touch_all(&mut self) { self.all_dirty = true; }
    fn push_history(&mut self, line: Line) {
        self.history.push_back(line);
        self.trim_history();
    }
    fn trim_history(&mut self) {
        let over = self.history.len().saturating_sub(self.history_max);
        if over == 0 { return; }
        self.history.drain(..over);
        for _ in 0..over { self.images_trimmed(); }
        self.rows_dropped(over);
    }
    /// `n` lines left the top of the history: everything kept in absolute rows moves up.
    fn rows_dropped(&mut self, n: usize) {
        if n == 0 { return; }
        // Selections are in absolute rows: they move up with the history.
        if let Some((a, b)) = self.selection.as_mut() {
            if a.row < n { self.selection = None; } else { a.row -= n; b.row -= n; }
        }
        self.ext.dropped = self.ext.dropped.wrapping_add(n);
        // Marks are kept in global rows; forget the ones that left.
        let dropped = self.ext.dropped;
        while self.ext.commands.front().is_some_and(|m| m.prompt.0 < dropped) { self.ext.commands.pop_front(); }
    }
    /// Lines ever dropped off the top of the history. Something holding absolute rows
    /// across output (copy mode does) subtracts the change.
    pub fn rows_dropped_total(&self) -> usize { self.ext.dropped }
    /// Moves rows `top..=bottom` up by `n`; what leaves the top of a full-screen region
    /// goes into the history.
    fn scroll_up(&mut self, n: usize) {
        let (top, bottom) = (self.top, self.bottom);
        let n = n.min(bottom - top + 1);
        let blank = Line { cells: vec![Cell::erased(&self.pen); self.cols], wrapped: false };
        for _ in 0..n {
            let line = self.lines.remove(top);
            if top == 0 && !self.alternate() { self.push_history(line); }
            self.lines.insert(bottom, blank.clone());
        }
        self.images_scrolled(top, bottom, n, true, top == 0 && !self.alternate());
        self.touch_all();
    }
    fn scroll_down(&mut self, n: usize) {
        let (top, bottom) = (self.top, self.bottom);
        let n = n.min(bottom - top + 1);
        let blank = Line { cells: vec![Cell::erased(&self.pen); self.cols], wrapped: false };
        for _ in 0..n {
            self.lines.remove(bottom);
            self.lines.insert(top, blank.clone());
        }
        self.images_scrolled(top, bottom, n, false, false);
        self.touch_all();
    }
    fn linefeed(&mut self) {
        self.pending_wrap = false;
        if self.y == self.bottom { self.scroll_up(1); } else if self.y + 1 < self.rows { self.y += 1; }
    }
    fn reverse_index(&mut self) {
        self.pending_wrap = false;
        if self.y == self.top { self.scroll_down(1); } else if self.y > 0 { self.y -= 1; }
    }
    fn print(&mut self, c: char) {
        let c = if c.is_control() { ' ' } else { c };
        if self.pending_wrap {
            if self.autowrap {
                self.lines[self.y].wrapped = true;
                self.x = 0;
                self.linefeed();
            }
            self.pending_wrap = false;
        }
        let cell = Cell { ch: c, fg: self.pen.fg, bg: self.pen.bg, flags: self.pen.flags, link: self.ext.link };
        let (x, y, cols) = (self.x, self.y, self.cols);
        let row = &mut self.lines[y].cells;
        if self.insert { row.insert(x, cell); row.truncate(cols); } else { row[x] = cell; }
        self.touch(y);
        self.last_char = c;
        if x + 1 >= cols { self.pending_wrap = true; } else { self.x += 1; }
    }
    fn goto(&mut self, x: usize, y: usize) {
        let (lo, hi) = if self.origin { (self.top, self.bottom) } else { (0, self.rows - 1) };
        self.x = x.min(self.cols - 1);
        self.y = (y + lo).clamp(lo, hi);
        self.pending_wrap = false;
    }
    fn erase_cells(&mut self, row: usize, from: usize, to: usize) {
        let blank = Cell::erased(&self.pen);
        let line = &mut self.lines[row];
        let to = to.min(line.cells.len());
        for c in &mut line.cells[from.min(to)..to] { *c = blank; }
        if to == line.cells.len() { line.wrapped = false; }
        self.touch(row);
    }
    fn erase_display(&mut self, mode: u16) {
        let (x, y, rows, cols) = (self.x, self.y, self.rows, self.cols);
        match mode {
            0 => { self.erase_cells(y, x, cols); for r in y + 1..rows { self.erase_cells(r, 0, cols); } }
            1 => { for r in 0..y { self.erase_cells(r, 0, cols); } self.erase_cells(y, 0, x + 1); }
            2 => { for r in 0..rows { self.erase_cells(r, 0, cols); } self.images_erased(2); }
            3 => { let h = self.history.len(); self.history.clear(); self.rows_dropped(h); self.view = 0; self.selection = None; self.images_erased(3); self.touch_all(); }
            _ => {}
        }
    }
    fn alt_screen(&mut self, on: bool, save_cursor: bool) {
        if on == self.alternate() { return; }
        if on {
            let cursor = Saved { x: self.x, y: self.y, pen: self.pen, origin: self.origin };
            let fresh = (0..self.rows).map(|_| Line::new(self.cols)).collect();
            self.saved_primary = Some((core::mem::replace(&mut self.lines, fresh), cursor));
            self.view = 0;
        } else if let Some((lines, cursor)) = self.saved_primary.take() {
            self.lines = lines;
            if save_cursor { self.x = cursor.x; self.y = cursor.y; self.pen = cursor.pen; self.origin = cursor.origin; }
        }
        self.pending_wrap = false;
        self.images_alt_switched();
        self.touch_all();
    }
    fn mode(&mut self, private: bool, n: u16, on: bool) {
        if !private { if n == 4 { self.insert = on; } return; }
        match n {
            1 => self.app_cursor = on,
            6 => { self.origin = on; self.goto(0, 0); }
            7 => self.autowrap = on,
            25 => { self.cursor_visible = on; self.touch(self.y); }
            47 | 1047 => self.alt_screen(on, false),
            1049 => { if on { self.save_cursor(); } self.alt_screen(on, true); if on { self.erase_display(2); } }
            1000 | 1002 | 1003 => self.mouse = if on { n } else { 0 },
            1006 => self.mouse_sgr = on,
            2004 => self.bracketed_paste = on,
            _ => {}
        }
    }
    fn save_cursor(&mut self) { self.saved = Saved { x: self.x, y: self.y, pen: self.pen, origin: self.origin }; }
    fn restore_cursor(&mut self) {
        let s = self.saved;
        self.x = s.x.min(self.cols - 1); self.y = s.y.min(self.rows - 1); self.pen = s.pen; self.origin = s.origin;
        self.pending_wrap = false;
    }
    fn reset(&mut self) {
        let (cols, rows, max) = (self.cols, self.rows, self.history_max);
        // Keep the global row count moving forward for anyone holding absolute rows.
        let dropped = self.ext.dropped.wrapping_add(self.history.len());
        *self = Screen::new(cols, rows);
        self.history_max = max;
        self.ext.dropped = dropped;
    }
    fn sgr(&mut self, params: &vte::Params) {
        let mut it = params.iter();
        if params.is_empty() { self.pen = PLAIN; return; }
        while let Some(p) = it.next() {
            let n = p[0];
            match n {
                0 => self.pen = PLAIN,
                1 => self.pen.flags |= BOLD,
                2 => self.pen.flags |= DIM,
                3 => self.pen.flags |= ITALIC,
                4 => if p.get(1) == Some(&0) { self.pen.flags &= !UNDERLINE } else { self.pen.flags |= UNDERLINE },
                7 => self.pen.flags |= INVERSE,
                8 => self.pen.flags |= HIDDEN,
                9 => self.pen.flags |= STRIKE,
                21 | 22 => self.pen.flags &= !(BOLD | DIM),
                23 => self.pen.flags &= !ITALIC,
                24 => self.pen.flags &= !UNDERLINE,
                27 => self.pen.flags &= !INVERSE,
                28 => self.pen.flags &= !HIDDEN,
                29 => self.pen.flags &= !STRIKE,
                30..=37 => self.pen.fg = Color::Indexed((n - 30) as u8),
                39 => self.pen.fg = Color::Default,
                40..=47 => self.pen.bg = Color::Indexed((n - 40) as u8),
                49 => self.pen.bg = Color::Default,
                90..=97 => self.pen.fg = Color::Indexed((n - 90 + 8) as u8),
                100..=107 => self.pen.bg = Color::Indexed((n - 100 + 8) as u8),
                38 | 48 => {
                    // Either `38;5;N` / `38;2;R;G;B` or the colon form `38:5:N` / `38:2::R:G:B`.
                    let color = if p.len() > 1 { extended(&p[1..]) } else {
                        let mut more: Vec<u16> = Vec::new();
                        match it.next().map(|q| q[0]) {
                            Some(5) => { if let Some(q) = it.next() { more.extend_from_slice(&[5, q[0]]); } }
                            Some(2) => { more.push(2); for _ in 0..3 { if let Some(q) = it.next() { more.push(q[0]); } } }
                            _ => {}
                        }
                        extended(&more)
                    };
                    if let Some(c) = color { if n == 38 { self.pen.fg = c } else { self.pen.bg = c } }
                }
                _ => {}
            }
        }
    }
}

/// `5;N` or `2;R;G;B` (with an optional colour-space id before R in the colon form).
fn extended(p: &[u16]) -> Option<Color> {
    match p {
        [5, n, ..] => Some(Color::Indexed(*n as u8)),
        [2, _, r, g, b] => Some(Color::Rgb(*r as u8, *g as u8, *b as u8)),
        [2, r, g, b, ..] => Some(Color::Rgb(*r as u8, *g as u8, *b as u8)),
        _ => None,
    }
}

/// The longest OSC string kept (an OSC 52 clipboard set is base64, so about 12 KiB of text).
pub(crate) const OSC_MAX: usize = 16 * 1024;

/// The parser state between calls: a sequence or UTF-8 character may arrive split.
/// Boxed: the OSC buffer is large. The second part lifts kitty's APC strings out first.
pub struct Parser(alloc::boxed::Box<vte::Parser<OSC_MAX>>, kitty::ApcScan);
impl Parser { pub fn new() -> Self { Parser(alloc::boxed::Box::new(vte::Parser::new_with_size()), kitty::ApcScan::default()) } }
impl Default for Parser { fn default() -> Self { Self::new() } }

struct Performer<'a>(&'a mut Screen);

fn arg(params: &vte::Params, i: usize, default: u16) -> u16 {
    match params.iter().nth(i).map(|p| p[0]) { Some(0) | None => default, Some(n) => n }
}

impl vte::Perform for Performer<'_> {
    fn print(&mut self, c: char) { self.0.print(c); }
    fn execute(&mut self, byte: u8) {
        let s = &mut *self.0;
        match byte {
            0x07 => s.bells += 1,
            0x08 => { if s.pending_wrap { s.pending_wrap = false; } else { s.x = s.x.saturating_sub(1); } }
            0x09 => {
                s.pending_wrap = false;
                let mut x = s.x + 1;
                while x < s.cols - 1 && !s.tabs[x] { x += 1; }
                s.x = x.min(s.cols - 1);
            }
            0x0A..=0x0C => s.linefeed(),
            0x0D => { s.x = 0; s.pending_wrap = false; }
            _ => {}
        }
    }
    fn csi_dispatch(&mut self, params: &vte::Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore { return; }
        let s = &mut *self.0;
        let private = intermediates.first() == Some(&b'?');
        let a = |i, d| arg(params, i, d) as usize;
        match (action, intermediates) {
            ('A', []) => { let top = if s.y >= s.top { s.top } else { 0 }; s.y = s.y.saturating_sub(a(0, 1)).max(top); s.pending_wrap = false; }
            ('B', []) | ('e', []) => { let bottom = if s.y <= s.bottom { s.bottom } else { s.rows - 1 }; s.y = (s.y + a(0, 1)).min(bottom); s.pending_wrap = false; }
            ('C', []) | ('a', []) => { s.x = (s.x + a(0, 1)).min(s.cols - 1); s.pending_wrap = false; }
            ('D', []) => { s.x = s.x.min(s.cols - 1).saturating_sub(a(0, 1)); s.pending_wrap = false; }
            ('E', []) => { s.y = (s.y + a(0, 1)).min(s.bottom.max(s.y)); s.x = 0; s.pending_wrap = false; }
            ('F', []) => { s.y = s.y.saturating_sub(a(0, 1)).max(s.top.min(s.y)); s.x = 0; s.pending_wrap = false; }
            ('G', []) | ('`', []) => { s.x = (a(0, 1) - 1).min(s.cols - 1); s.pending_wrap = false; }
            ('H', []) | ('f', []) => { let (row, col) = (a(0, 1) - 1, a(1, 1) - 1); s.goto(col, row); }
            ('d', []) => { let x = s.x; let row = a(0, 1) - 1; s.goto(x, row); }
            ('J', []) => s.erase_display(arg(params, 0, 0)),
            ('K', []) => {
                let (x, y, cols) = (s.x.min(s.cols - 1), s.y, s.cols);
                match arg(params, 0, 0) { 0 => s.erase_cells(y, x, cols), 1 => s.erase_cells(y, 0, x + 1), 2 => s.erase_cells(y, 0, cols), _ => {} }
            }
            ('X', []) => { let (x, y) = (s.x.min(s.cols - 1), s.y); let n = a(0, 1); s.erase_cells(y, x, x + n); }
            ('@', []) => {
                let (x, y, cols, blank) = (s.x.min(s.cols - 1), s.y, s.cols, Cell::erased(&s.pen));
                let row = &mut s.lines[y].cells;
                for _ in 0..a(0, 1).min(cols - x) { row.insert(x, blank); }
                row.truncate(cols);
                s.touch(y);
            }
            ('P', []) => {
                let (x, y, cols, blank) = (s.x.min(s.cols - 1), s.y, s.cols, Cell::erased(&s.pen));
                let row = &mut s.lines[y].cells;
                for _ in 0..a(0, 1).min(cols - x) { row.remove(x); row.push(blank); }
                s.touch(y);
            }
            ('L', []) | ('M', []) => {
                if s.y < s.top || s.y > s.bottom { return; }
                let top = s.top;
                s.top = s.y;
                if action == 'L' { s.scroll_down(a(0, 1)) } else {
                    // Deleting lines inside a region never feeds the history.
                    let n = a(0, 1).min(s.bottom - s.y + 1);
                    let blank = Line { cells: vec![Cell::erased(&s.pen); s.cols], wrapped: false };
                    for _ in 0..n { s.lines.remove(s.y); s.lines.insert(s.bottom, blank.clone()); }
                    s.images_scrolled(s.y, s.bottom, n, true, false);
                    s.touch_all();
                }
                s.top = top;
                s.x = 0;
                s.pending_wrap = false;
            }
            ('S', []) => {
                // Scroll up inside the region. A full-screen scroll also feeds the history.
                s.scroll_up(a(0, 1));
            }
            ('T', []) => s.scroll_down(a(0, 1)),
            ('b', []) => { let c = s.last_char; for _ in 0..a(0, 1).min(s.cols * s.rows) { s.print(c); } }
            ('g', []) => match arg(params, 0, 0) { 0 => { let x = s.x.min(s.cols - 1); s.tabs[x] = false; } 3 => s.tabs.iter_mut().for_each(|t| *t = false), _ => {} },
            ('m', []) => s.sgr(params),
            ('r', []) => {
                let top = a(0, 1) - 1;
                let bottom = (if arg(params, 1, 0) == 0 { s.rows } else { a(1, 1) }).min(s.rows) - 1;
                if top < bottom { s.top = top; s.bottom = bottom; s.goto(0, 0); }
            }
            ('s', []) => s.save_cursor(),
            ('u', []) => s.restore_cursor(),
            ('h', _) | ('l', _) => {
                for p in params.iter() { s.mode(private, p[0], action == 'h'); }
            }
            ('n', []) => match arg(params, 0, 0) {
                5 => s.replies.extend_from_slice(b"\x1b[0n"),
                6 => {
                    let row = if s.origin { s.y - s.top } else { s.y } + 1;
                    let col = s.x.min(s.cols - 1) + 1;
                    s.replies.extend_from_slice(alloc::format!("\x1b[{};{}R", row, col).as_bytes());
                }
                _ => {}
            },
            // A VT220 with colour, which is what `TERM=xterm-256color` programs expect.
            // `4`: sixel graphics.
            ('c', []) => s.replies.extend_from_slice(b"\x1b[?62;4;22c"),
            ('S', [b'?']) => s.xtsmgraphics(params),
            ('t', []) if matches!(arg(params, 0, 0), 14 | 16) => s.report_pixels(arg(params, 0, 0)),
            ('c', [b'>']) => s.replies.extend_from_slice(b"\x1b[>1;10;0c"),
            ('t', []) if arg(params, 0, 0) == 18 => {
                let (rows, cols) = (s.rows, s.cols);
                s.replies.extend_from_slice(alloc::format!("\x1b[8;{};{}t", rows, cols).as_bytes());
            }
            _ => {}
        }
    }
    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        let s = &mut *self.0;
        match (intermediates, byte) {
            ([], b'7') => s.save_cursor(),
            ([], b'8') => s.restore_cursor(),
            ([], b'D') => s.linefeed(),
            ([], b'E') => { s.x = 0; s.linefeed(); }
            ([], b'M') => s.reverse_index(),
            ([], b'H') => { let x = s.x.min(s.cols - 1); s.tabs[x] = true; }
            ([], b'c') => s.reset(),
            ([], b'=') => s.app_keypad = true,
            ([], b'>') => s.app_keypad = false,
            // DECALN fills the screen with E, the classic alignment test.
            ([b'#'], b'8') => {
                for r in 0..s.rows { for c in s.lines[r].cells.iter_mut() { *c = Cell { ch: 'E', ..Cell::BLANK }; } }
                s.touch_all();
            }
            _ => {}
        }
    }
    // DCS: sixel images (`DCS P1;P2;P3 q ... ST`).
    fn hook(&mut self, params: &vte::Params, intermediates: &[u8], ignore: bool, action: char) {
        if action == 'q' && intermediates.is_empty() && !ignore { self.0.sixel_start(params); }
    }
    fn put(&mut self, byte: u8) { self.0.sixel_put(byte); }
    fn unhook(&mut self) { self.0.sixel_end(); }
    fn osc_dispatch(&mut self, params: &[&[u8]], _bell: bool) {
        let s = &mut *self.0;
        if let [kind, title, ..] = params {
            if *kind == b"0" || *kind == b"2" {
                s.title = title.iter().copied().take(64).collect();
                s.title_changed = true;
            }
        }
        // Hyperlinks, clipboard, notifications, working directory, shell marks: osc.rs.
        s.osc(params);
    }
}

/// The xterm palette: 16 named colours (here a calm dark theme), the 6x6x6 cube and
/// 24 greys. 0x00RRGGBB.
pub fn palette(i: u8) -> u32 {
    const NAMED: [u32; 16] = [
        0x1E1E2E, 0xF38BA8, 0xA6E3A1, 0xF9E2AF, 0x89B4FA, 0xF5C2E7, 0x94E2D5, 0xBAC2DE,
        0x585B70, 0xF38BA8, 0xA6E3A1, 0xF9E2AF, 0x89B4FA, 0xF5C2E7, 0x94E2D5, 0xA6ADC8,
    ];
    match i {
        0..=15 => NAMED[i as usize],
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v as u32 };
            level(i / 36) << 16 | level(i / 6 % 6) << 8 | level(i % 6)
        }
        _ => { let g = 8 + 10 * (i as u32 - 232); g << 16 | g << 8 | g }
    }
}
pub const DEFAULT_FG: u32 = 0xCDD6F4;
pub const DEFAULT_BG: u32 = 0x11111B;

/// A cell's final colours, 0x00RRGGBB, after bold-brightening and inverse.
pub fn colors(cell: &Cell) -> (u32, u32) { colors_in(&theme::MOCHA, cell) }
/// A cell's final colours in `theme`.
pub fn colors_in(theme: &theme::Theme, cell: &Cell) -> (u32, u32) {
    let palette = |i: u8| theme.palette(i);
    let resolve = |c: Color, default: u32, bright: bool| match c {
        Color::Default => default,
        Color::Indexed(i) if bright && i < 8 => palette(i + 8),
        Color::Indexed(i) => palette(i),
        Color::Rgb(r, g, b) => (r as u32) << 16 | (g as u32) << 8 | b as u32,
    };
    let mut fg = resolve(cell.fg, theme.fg, cell.flags & BOLD != 0);
    let mut bg = resolve(cell.bg, theme.bg, false);
    if cell.flags & INVERSE != 0 { core::mem::swap(&mut fg, &mut bg); }
    if cell.flags & DIM != 0 { fg = (fg >> 1) & 0x7F7F7F; }
    if cell.flags & HIDDEN != 0 { fg = bg; }
    (fg, bg)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_text;

/// A fixed screen in the spirit of vttest's first pages: a frame drawn with absolute
/// moves, colours from all three palettes, and character insert, delete and erase. Term's
/// `vtcheck` draws it on the real system and compares with [`check::ROWS`], which the host
/// tests prove; so the VT that runs is the VT that was tested.
pub mod check {
    pub const FIXTURE: &str = concat!(
        "\x1b[H\x1b[2J\x1b[1;1H+--------------------+\x1b[6;1H+--------------------+",
        "\x1b[2;1H|\x1b[2;22H|\x1b[3;1H|\x1b[3;22H|\x1b[4;1H|\x1b[4;22H|\x1b[5;1H|\x1b[5;22H|",
        "\x1b[2;3H\x1b[41;97m RED \x1b[0m \x1b[42;30m GREEN \x1b[0m",
        "\x1b[3;3H\x1b[48;5;27m\x1b[38;2;255;255;0m TRUE \x1b[0m",
        "\x1b[4;3HXXXXXXXXXX\x1b[4;5H\x1b[3P\x1b[4;3H\x1b[2@",
        "\x1b[5;3Hwrap-test\x1b[5;7H\x1b[K\x1b[7;1H",
    );
    pub const ROWS: [&str; 6] = [
        "+--------------------+",
        "|  RED   GREEN       |",
        "|  TRUE              |",
        "|   XXXXXXX         |",
        "| wrap",
        "+--------------------+",
    ];
    /// Cells whose colours the check also looks at: (column, row, foreground, background).
    /// Blank cells, so the drawn pixel there is the background whatever the glyphs look like.
    pub const COLORS: [(usize, usize, u32, u32); 3] = [(2, 1, 0xA6ADC8, 0xF38BA8), (8, 1, 0x1E1E2E, 0xA6E3A1), (2, 2, 0xFFFF00, 0x005FFF)];
}
