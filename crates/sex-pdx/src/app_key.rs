// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Shell-delivered key data with a complete modifier snapshot. A decoded key
//! does not authenticate its sender; apps check actual Shell PD/task first.
const TAG: u64 = 1 << 16;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key { pub code: u8, pub shift: bool, pub control: bool }
impl Key {
    pub const fn wire(self) -> u64 {
        TAG | self.code as u64 | (self.shift as u64) << 8 | (self.control as u64) << 9
    }
    pub fn decode(word: u64, down: u64, class: u64) -> Option<Self> {
        if down != 1 || class != 1 || word & !(TAG | 0x3ff) != 0 || word & TAG == 0 { return None; }
        let key = Self { code: word as u8, shift: word & 256 != 0, control: word & 512 != 0 };
        (key.code != 0 && key.wire() == word).then_some(key)
    }
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
    pub const fn key(&self, code:u8) -> Key { Key { code, shift:self.left_shift || self.right_shift, control:self.left_control || self.right_control } }
}

/// Media keys. On PS/2 they are E0 20 / E0 2E / E0 30, the same byte as D, C and B, and
/// on USB keyboards usages 0x7F..0x81; Input gives them these codes of their own. Shell
/// takes them for the master volume; programs never see them.
pub const KEY_MUTE: u8 = 0x75;
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
