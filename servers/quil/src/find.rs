// SPDX-License-Identifier: GPL-3.0-only
//! Bounded in-document find and replace. Query and replacement are private
//! editor state: they never leave Quil, need no service or capability, and
//! matching compares exact UTF-8 bytes. Replacement edits only a selection
//! that is exactly one match, as one ordinary undoable edit. A valid UTF-8 query can only match on scalar boundaries of a
//! valid UTF-8 document, so a selection never splits a scalar.

pub const QUERY_MAX: usize = 32;
const PREFIX: &[u8] = b"Find: ";
const SEPARATOR: &[u8] = b" > ";
pub const PROMPT_MAX: usize = PREFIX.len() + QUERY_MAX + SEPARATOR.len() + QUERY_MAX + 1;

/// Which text the typed bytes edit. Down selects the replacement and Up the
/// query; Tab stays with the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field { Query, Replacement }

pub struct Find {
    query: [u8; QUERY_MAX],
    len: usize,
    replacement: [u8; QUERY_MAX],
    replacement_len: usize,
    field: Field,
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
    pub const fn new() -> Self {
        Self { query: [0; QUERY_MAX], len: 0, replacement: [0; QUERY_MAX], replacement_len: 0, field: Field::Query, active: false }
    }
    pub fn active(&self) -> bool { self.active }
    pub fn begin(&mut self) { self.active = true; }
    /// Leaving find mode erases the query and the replacement.
    pub fn clear(&mut self) { *self = Self::new(); }
    pub fn query(&self) -> &[u8] { &self.query[..self.len] }
    pub fn replacement(&self) -> &[u8] { &self.replacement[..self.replacement_len] }
    pub fn field(&self) -> Field { self.field }
    pub fn set_field(&mut self, field: Field) {
        if self.active { self.field = field; }
    }

    fn editing(&mut self) -> (&mut [u8; QUERY_MAX], &mut usize) {
        match self.field { Field::Query => (&mut self.query, &mut self.len), Field::Replacement => (&mut self.replacement, &mut self.replacement_len) }
    }

    /// Append one printable ASCII byte to the active field; a full field
    /// refuses without change.
    pub fn push(&mut self, byte: u8) -> bool {
        if !self.active || !(0x20..0x7f).contains(&byte) { return false; }
        let (buf, len) = self.editing();
        if *len == QUERY_MAX { return false; }
        buf[*len] = byte;
        *len += 1;
        true
    }

    pub fn pop(&mut self) -> bool {
        if !self.active { return false; }
        let (buf, len) = self.editing();
        if *len == 0 { return false; }
        *len -= 1;
        buf[*len] = 0;
        true
    }

    /// `Find: query#` while only searching; once a replacement field is in
    /// use, `Find: query > replacement` with `#` after the active field.
    pub fn prompt(&self) -> ([u8; PROMPT_MAX], usize) {
        let mut out = [0; PROMPT_MAX];
        let mut n = 0;
        let mut put = |bytes: &[u8]| for &b in bytes { out[n] = b; n += 1; };
        put(PREFIX);
        put(self.query());
        if self.field == Field::Query { put(b"#"); }
        if self.field == Field::Replacement || self.replacement_len > 0 {
            put(SEPARATOR);
            put(self.replacement());
            if self.field == Field::Replacement { put(b"#"); }
        }
        (out, n)
    }

    /// True when `selected` is exactly one occurrence of the query.
    pub fn selection_matches(&self, selected: &[u8]) -> bool {
        self.len > 0 && selected == self.query()
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
