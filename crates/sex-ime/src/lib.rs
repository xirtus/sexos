// SPDX-License-Identifier: GPL-3.0-only
//! Input methods: letters typed become a pre-edit string, the method offers
//! candidates for it, and choosing one commits text to the program.
//!
//! [`Pinyin`]: Mandarin by Pinyin. `nihao` offers 你好; Space takes the first
//! candidate, 1-9 take another, `-`/`=` page through them, Enter commits the
//! letters as typed, Escape drops them, Backspace takes one back. A syllable
//! string with no word for all of it offers words for its first syllables and
//! keeps the rest in the pre-edit.
//!
//! No allocation and no IPC: Shell owns one [`Pinyin`] and feeds it keys.
#![no_std]

mod dict;
pub mod syllables;

pub use dict::lookup;

/// Letters a pre-edit may hold.
pub const PREEDIT_MAX: usize = 48;
/// Candidates shown at once.
pub const PAGE: usize = 9;
/// Bytes a commit may hold (a word of up to eight characters, or the letters).
pub const COMMIT_MAX: usize = 64;

/// A key as the input method sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input { Letter(u8), Digit(u8), Space, Enter, Backspace, Escape, PrevPage, NextPage, Other }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Not the method's: the program gets the key as usual.
    Pass,
    /// The method took the key; the pre-edit or candidates changed.
    Changed,
    /// The method took the key and commits this text (see [`Pinyin::committed`]).
    Commit,
}

pub struct Pinyin {
    preedit: [u8; PREEDIT_MAX],
    len: usize,
    page: usize,
    commit: [u8; COMMIT_MAX],
    commit_len: usize,
}

impl Pinyin {
    pub const fn new() -> Self { Self { preedit: [0; PREEDIT_MAX], len: 0, page: 0, commit: [0; COMMIT_MAX], commit_len: 0 } }
    pub fn preedit(&self) -> &str { core::str::from_utf8(&self.preedit[..self.len]).unwrap_or("") }
    pub fn active(&self) -> bool { self.len != 0 }
    pub fn committed(&self) -> &str { core::str::from_utf8(&self.commit[..self.commit_len]).unwrap_or("") }
    pub fn page(&self) -> usize { self.page }

    /// The dictionary entry the candidates come from: the longest run of leading
    /// syllables (at most two) that has words, and how many letters it covers.
    fn entry(&self) -> Option<(&'static str, usize)> {
        let mut ends = [0usize; PREEDIT_MAX];
        let n = syllables::split(self.preedit(), &mut ends);
        if n == 0 { return None; }
        let mut key = [0u8; PREEDIT_MAX + 8];
        for take in (1..=n.min(2)).rev() {
            let mut k = 0;
            let mut from = 0;
            for &end in &ends[..take] {
                if k != 0 { key[k] = b'-'; k += 1; }
                let part = &self.preedit[from..end];
                key[k..k + part.len()].copy_from_slice(part);
                k += part.len();
                from = end;
            }
            if let Some(words) = lookup(core::str::from_utf8(&key[..k]).ok()?) { return Some((words, ends[take - 1])); }
        }
        None
    }

    /// The candidates on the current page, in order (up to [`PAGE`]).
    pub fn candidates(&self) -> impl Iterator<Item = &'static str> {
        let words = self.entry().map_or("", |e| e.0);
        words.split(' ').filter(|w| !w.is_empty()).skip(self.page * PAGE).take(PAGE)
    }

    fn set_commit(&mut self, text: &str) {
        let n = text.len().min(COMMIT_MAX);
        self.commit[..n].copy_from_slice(&text.as_bytes()[..n]);
        self.commit_len = n;
    }

    fn choose(&mut self, index: usize) -> Outcome {
        let Some((words, covered)) = self.entry() else { return Outcome::Changed };
        let Some(word) = words.split(' ').filter(|w| !w.is_empty()).nth(self.page * PAGE + index) else { return Outcome::Changed };
        self.set_commit(word);
        self.preedit.copy_within(covered..self.len, 0);
        self.len -= covered;
        self.page = 0;
        Outcome::Commit
    }

    pub fn key(&mut self, input: Input) -> Outcome {
        self.commit_len = 0;
        match input {
            Input::Letter(c) if c.is_ascii_lowercase() => {
                if self.len == PREEDIT_MAX { return Outcome::Changed; }
                self.preedit[self.len] = c; self.len += 1; self.page = 0;
                Outcome::Changed
            }
            _ if self.len == 0 => Outcome::Pass,
            Input::Space => self.choose(0),
            Input::Digit(d @ 1..=9) => self.choose(d as usize - 1),
            Input::Enter => {
                let mut letters = [0u8; PREEDIT_MAX];
                let n = self.len;
                letters[..n].copy_from_slice(&self.preedit[..n]);
                self.set_commit(core::str::from_utf8(&letters[..n]).unwrap_or(""));
                self.len = 0; self.page = 0;
                Outcome::Commit
            }
            Input::Backspace => { self.len -= 1; self.page = 0; Outcome::Changed }
            Input::Escape => { self.len = 0; self.page = 0; Outcome::Changed }
            Input::NextPage => { if self.candidates().count() == PAGE { self.page += 1; } Outcome::Changed }
            Input::PrevPage => { self.page = self.page.saturating_sub(1); Outcome::Changed }
            // While composing, other keys wait (as in other input methods).
            _ => Outcome::Changed,
        }
    }
}

impl Default for Pinyin { fn default() -> Self { Self::new() } }
