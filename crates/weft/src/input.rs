// SPDX-License-Identifier: GPL-3.0-only
//! Input Weft understands: keys as Shell sends them (PS/2 set 1 codes, Shift, Ctrl and
//! the text the person's layout typed) and pointer events in window pixels. The
//! `sexos` feature converts `sex_app::Event`s; host tests build these directly.
use alloc::string::String;
use core::fmt::Write;

/// Key codes Weft acts on (PS/2 set 1; Input's normalized codes are accepted too).
pub mod code {
    pub const ESCAPE: u8 = 0x01;
    pub const BACKSPACE: u8 = 0x0E;
    pub const TAB: u8 = 0x0F;
    pub const ENTER: u8 = 0x1C;
    pub const SPACE: u8 = 0x39;
    pub const F1: u8 = 0x3B;
    pub const F10: u8 = 0x44;
    pub const F11: u8 = 0x57;
    pub const F12: u8 = 0x58;
    pub const HOME: u8 = 0x47;
    pub const UP: u8 = 0x48;
    pub const PAGE_UP: u8 = 0x49;
    pub const LEFT: u8 = 0x4B;
    pub const RIGHT: u8 = 0x4D;
    pub const END: u8 = 0x4F;
    pub const DOWN: u8 = 0x50;
    pub const PAGE_DOWN: u8 = 0x51;
    pub const DELETE: u8 = 0x53;
    /// The context-menu key.
    pub const MENU: u8 = 0x5D;
    /// A key that only carries text (an input method's commit).
    pub const TEXT: u8 = 0x7E;
}

/// Movement and editing keys, whichever code Shell used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav { Left, Right, Up, Down, Home, End, PageUp, PageDown, Delete, Backspace, Tab, Enter, Escape, Space }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub code: u8,
    pub shift: bool,
    pub control: bool,
    /// What the layout typed, if anything (none while Ctrl is held).
    pub text: Option<char>,
}

impl Key {
    pub const fn new(code: u8) -> Self { Self { code, shift: false, control: false, text: None } }
    pub const fn ctrl(code: u8) -> Self { Self { code, shift: false, control: true, text: None } }
    pub const fn shift(mut self) -> Self { self.shift = true; self }
    /// A key that typed `c` (letters and digits get their US code, so shortcuts see them).
    pub fn typed(c: char) -> Self {
        let code = letter_code(c.to_ascii_lowercase()).unwrap_or(code::TEXT);
        Self { code, shift: c.is_uppercase(), control: false, text: Some(c) }
    }
    pub fn nav(&self) -> Option<Nav> {
        Some(match self.code {
            0x4B | 0x69 => Nav::Left, 0x4D | 0x6A => Nav::Right, 0x48 | 0x67 => Nav::Up, 0x50 | 0x6C => Nav::Down,
            0x47 | 0x66 => Nav::Home, 0x4F | 0x6B => Nav::End, 0x49 | 0x68 => Nav::PageUp, 0x51 | 0x6D => Nav::PageDown,
            0x53 | 0x6F => Nav::Delete, code::BACKSPACE => Nav::Backspace, code::TAB => Nav::Tab,
            code::ENTER => Nav::Enter, code::ESCAPE => Nav::Escape, code::SPACE if self.text.map_or(true, |c| c == ' ') => Nav::Space,
            _ => return None,
        })
    }
    /// Text to insert: what the layout typed, never a control character.
    pub fn insert(&self) -> Option<char> {
        if self.control { return None; }
        self.text.filter(|c| !c.is_control())
    }
}

/// Mouse buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button { Left, Middle, Right }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerKind {
    Move,
    Down(Button),
    Up(Button),
    /// Wheel steps; positive moves toward the document's start (scrolls up).
    Wheel(i8),
    Leave,
}

/// A pointer event in window pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pointer { pub x: f32, pub y: f32, pub kind: PointerKind }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Key(Key),
    Pointer(Pointer),
    /// The window's pixel area is now this size.
    Resize { width: u32, height: u32 },
    /// The person asked to close the window.
    Close,
}

/// A key chord: `Ctrl+Shift+P`, `F2`, `Ctrl+W`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Shortcut { pub code: u8, pub control: bool, pub shift: bool }

impl Shortcut {
    pub const fn new(code: u8, control: bool, shift: bool) -> Self { Self { code, control, shift } }
    /// Parses `Ctrl+Shift+P`, `ctrl+w`, `F5`, `Shift+F10`, `Ctrl+Tab`, `Ctrl+=`.
    pub fn parse(text: &str) -> Option<Self> {
        let mut s = Shortcut { code: 0, control: false, shift: false };
        for part in text.split('+').map(str::trim) {
            let lower = lower(part);
            match lower.as_str() {
                "ctrl" | "control" => s.control = true,
                "shift" => s.shift = true,
                "" if text.ends_with("++") || text == "+" => s.code = 0x0D,
                name => s.code = named_code(name)?,
            }
        }
        (s.code != 0).then_some(s)
    }
    pub fn matches(&self, key: &Key) -> bool {
        self.control == key.control && self.shift == key.shift && same_code(self.code, key.code)
    }
    /// `Ctrl+Shift+P`, as menus show it.
    pub fn label(&self) -> String {
        let mut out = String::new();
        if self.control { out.push_str("Ctrl+"); }
        if self.shift { out.push_str("Shift+"); }
        match code_name(self.code) {
            Some(n) => out.push_str(n),
            None => { let _ = write!(out, "#{:02X}", self.code); }
        }
        out
    }
}

fn same_code(a: u8, b: u8) -> bool {
    if a == b { return true; }
    let norm = |c: u8| match c { 0x69 => 0x4B, 0x6A => 0x4D, 0x67 => 0x48, 0x6C => 0x50, 0x66 => 0x47, 0x6B => 0x4F, 0x68 => 0x49, 0x6D => 0x51, 0x6F => 0x53, c => c };
    norm(a) == norm(b)
}

fn lower(s: &str) -> String { s.chars().map(|c| c.to_ascii_lowercase()).collect() }

const LETTERS: &[u8; 26] = &[0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31, 0x18, 0x19, 0x10, 0x13, 0x1F, 0x14, 0x16, 0x2F, 0x11, 0x2D, 0x15, 0x2C];
const NAMES: &[(&str, u8)] = &[
    ("Esc", 0x01), ("Escape", 0x01), ("Backspace", 0x0E), ("Tab", 0x0F), ("Enter", 0x1C), ("Return", 0x1C), ("Space", 0x39),
    ("-", 0x0C), ("Minus", 0x0C), ("=", 0x0D), ("Plus", 0x0D), ("Equals", 0x0D), ("[", 0x1A), ("]", 0x1B), (";", 0x27), ("'", 0x28), ("`", 0x29),
    ("\\", 0x2B), (",", 0x33), (".", 0x34), ("/", 0x35), ("F1", 0x3B), ("F2", 0x3C), ("F3", 0x3D), ("F4", 0x3E), ("F5", 0x3F), ("F6", 0x40),
    ("F7", 0x41), ("F8", 0x42), ("F9", 0x43), ("F10", 0x44), ("F11", 0x57), ("F12", 0x58), ("Home", 0x47), ("Up", 0x48),
    ("PageUp", 0x49), ("Left", 0x4B), ("Right", 0x4D), ("End", 0x4F), ("Down", 0x50), ("PageDown", 0x51), ("Delete", 0x53), ("Del", 0x53),
    ("Insert", 0x52), ("Menu", 0x5D),
];

fn letter_code(c: char) -> Option<u8> {
    match c {
        'a'..='z' => Some(LETTERS[c as usize - 'a' as usize]),
        '1'..='9' => Some(c as u8 - b'1' + 0x02),
        '0' => Some(0x0B),
        _ => None,
    }
}

fn named_code(name: &str) -> Option<u8> {
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if let Some(code) = letter_code(c) { return Some(code); }
    }
    NAMES.iter().find(|(n, _)| lower(n) == name).map(|(_, c)| *c)
}

fn code_name(code: u8) -> Option<&'static str> {
    const UPPER: [&str; 26] = ["A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z"];
    const DIGITS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
    if let Some(i) = LETTERS.iter().position(|c| *c == code) { return Some(UPPER[i]); }
    if (0x02..=0x0B).contains(&code) { return Some(DIGITS[(code - 0x02) as usize]); }
    NAMES.iter().find(|(_, c)| *c == code).map(|(n, _)| *n)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shortcuts_parse_label_and_match() {
        let p = Shortcut::parse("Ctrl+Shift+P").unwrap();
        assert_eq!(p, Shortcut::new(0x19, true, true));
        assert_eq!(p.label(), "Ctrl+Shift+P");
        assert!(p.matches(&Key::ctrl(0x19).shift()));
        assert!(!p.matches(&Key::ctrl(0x19)));
        assert_eq!(Shortcut::parse("f2").unwrap().label(), "F2");
        assert_eq!(Shortcut::parse("Ctrl+Tab").unwrap(), Shortcut::new(0x0F, true, false));
        assert_eq!(Shortcut::parse("Ctrl+1").unwrap().label(), "Ctrl+1");
        assert!(Shortcut::parse("Ctrl+Left").unwrap().matches(&Key::ctrl(0x69)));
        assert_eq!(Shortcut::parse("Ctrl+Bogus"), None);
    }
    #[test]
    fn typed_keys_carry_their_letter_code() {
        assert_eq!(Key::typed('s').code, 0x1F);
        assert_eq!(Key::typed('ü').code, code::TEXT);
        assert_eq!(Key::typed('S').shift, true);
    }
}
