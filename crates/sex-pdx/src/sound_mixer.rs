// SPDX-License-Identifier: GPL-3.0-only
//! The person's say over sound, kept by Bell: a master volume and mute over everything
//! Bell plays, and a volume and mute for each program row's stream on top of the
//! volume the program asks for in its own ring (`sound_ring::VOLUME`).
//!
//! Only the desktop asks: Shell (volume keys), SilkBar (reads), and Spindle rows (the
//! `volume` command). A program cannot turn others down or itself up past the person's
//! setting. Volumes are percent, 0..=100; master follows a squared curve so equal key
//! steps sound like equal steps.
//!
//! rdi = op OP_MIXER, rsi = action, rdx = a, r10 = b. Bell replies with a value.

/// Shell, SilkBar or Spindle to Bell.
pub const OP_MIXER: u64 = 0xCA;

/// Master volume to `a` (0..=100). Replies with `state`.
pub const SET_MASTER: u64 = 1;
/// Master mute: `a` 0 off, 1 on, 2 toggle. Replies with `state`.
pub const MUTE_MASTER: u64 = 2;
/// Master volume by `a` as i64 (a volume key's step); unmutes. Replies with `state`
/// and plays a short tick at the new loudness.
pub const STEP_MASTER: u64 = 3;
/// Row `a`'s stream: volume `b & 0xFF` (0..=100), muted when `b & MUTED`.
/// Replies with that row's `stream` word.
pub const SET_STREAM: u64 = 4;
/// Replies with `state`.
pub const STATE: u64 = 5;
/// Replies with row `a`'s `stream` word.
pub const STREAM: u64 = 6;

pub const MUTED: u64 = 1 << 8;
/// Every mixer reply carries this in its top byte, so a caller can tell it from
/// Bell's other replies (a notification's event id) that may arrive in between.
pub const TAG: u64 = 0x4D << 56;
pub const fn is_reply(word: u64) -> bool { word >> 56 == TAG >> 56 }
/// Reply when the caller may not ask, or the request is malformed.
pub const REFUSED: u64 = TAG | 1 << 55;

/// Master state word: volume in bits 0..8, muted bit 8, sound card present bit 9.
pub const fn state(volume: u8, muted: bool, card: bool) -> u64 { TAG | volume as u64 | (muted as u64) << 8 | (card as u64) << 9 }
pub const fn state_volume(word: u64) -> u8 { word as u8 }
pub const fn state_muted(word: u64) -> bool { word & 1 << 8 != 0 }
pub const fn state_card(word: u64) -> bool { word & 1 << 9 != 0 }

/// One row's stream word: the person's volume (bits 0..8), muted (bit 8), playing
/// (bit 9), the volume the program asks for (bits 16..24), its PD (bits 32..56), and
/// `TAG`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stream { pub volume: u8, pub muted: bool, pub playing: bool, pub asked: u8, pub pd: u32 }
impl Stream {
    pub const fn word(self) -> u64 {
        TAG | self.volume as u64 | (self.muted as u64) << 8 | (self.playing as u64) << 9 | (self.asked as u64) << 16 | ((self.pd & 0xFF_FFFF) as u64) << 32
    }
    pub const fn of(word: u64) -> Stream {
        Stream { volume: word as u8, muted: word & 1 << 8 != 0, playing: word & 1 << 9 != 0, asked: (word >> 16) as u8, pd: ((word >> 32) & 0xFF_FFFF) as u32 }
    }
}

/// Gain in 1/65536 for a master volume: squared, so 50% is a quarter of the amplitude.
pub const fn master_gain(volume: u8) -> u32 {
    let v = if volume > 100 { 100 } else { volume } as u32;
    v * v * 65536 / 10_000
}
