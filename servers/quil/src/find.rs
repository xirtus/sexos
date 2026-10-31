// SPDX-License-Identifier: GPL-3.0-only
//! Bounded in-document find. The query is private editor state: it never
//! leaves Quil, needs no service or capability, and matching compares exact
//! UTF-8 bytes. A valid UTF-8 query can only match on scalar boundaries of a
//! valid UTF-8 document, so a selection never splits a scalar.

pub const QUERY_MAX: usize = 32;
const PREFIX: &[u8] = b"Find: ";
pub const PROMPT_MAX: usize = PREFIX.len() + QUERY_MAX + 1;

pub struct Find {
    query: [u8; QUERY_MAX],
    len: usize,
    active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
    /// True when the search continued from the document start.
    pub wrapped: bool,
}

impl Find {
    pub const fn new() -> Self { Self { query: [0; QUERY_MAX], len: 0, active: false } }
    pub fn active(&self) -> bool { self.active }
    pub fn begin(&mut self) { self.active = true; }
    /// Leaving find mode erases the query.
    pub fn clear(&mut self) { self.query = [0; QUERY_MAX]; self.len = 0; self.active = false; }
    pub fn query(&self) -> &[u8] { &self.query[..self.len] }

    /// Append one printable ASCII byte; a full query refuses without change.
    pub fn push(&mut self, byte: u8) -> bool {
        if !self.active || !(0x20..0x7f).contains(&byte) || self.len == QUERY_MAX { return false; }
        self.query[self.len] = byte;
        self.len += 1;
        true
    }

    pub fn pop(&mut self) -> bool {
        if !self.active || self.len == 0 { return false; }
        self.len -= 1;
        self.query[self.len] = 0;
        true
    }

    pub fn prompt(&self) -> ([u8; PROMPT_MAX], usize) {
        let mut out = [0; PROMPT_MAX];
        let mut n = 0;
        for &b in PREFIX.iter().chain(self.query().iter()).chain(b"#") {
            out[n] = b;
            n += 1;
        }
        (out, n)
    }

    /// The first occurrence starting at or after `from`, else the first from
    /// the document start that begins before `from`. Empty queries never match.
    pub fn next(&self, text: &[u8], from: usize) -> Option<Match> {
        next_match(text, self.query(), from)
    }
}

pub fn next_match(text: &[u8], needle: &[u8], from: usize) -> Option<Match> {
    if needle.is_empty() || needle.len() > text.len() { return None; }
    let last = text.len() - needle.len();
    let from = from.min(text.len());
    let at = |start: usize| text[start..start + needle.len()] == *needle;
    if let Some(start) = (from..=last).find(|&s| at(s)) {
        return Some(Match { start, end: start + needle.len(), wrapped: false });
    }
    (0..from.min(last + 1)).find(|&s| at(s)).map(|start| Match { start, end: start + needle.len(), wrapped: true })
}
