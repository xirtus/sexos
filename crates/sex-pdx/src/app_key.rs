// SPDX-License-Identifier: GPL-3.0-only
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
