// SPDX-License-Identifier: GPL-3.0-only
//! Media keys and the volume on-screen display.
//!
//! Volume and mute keys go to Bell (see `desktop_keys`); Bell answers each with the
//! master state, and that answer is shown in the panel for a moment: the OSD. The
//! player keys (play/pause, stop, next, previous) are for whichever program plays:
//! Shell asks Bell which (`sound_mixer::LAST_PLAYER`, naming the focused program) and,
//! when Bell answers, hands the key to that program as an ordinary key. A program
//! learns nothing it did not before: only that a key arrived.
//!
//! Bell answers in the order it was asked, so a small FIFO of what each answer is for
//! is enough. Answers that are not mixer answers are someone else's.

/// Player key codes, as `sex_pdx::app_key` names them (literals so this file builds alone).
pub const KEY_PLAY_PAUSE: u8 = 0x78;
pub const KEY_STOP: u8 = 0x7A;
pub const KEY_NEXT: u8 = 0x7C;
pub const KEY_PREVIOUS: u8 = 0x7F;

pub const fn is_player_key(code: u8) -> bool {
    matches!(code, KEY_PLAY_PAUSE | KEY_STOP | KEY_NEXT | KEY_PREVIOUS)
}

/// How long the OSD stays, in ticks (ms).
pub const OSD_TICKS: u64 = 1_500;

/// What a mixer answer from Bell is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    /// A volume or mute key: show the new state.
    Volume,
    /// A player key with this code: deliver it to the program Bell names.
    Player(u8),
}

const ASKS: usize = 8;
pub struct Asks { q: [Option<Ask>; ASKS], head: usize, len: usize }
impl Asks {
    pub const fn new() -> Self { Self { q: [None; ASKS], head: 0, len: 0 } }
    /// False when full: the key is dropped rather than an answer misread.
    pub fn push(&mut self, ask: Ask) -> bool {
        if self.len == ASKS { return false; }
        self.q[(self.head + self.len) % ASKS] = Some(ask);
        self.len += 1;
        true
    }
    pub fn pop(&mut self) -> Option<Ask> {
        if self.len == 0 { return None; }
        let a = self.q[self.head].take();
        self.head = (self.head + 1) % ASKS;
        self.len -= 1;
        a
    }
    pub fn pending(&self) -> bool { self.len != 0 }
}

/// Mixer answer tag and fields (`sex_pdx::sound_mixer`), kept literal for host tests.
const TAG: u64 = 0x4D << 56;
const REFUSED: u64 = TAG | 1 << 55;
pub const fn is_mixer_answer(word: u64) -> bool { word >> 56 == TAG >> 56 }
pub const fn refused(word: u64) -> bool { word == REFUSED }
/// The PD inside a `stream` word.
pub const fn stream_pd(word: u64) -> u32 { ((word >> 32) & 0xFF_FFFF) as u32 }

/// The OSD line for a master `state` word: "Volume 55%  |||||||||||.........",
/// one bar per 5%, or "Muted" with the volume it will come back at.
pub fn osd_text(state: u64, out: &mut [u8; 48]) -> usize {
    let volume = (state as u8).min(100);
    let muted = state & 1 << 8 != 0;
    let card = state & 1 << 9 != 0;
    let mut n = 0;
    let mut put = |b: &[u8], n: &mut usize| for &c in b { if *n < out.len() { out[*n] = c; *n += 1; } };
    put(if muted { b"Muted " } else { b"Volume " }, &mut n);
    let mut d = [0u8; 3];
    let mut i = 3;
    let mut v = volume;
    loop { i -= 1; d[i] = b'0' + v % 10; v /= 10; if v == 0 { break; } }
    put(&d[i..], &mut n);
    put(b"%  ", &mut n);
    for k in 0..20u8 { put(if !muted && k < volume / 5 { b"|" } else { b"." }, &mut n); }
    if !card { put(b"  no sound card", &mut n); }
    n
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    fn text(state: u64) -> std::string::String {
        let mut out = [0u8; 48];
        let n = osd_text(state, &mut out);
        std::string::String::from_utf8(out[..n].to_vec()).unwrap()
    }
    #[test]
    fn osd_shows_volume_bars_and_mute() {
        assert_eq!(text(TAG | 55 | 1 << 9), "Volume 55%  |||||||||||.........");
        assert_eq!(text(TAG | 100 | 1 << 9), "Volume 100%  ||||||||||||||||||||");
        assert_eq!(text(TAG | 0 | 1 << 9), "Volume 0%  ....................");
        assert_eq!(text(TAG | 50 | 1 << 8 | 1 << 9), "Muted 50%  ....................");
        assert!(text(TAG | 50).ends_with("no sound card"));
    }
    #[test]
    fn answers_come_back_in_order() {
        let mut a = Asks::new();
        assert!(a.push(Ask::Volume));
        assert!(a.push(Ask::Player(KEY_PLAY_PAUSE)));
        assert_eq!(a.pop(), Some(Ask::Volume));
        assert_eq!(a.pop(), Some(Ask::Player(KEY_PLAY_PAUSE)));
        assert_eq!(a.pop(), None);
        for _ in 0..ASKS { assert!(a.push(Ask::Volume)); }
        assert!(!a.push(Ask::Volume));
    }
    #[test]
    fn stream_word_names_the_pd() {
        assert_eq!(stream_pd(TAG | 40u64 << 32 | 1 << 9 | 100), 40);
        assert!(is_mixer_answer(REFUSED) && refused(REFUSED));
        assert!(!is_mixer_answer(7));
        assert!(is_player_key(0x78) && !is_player_key(0x75));
    }
}
