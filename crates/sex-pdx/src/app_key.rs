// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Shell-delivered key data with a complete modifier snapshot. A decoded key
//! does not authenticate its sender; apps check actual Shell PD/task first.
const TAG: u64 = 1 << 16;
/// The word carries what the key typed under the person's layout, in bits 32..53
/// (0 there: the key typed nothing, e.g. a dead key waiting for its letter).
const TEXT: u64 = 1 << 17;
const SCALAR_SHIFT: u32 = 32;
const SCALAR_MASK: u64 = 0x1f_ffff;

/// What a key typed, as Shell's keyboard layout says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Text {
    /// Shell said nothing: the key types what it types on a US keyboard ([`us_char`]).
    /// Shell sends this whenever that is right, so programs built before layouts
    /// existed keep working on the US layout.
    Us,
    /// Nothing (a dead key or a Compose sequence under way).
    Nothing,
    Char(char),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key { pub code: u8, pub shift: bool, pub control: bool, pub text: Text }
impl Key {
    pub const fn new(code: u8, shift: bool, control: bool) -> Self { Self { code, shift, control, text: Text::Us } }
    pub const fn wire(self) -> u64 {
        let base = TAG | self.code as u64 | (self.shift as u64) << 8 | (self.control as u64) << 9;
        match self.text {
            Text::Us => base,
            Text::Nothing => base | TEXT,
            Text::Char(c) => base | TEXT | (c as u64) << SCALAR_SHIFT,
        }
    }
    pub fn decode(word: u64, down: u64, class: u64) -> Option<Self> {
        if down != 1 || class != 1 || word & !(TAG | TEXT | 0x3ff | SCALAR_MASK << SCALAR_SHIFT) != 0 || word & TAG == 0 { return None; }
        let scalar = (word >> SCALAR_SHIFT) as u32;
        let text = match (word & TEXT != 0, scalar) {
            (false, 0) => Text::Us,
            (false, _) => return None,
            (true, 0) => Text::Nothing,
            (true, s) => Text::Char(char::from_u32(s)?),
        };
        let key = Self { code: word as u8, shift: word & 256 != 0, control: word & 512 != 0, text };
        (key.code != 0 && key.wire() == word).then_some(key)
    }
    /// This key with what the layout typed; [`Text::Us`] when that is what a US
    /// keyboard types anyway.
    pub fn typing(self, typed: Option<char>) -> Self {
        let text = if typed == us_char(self).map(|b| b as char) { Text::Us } else { typed.map_or(Text::Nothing, Text::Char) };
        Self { text, ..self }
    }
    /// The character to insert for this key, if any.
    pub fn char(self) -> Option<char> {
        match self.text { Text::Us => us_char(self).map(|b| b as char), Text::Nothing => None, Text::Char(c) => Some(c) }
    }
}

/// What a key types on a US keyboard: the table programs used before layouts.
pub fn us_char(key: Key) -> Option<u8> {
    let plain: &[u8; 58] = b"\0\x001234567890-=\0\0qwertyuiop[]\0\0asdfghjkl;'`\0\\zxcvbnm,./\0*\0 ";
    let upper: &[u8; 58] = b"\0\0!@#$%^&*()_+\0\0QWERTYUIOP{}\0\0ASDFGHJKL:\"~\0|ZXCVBNM<>?\0*\0 ";
    if key.control { return None; }
    let c = *if key.shift { upper } else { plain }.get(key.code as usize)?;
    (c != 0).then_some(c)
}
/// Hardware modifier ownership remains in Shell across focus changes. Both
/// shifts/controls have separate state so releasing one preserves the other.
pub struct Modifiers { left_shift: bool, right_shift: bool, left_control: bool, right_control: bool }
impl Modifiers {
    pub const fn new() -> Self { Self { left_shift:false, right_shift:false, left_control:false, right_control:false } }
    pub fn observe(&mut self, code:u8, value:u64) -> bool {
        if value > 1 { return false; }
        let state = match code { 0x2a=>&mut self.left_shift, 0x36=>&mut self.right_shift, 0x1d=>&mut self.left_control, 0x61=>&mut self.right_control, _=>return false };
        *state = value == 1; true
    }
    pub const fn key(&self, code:u8) -> Key { Key::new(code, self.left_shift || self.right_shift, self.left_control || self.right_control) }
}

/// Media keys. On PS/2 they are E0 20 / E0 2E / E0 30, the same byte as D, C and B, and
/// on USB keyboards usages 0x7F..0x81; Input gives them these codes of their own. Shell
/// takes them for the master volume; programs never see them.
pub const KEY_MUTE: u8 = 0x75;
/// A key that only carries text: what an input method commits (`你好` after
/// `nihao` Space) arrives as one such key a character, with [`Text::Char`].
pub const KEY_TEXT: u8 = 0x7E;
pub const KEY_VOLUME_DOWN: u8 = 0x76;
pub const KEY_VOLUME_UP: u8 = 0x77;

/// Player keys: E0 22 / E0 24 / E0 19 / E0 10 on PS/2 (again sharing G, J, P and Q's
/// byte). Shell routes each to the focused program when it plays sound, else to the
/// program that played last (`sound_mixer::LAST_PLAYER`); it arrives there as an
/// ordinary key with this code.
pub const KEY_PLAY_PAUSE: u8 = 0x78;
pub const KEY_STOP: u8 = 0x7A;
pub const KEY_NEXT: u8 = 0x7C;
pub const KEY_PREVIOUS: u8 = 0x7F;
