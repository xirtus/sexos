// SPDX-License-Identifier: GPL-3.0-only
//! What Bodice remembers, as text files in its folder:
//!
//! * `positions.txt`: one line per book, `NAME<TAB>CHAPTER<TAB>OFFSET`, the place it
//!   was left (see [`crate::layout::Position`]).
//! * `settings.txt`: `font=N` and `night=0|1`.
use crate::layout::Position;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Positions { pub books: Vec<(String, Position)> }

impl Positions {
    pub fn parse(text: &str) -> Positions {
        let mut p = Positions::default();
        for line in text.lines() {
            let mut f = line.split('\t');
            let (Some(name), Some(c), Some(o)) = (f.next(), f.next(), f.next()) else { continue };
            let (Ok(chapter), Ok(offset)) = (c.parse(), o.parse()) else { continue };
            if !name.is_empty() { p.set(name, Position { chapter, offset }); }
        }
        p
    }
    pub fn serialize(&self) -> String {
        let mut s = String::new();
        for (name, pos) in &self.books { s.push_str(&format!("{}\t{}\t{}\n", name, pos.chapter, pos.offset)); }
        s
    }
    pub fn get(&self, name: &str) -> Option<Position> { self.books.iter().find(|(n, _)| n == name).map(|(_, p)| *p) }
    pub fn set(&mut self, name: &str, pos: Position) {
        let name: String = name.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        match self.books.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = pos,
            None => self.books.push((name, pos)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// An index into the reader's list of font sizes.
    pub font: usize,
    pub night: bool,
}

impl Default for Settings { fn default() -> Settings { Settings { font: 1, night: false } } }

impl Settings {
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            match line.split_once('=') {
                Some(("font", v)) => if let Ok(n) = v.trim().parse() { s.font = n; },
                Some(("night", v)) => s.night = v.trim() == "1",
                _ => {}
            }
        }
        s
    }
    pub fn serialize(&self) -> String { format!("font={}\nnight={}\n", self.font, self.night as u8) }
}
