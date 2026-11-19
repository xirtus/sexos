// SPDX-License-Identifier: GPL-3.0-only
//! SexOS keys to X11 keysyms, which RFB's KeyEvent carries. A SexOS key is a keyboard
//! code (PS/2 set 1, or Input's normalized code for the extended keys) and the
//! character the person's layout typed with it, if any.

pub const BACKSPACE: u32 = 0xFF08;
pub const TAB: u32 = 0xFF09;
pub const RETURN: u32 = 0xFF0D;
pub const ESCAPE: u32 = 0xFF1B;
pub const HOME: u32 = 0xFF50;
pub const LEFT: u32 = 0xFF51;
pub const UP: u32 = 0xFF52;
pub const RIGHT: u32 = 0xFF53;
pub const DOWN: u32 = 0xFF54;
pub const PAGE_UP: u32 = 0xFF55;
pub const PAGE_DOWN: u32 = 0xFF56;
pub const END: u32 = 0xFF57;
pub const INSERT: u32 = 0xFF63;
pub const F1: u32 = 0xFFBE;
pub const SHIFT_L: u32 = 0xFFE1;
pub const CONTROL_L: u32 = 0xFFE3;
pub const DELETE: u32 = 0xFFFF;

/// The keysym for a key that types no character, from its code.
pub fn special(code: u8) -> Option<u32> {
    Some(match code {
        0x01 => ESCAPE,
        0x0E => BACKSPACE,
        0x0F => TAB,
        0x1C => RETURN,
        0x3B..=0x44 => F1 + (code - 0x3B) as u32,
        0x57 => F1 + 10,
        0x58 => F1 + 11,
        0x4B | 0x69 => LEFT,
        0x4D | 0x6A => RIGHT,
        0x48 | 0x67 => UP,
        0x50 | 0x6C => DOWN,
        0x47 | 0x66 => HOME,
        0x4F | 0x6B => END,
        0x49 | 0x68 => PAGE_UP,
        0x51 | 0x6D => PAGE_DOWN,
        0x52 | 0x6E => INSERT,
        0x53 | 0x6F => DELETE,
        _ => return None,
    })
}

/// The keysym for a character: Latin-1 is its own keysym, the rest of Unicode is
/// `0x0100_0000 + code point`; control characters have none.
pub fn from_char(c: char) -> Option<u32> {
    let cp = c as u32;
    match cp {
        0x20..=0x7E | 0xA0..=0xFF => Some(cp),
        0..=0x1F | 0x7F..=0x9F => None,
        _ => Some(0x0100_0000 + cp),
    }
}

/// The keysym for a key: the special keys by code first (Enter types `\n` but is
/// Return), then the character it typed.
pub fn from_key(code: u8, typed: Option<char>) -> Option<u32> {
    special(code).or_else(|| typed.and_then(from_char))
}
