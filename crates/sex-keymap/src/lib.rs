// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Keyboard layouts: what a key types, given the layout the person chose.
//!
//! Input delivers key codes (PS/2 set 1, the same for USB keyboards). Shell keeps one
//! [`Keyboard`], feeds it every key edge, and hands the focused program the character
//! each press typed alongside the code, so programs never need a table of their own.
//!
//! - Four levels per key: plain, Shift, AltGr (right Alt), Shift+AltGr.
//! - Caps Lock shifts letters only.
//! - Dead keys: a combining mark at a level waits for the next key and makes the
//!   accented letter (`´` then `e` gives `é`); Space or the same dead key again
//!   gives the accent itself.
//! - Compose: the Menu key (and right Alt on layouts without AltGr levels) starts a
//!   sequence such as `o` `"` → `ö`, `=` `e` → `€`, `-` `-` `-` → `—`.
//!
//! Pure data and arithmetic: no IPC, no allocation, so the same file runs in host
//! tests and in Shell.
#![no_std]

pub mod compose;
mod compose_data;
pub mod layouts;

/// Key codes this module gives a meaning of its own.
pub mod code {
    pub const SHIFT_LEFT: u8 = 0x2A;
    pub const SHIFT_RIGHT: u8 = 0x36;
    pub const CTRL_LEFT: u8 = 0x1D;
    /// Right Ctrl: USB keyboards, and PS/2 E0 1D.
    pub const CTRL_RIGHT: u8 = 0x61;
    pub const ALT_LEFT: u8 = 0x38;
    /// Right Alt, AltGr: USB keyboards, and PS/2 E0 38.
    pub const ALT_GR: u8 = 0x64;
    pub const CAPS_LOCK: u8 = 0x3A;
    pub const SUPER_LEFT: u8 = 0x5B;
    pub const SUPER_RIGHT: u8 = 0x5C;
    /// The Menu (Application) key, which is the Compose key.
    pub const MENU: u8 = 0x5D;
    pub const SPACE: u8 = 0x39;
    pub const KEYPAD_STAR: u8 = 0x37;
    pub const KEYPAD_MINUS: u8 = 0x4A;
    pub const KEYPAD_PLUS: u8 = 0x4E;
}

/// One level of one key: nothing, a character, or a dead key holding its mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sym { None, Char(char), Dead(char) }

/// What one key press typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Typed {
    /// Not a typing key (Enter, arrows, F keys...), or a shortcut with Ctrl/Alt held.
    None,
    /// A dead key or a Compose sequence is under way: the program gets the key,
    /// but nothing is typed yet.
    Pending,
    /// The character to insert.
    Char(char),
}

/// How many layouts a person can switch between.
pub const MAX_CHOSEN: usize = 8;
const CODES: usize = 0x80;

pub struct Keyboard {
    table: [[Sym; 4]; CODES],
    chosen: [u8; MAX_CHOSEN],
    chosen_len: usize,
    active: usize,
    /// Right Alt starts Compose on this layout (it has no AltGr levels).
    alt_gr_composes: bool,
    shift: u8,
    ctrl: u8,
    alt: bool,
    alt_gr: bool,
    caps: bool,
    dead: Option<char>,
    composing: bool,
    compose: [char; 3],
    compose_len: usize,
    /// The table holds a layout (a `const` Keyboard loads US on first use).
    ready: bool,
}

fn decode_level(c: char) -> Sym {
    if ('\u{300}'..='\u{36f}').contains(&c) { Sym::Dead(c) } else if c == '\0' { Sym::None } else { Sym::Char(c) }
}

impl Keyboard {
    /// US layout alone, loaded on first use.
    pub const fn new() -> Self {
        Self {
            table: [[Sym::None; 4]; CODES], chosen: [0; MAX_CHOSEN], chosen_len: 1, active: 0,
            alt_gr_composes: true, shift: 0, ctrl: 0, alt: false, alt_gr: false, caps: false,
            dead: None, composing: false, compose: ['\0'; 3], compose_len: 0, ready: false,
        }
    }
    fn ready(&mut self) { if !self.ready { self.choose(["us"]); } }

    /// Fills the table for layout `index` of [`layouts::ALL`]. False if it does not parse.
    fn load(&mut self, index: usize) -> bool {
        let Some(def) = layouts::ALL.get(index) else { return false };
        let mut table = [[Sym::None; 4]; CODES];
        for (row, codes) in def.rows.iter().zip(layouts::ROW_CODES) {
            let mut keys = row.split(' ');
            for &c in codes {
                let Some(key) = keys.next() else { return false };
                let mut levels = key.chars();
                for level in 0..4 { table[c as usize][level] = levels.next().map_or(Sym::None, decode_level); }
                if levels.next().is_some() || table[c as usize][0] == Sym::None { return false; }
            }
            if keys.next().is_some() { return false; }
        }
        self.alt_gr_composes = table.iter().all(|k| matches!(k[2], Sym::None));
        table[code::SPACE as usize] = [Sym::Char(' '), Sym::Char(' '), Sym::Char('\u{a0}'), Sym::Char('\u{a0}')];
        table[code::KEYPAD_STAR as usize] = [Sym::Char('*'); 4];
        table[code::KEYPAD_MINUS as usize] = [Sym::Char('-'); 4];
        table[code::KEYPAD_PLUS as usize] = [Sym::Char('+'); 4];
        self.table = table;
        self.dead = None;
        self.composing = false;
        true
    }

    /// Choose the layouts to switch between, by name; the first becomes active.
    /// Unknown names are skipped; false (and nothing changed) if none is known.
    pub fn choose<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) -> bool {
        let mut chosen = [0u8; MAX_CHOSEN];
        let mut n = 0;
        for name in names {
            if let Some(i) = layouts::by_name(name.trim()) {
                if n < MAX_CHOSEN && !chosen[..n].contains(&(i as u8)) { chosen[n] = i as u8; n += 1; }
            }
        }
        if n == 0 || !self.load(chosen[0] as usize) { return false; }
        self.chosen = chosen;
        self.chosen_len = n;
        self.active = 0;
        self.ready = true;
        true
    }

    /// The next chosen layout (wrapping); returns it.
    pub fn next_layout(&mut self) -> &'static layouts::Def {
        self.ready();
        let next = (self.active + 1) % self.chosen_len;
        if self.load(self.chosen[next] as usize) { self.active = next; }
        self.layout()
    }
    pub fn layout(&self) -> &'static layouts::Def { &layouts::ALL[self.chosen[self.active] as usize] }
    pub fn chosen(&self) -> impl Iterator<Item = &'static layouts::Def> + '_ {
        self.chosen[..self.chosen_len].iter().map(|&i| &layouts::ALL[i as usize])
    }
    pub fn caps_lock(&self) -> bool { self.caps }
    pub fn pending(&self) -> bool { self.dead.is_some() || self.composing }

    /// Tracks modifiers and locks. True when `code` is one of them (or the Compose
    /// key), which a program never needs as a key of its own.
    pub fn observe(&mut self, code: u8, value: u64) -> bool {
        self.ready();
        let down = value != 0;
        let bit = |c: u8| if c == code::SHIFT_LEFT || c == code::CTRL_LEFT { 1 } else { 2 };
        match code {
            code::SHIFT_LEFT | code::SHIFT_RIGHT => { if down { self.shift |= bit(code) } else { self.shift &= !bit(code) } }
            code::CTRL_LEFT | code::CTRL_RIGHT => { if down { self.ctrl |= bit(code) } else { self.ctrl &= !bit(code) } }
            code::ALT_LEFT => self.alt = down,
            code::ALT_GR => {
                if self.alt_gr_composes { if value == 1 { self.start_compose(); } } else { self.alt_gr = down; }
            }
            code::CAPS_LOCK => { if value == 1 { self.caps = !self.caps; } }
            code::MENU => { if value == 1 { self.start_compose(); } }
            _ => return false,
        }
        true
    }
    fn start_compose(&mut self) {
        self.dead = None;
        self.composing = !self.composing;
        self.compose_len = 0;
    }

    /// The level a press of this key uses now (after the first `observe`/`press`).
    pub fn sym(&self, code: u8) -> Sym {
        let Some(levels) = self.table.get(code as usize) else { return Sym::None };
        let mut shift = self.shift != 0;
        if self.caps {
            if let Sym::Char(c) = levels[0] { if c.is_alphabetic() && levels[1] != levels[0] { shift = !shift; } }
        }
        let level = shift as usize + 2 * self.alt_gr as usize;
        match levels[level] {
            // A missing Shift+AltGr level falls back to AltGr.
            Sym::None if level == 3 => levels[2],
            s => s,
        }
    }

    /// The key a shortcut on `code` means. Shortcuts follow the letters on the keys:
    /// Ctrl+Z is the key that types `z` here (the key labelled Y on a US keyboard
    /// under German), reported with the US code programs already match. Keys whose
    /// plain level is not a Latin letter (Russian, digits, punctuation) keep their
    /// position, as on other systems.
    pub fn shortcut_code(&mut self, code: u8) -> u8 {
        self.ready();
        match self.table.get(code as usize).map(|levels| levels[0]) {
            Some(Sym::Char(c)) if c.is_ascii_lowercase() => layouts::us_code(c).unwrap_or(code),
            _ => code,
        }
    }

    /// One key press (not a modifier, not a release): what it typed.
    pub fn press(&mut self, code: u8) -> Typed {
        self.ready();
        if self.ctrl != 0 || self.alt {
            return Typed::None;
        }
        let sym = self.sym(code);
        if self.composing {
            let key = match sym {
                Sym::Char(c) => c,
                Sym::Dead(m) => compose::spacing(m),
                Sym::None => { self.composing = false; return Typed::None; }
            };
            self.compose[self.compose_len] = key;
            self.compose_len += 1;
            return match compose::sequence(&self.compose[..self.compose_len]) {
                compose::Seq::Done(c) => { self.composing = false; Typed::Char(c) }
                compose::Seq::Prefix if self.compose_len < 3 => Typed::Pending,
                _ => { self.composing = false; Typed::Pending }
            };
        }
        match (self.dead.take(), sym) {
            (_, Sym::None) => Typed::None,
            (None, Sym::Dead(m)) => { self.dead = Some(m); Typed::Pending }
            (None, Sym::Char(c)) => Typed::Char(c),
            (Some(d), Sym::Dead(m)) if d == m => Typed::Char(compose::spacing(d)),
            (Some(_), Sym::Dead(m)) => { self.dead = Some(m); Typed::Pending }
            (Some(d), Sym::Char(' ')) => Typed::Char(compose::spacing(d)),
            (Some(d), Sym::Char(c)) => Typed::Char(compose::accent(d, c).unwrap_or(c)),
        }
    }
}

impl Default for Keyboard {
    fn default() -> Self { Self::new() }
}
