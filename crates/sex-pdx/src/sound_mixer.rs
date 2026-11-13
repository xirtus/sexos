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
/// Whether row `a`'s program may hear the microphone: `b` 1 allows, 0 stops it at
/// once. Only the person decides; it lapses when the program ends. Replies with that
/// row's `stream` word.
pub const MICROPHONE: u64 = 7;
/// Replies with the `outputs` word: how many outputs the card has, of what kind,
/// and which play. One word holds them all.
pub const OUTPUTS: u64 = 8;
/// Play to the outputs in mask `a` (bit k is output k) and silence the rest.
/// Replies with the `outputs` word, or `REFUSED` when `a` names none. `a` = `AUTO`
/// goes back to the default outputs, following the headphone jack.
pub const SET_OUTPUTS: u64 = 10;
pub const AUTO: u64 = 1 << 8;
/// Which program a media key (play/pause, stop, next, previous) is for: program `a`
/// (the focused one's PD, 0 for none) when it has played sound since it started, else
/// the program that most recently started playing. Replies with that row's `stream`
/// word (its PD inside), or `REFUSED` when no program has played. Shell asks; Bell
/// only names the program, and Shell delivers the key to it as an ordinary key.
pub const LAST_PLAYER: u64 = 11;
/// Set in the outputs word while Bell follows the headphone jack.
pub const OUTPUTS_AUTO: u64 = 1 << 12;

/// An output's kind.
pub const LINE_OUT: u8 = 0;
pub const SPEAKER: u8 = 1;
pub const HEADPHONES: u8 = 2;
pub const OTHER: u8 = 14;
/// The one output of a virtual sound card.
pub const VIRTUAL: u8 = 15;
pub const MAX_OUTPUTS: usize = 8;
/// The outputs word: which play (bits 0..8), how many there are (bits 8..12), and
/// output k's kind in bits 16+4k..20+4k.
pub const fn outputs(on: u8, kinds: &[u8]) -> u64 {
    let mut w = TAG | on as u64 | ((kinds.len() & 0xF) as u64) << 8;
    let mut k = 0;
    while k < kinds.len() && k < MAX_OUTPUTS { w |= ((kinds[k] & 0xF) as u64) << (16 + 4 * k); k += 1; }
    w
}
pub const fn outputs_on(word: u64) -> u8 { word as u8 }
pub const fn outputs_count(word: u64) -> u8 { (word >> 8 & 0xF) as u8 }
pub const fn output_kind(word: u64, k: usize) -> u8 { (word >> (16 + 4 * k) & 0xF) as u8 }

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
/// (bit 9), may hear the microphone (bit 10), hearing it now (bit 11), the volume the program asks for (bits 16..24), its PD (bits 32..56), and
/// `TAG`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stream { pub volume: u8, pub muted: bool, pub playing: bool, pub asked: u8, pub pd: u32, pub mic_allowed: bool, pub listening: bool }
impl Stream {
    pub const fn word(self) -> u64 {
        TAG | self.volume as u64 | (self.muted as u64) << 8 | (self.playing as u64) << 9
            | (self.mic_allowed as u64) << 10 | (self.listening as u64) << 11 | (self.asked as u64) << 16 | ((self.pd & 0xFF_FFFF) as u64) << 32
    }
    pub const fn of(word: u64) -> Stream {
        Stream { volume: word as u8, muted: word & 1 << 8 != 0, playing: word & 1 << 9 != 0, asked: (word >> 16) as u8, pd: ((word >> 32) & 0xFF_FFFF) as u32,
            mic_allowed: word & 1 << 10 != 0, listening: word & 1 << 11 != 0 }
    }
}

/// Gain in 1/65536 for a master volume: squared, so 50% is a quarter of the amplitude.
pub const fn master_gain(volume: u8) -> u32 {
    let v = if volume > 100 { 100 } else { volume } as u32;
    v * v * 65536 / 10_000
}
