// SPDX-License-Identifier: GPL-3.0-only
//! Searching the scrollback: plain text or (with the `regex` feature) a regular
//! expression over history and screen together, soft-wrapped rows joined into the line
//! the program wrote. Matches are in absolute rows, and the screen keeps a set of them
//! to paint ([`Screen::set_highlights`]).

use crate::{Point, Screen};
use alloc::string::String;
use alloc::vec::Vec;

/// A match from `start` to `end`, both inclusive, in absolute rows (row 0 is the oldest
/// history line). A match on a soft-wrapped line can end on a later row than it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Match {
    pub start: Point,
    pub end: Point,
}
impl Match {
    pub fn contains(&self, p: Point) -> bool { p >= self.start && p <= self.end }
}

/// How the renderer paints a cell for the search highlights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Highlight {
    None,
    Match,
    /// The match the search is on now.
    Current,
}

/// Most matches one search returns: enough for any screenful of scrollback.
const MAX_MATCHES: usize = 100_000;

impl Screen {
    /// Every match of `pattern` in history and screen, oldest first. A plain pattern is
    /// literal text; with `regex` it is a regular expression (when the crate is built
    /// without the `regex` feature, or the expression does not compile, nothing matches).
    /// Matches do not overlap and are never empty.
    pub fn search(&self, pattern: &str, regex: bool, case_sensitive: bool) -> Vec<Match> {
        let mut out = Vec::new();
        if pattern.is_empty() { return out; }
        let mut finder = match Finder::new(pattern, regex, case_sensitive) { Some(f) => f, None => return out };
        let total = self.history.len() + self.rows;
        let mut row = 0;
        let mut text = String::new();
        let mut cells: Vec<Point> = Vec::new();
        let mut offsets: Vec<usize> = Vec::new();
        while row < total && out.len() < MAX_MATCHES {
            // One logical line: this row and the rows it soft-wraps onto.
            let first = row;
            let mut last = row;
            while last + 1 < total && self.abs_line(last).is_some_and(|l| l.wrapped) { last += 1; }
            text.clear(); cells.clear(); offsets.clear();
            for r in first..=last {
                let Some(line) = self.abs_line(r) else { break };
                let keep = if r == last { line.cells.iter().rposition(|c| c.ch != ' ').map_or(0, |i| i + 1) } else { line.cells.len() };
                for (col, c) in line.cells[..keep].iter().enumerate() {
                    offsets.push(text.len());
                    cells.push(Point { row: r, col });
                    text.push(c.ch);
                }
            }
            offsets.push(text.len());
            finder.find_all(&text, &mut |a, b| {
                // Byte range a..b to the cells of its first and last characters.
                let first = offsets.partition_point(|&o| o < a);
                let last = offsets.partition_point(|&o| o < b).saturating_sub(1);
                if first < cells.len() && last >= first { out.push(Match { start: cells[first], end: cells[last.min(cells.len() - 1)] }); }
            });
            row = last + 1;
        }
        out.truncate(MAX_MATCHES);
        out
    }

    /// The matches the renderer paints, and which one is current (painted stronger).
    pub fn set_highlights(&mut self, matches: Vec<Match>, current: Option<usize>) {
        self.ext.highlights = matches;
        self.ext.highlights_base = self.ext.dropped;
        self.ext.current = current.filter(|&i| i < self.ext.highlights.len());
        self.all_dirty = true;
    }
    pub fn clear_highlights(&mut self) {
        if !self.ext.highlights.is_empty() { self.ext.highlights.clear(); self.ext.current = None; self.all_dirty = true; }
    }
    /// The highlighted matches, in today's absolute rows (ones that left the history are gone).
    pub fn highlights(&self) -> Vec<Match> {
        let shift = self.ext.dropped.wrapping_sub(self.ext.highlights_base);
        self.ext.highlights.iter().filter(|m| m.start.row >= shift).map(|m| Match {
            start: Point { row: m.start.row - shift, col: m.start.col },
            end: Point { row: m.end.row - shift, col: m.end.col },
        }).collect()
    }
    pub fn current_highlight(&self) -> Option<usize> { self.ext.current }
    /// How a visible cell is highlighted.
    pub fn highlight(&self, col: usize, row: usize) -> Highlight {
        if self.ext.highlights.is_empty() { return Highlight::None; }
        let p = self.point_at(col, row);
        // Stored rows are as they were when set; shift the point instead of every match.
        let shift = self.ext.dropped.wrapping_sub(self.ext.highlights_base);
        let p = Point { row: p.row + shift, col: p.col };
        let hs = &self.ext.highlights;
        let i = hs.partition_point(|m| m.start <= p);
        if i == 0 || !hs[i - 1].contains(p) { return Highlight::None; }
        if self.ext.current == Some(i - 1) { Highlight::Current } else { Highlight::Match }
    }
    /// Brings absolute row `row` into view, a third of the way down, unless it is
    /// already visible.
    pub fn scroll_to(&mut self, row: usize) {
        let top = self.point_at(0, 0).row;
        if row >= top && row < top + self.rows { return; }
        self.view_top(row.saturating_sub(self.rows / 3));
    }
}

/// A plain or regex matcher. Calls back with byte ranges of non-overlapping, non-empty matches.
enum Finder {
    Plain { needle: Vec<char>, fold: bool },
    #[cfg(feature = "regex")]
    Regex(regex_automata::meta::Regex),
}
impl Finder {
    fn new(pattern: &str, regex: bool, case_sensitive: bool) -> Option<Finder> {
        if regex {
            #[cfg(feature = "regex")]
            {
                let syntax = regex_automata::util::syntax::Config::new().case_insensitive(!case_sensitive);
                return regex_automata::meta::Regex::builder().syntax(syntax).build(pattern).ok().map(Finder::Regex);
            }
            #[cfg(not(feature = "regex"))]
            return None;
        }
        let fold = !case_sensitive;
        Some(Finder::Plain { needle: pattern.chars().map(|c| if fold { fold_char(c) } else { c }).collect(), fold })
    }
    fn find_all(&mut self, text: &str, hit: &mut dyn FnMut(usize, usize)) {
        match self {
            Finder::Plain { needle, fold } => {
                let hay: Vec<(usize, char)> = text.char_indices().map(|(i, c)| (i, if *fold { fold_char(c) } else { c })).collect();
                let n = needle.len();
                let mut i = 0;
                while i + n <= hay.len() {
                    if hay[i..i + n].iter().map(|p| p.1).eq(needle.iter().copied()) {
                        let end = hay.get(i + n).map_or(text.len(), |p| p.0);
                        hit(hay[i].0, end);
                        i += n;
                    } else { i += 1; }
                }
            }
            #[cfg(feature = "regex")]
            Finder::Regex(re) => {
                for m in re.find_iter(text) { if !m.is_empty() { hit(m.start(), m.end()); } }
            }
        }
    }
}

/// Simple case folding: one lowercase character per character, which is what a terminal
/// search needs (no expansions such as `ß` to `ss`).
fn fold_char(c: char) -> char { if c.is_ascii() { c.to_ascii_lowercase() } else { c.to_lowercase().next().unwrap_or(c) } }
