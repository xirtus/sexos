// SPDX-License-Identifier: GPL-3.0-only
//! Spelling: the en_US Hunspell dictionary (SCOWL, assets/dictionaries/en_US) read by
//! `spellbook` (MPL-2.0), parsed the first time a word is checked. Words are runs of
//! letters with inner apostrophes; words with digits, single letters and ALL-CAPS
//! abbreviations are left alone.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

const AFF: &str = include_str!("../../../assets/dictionaries/en_US/en_US.aff");
const DIC: &str = include_str!("../../../assets/dictionaries/en_US/en_US.dic");

pub struct Checker {
    dict: Option<spellbook::Dictionary>,
    /// Words already looked up, so redrawing a screen checks nothing twice.
    known: BTreeMap<String, bool>,
}

impl Checker {
    pub fn new() -> Self { Checker { dict: None, known: BTreeMap::new() } }
    fn dict(&mut self) -> Option<&spellbook::Dictionary> {
        if self.dict.is_none() {
            let start = sex_pdx::get_ticks();
            match spellbook::Dictionary::new(AFF, DIC) {
                Ok(d) => { sex_pdx::serial_record!("[jot.spell.ready] ms={}", sex_pdx::get_ticks() - start); self.dict = Some(d); }
                Err(_) => { sex_pdx::serial_record!("[jot.spell.failed]"); return None; }
            }
        }
        self.dict.as_ref()
    }
    pub fn correct(&mut self, word: &str) -> bool {
        if let Some(k) = self.known.get(word) { return *k; }
        let ok = match self.dict() { Some(d) => d.check(word), None => true };
        if self.known.len() > 20_000 { self.known.clear(); }
        self.known.insert(word.into(), ok);
        ok
    }
    /// Byte ranges of the misspelt words in one line.
    pub fn misspelt(&mut self, line: &[u8]) -> Vec<(usize, usize)> {
        words(line).into_iter().filter(|&(a, b)| {
            let w = core::str::from_utf8(&line[a..b]).unwrap_or("");
            checkable(w) && !self.correct(w)
        }).collect()
    }
    pub fn suggest(&mut self, word: &str) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(d) = self.dict() { d.suggest(word, &mut out); }
        out
    }
}

fn letter(b: u8) -> bool { b.is_ascii_alphabetic() || b >= 0x80 }
/// Letter runs, with apostrophes inside them, as byte ranges.
pub fn words(line: &[u8]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < line.len() {
        if !letter(line[i]) { i += 1; continue; }
        let start = i;
        while i < line.len() && (letter(line[i]) || (line[i] == b'\'' && line.get(i + 1).is_some_and(|b| letter(*b)))) { i += 1; }
        // A word glued to digits or underscores is an identifier, not prose.
        let glued = |b: Option<&u8>| b.is_some_and(|b| b.is_ascii_digit() || *b == b'_');
        if !glued(line.get(i)) && !(start > 0 && glued(line.get(start - 1))) { out.push((start, i)); }
    }
    out
}
fn checkable(w: &str) -> bool { w.chars().count() > 1 && !w.chars().all(|c| c.is_uppercase()) }
