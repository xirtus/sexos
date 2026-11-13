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
    /// The master state, for the sound panel.
    Master,
    /// Row k's stream word, for the sound panel.
    Row(u8),
}

const ASKS: usize = 32;
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

/// The sound panel at the top of the notifications window (Super+N): the master
/// volume and each program that has a sound ring, with its own volume. Up/Down pick a
/// program, Left/Right change its volume by 5%, M mutes it. Bell keeps the volumes;
/// this only shows Bell's answers and asks for changes.
pub const ROWS: usize = 16;
pub const PANEL_STEP: u8 = 5;
pub struct SoundPanel { pub master: u64, pub rows: [u64; ROWS], pub selected: usize, pub open: bool }
impl SoundPanel {
    pub const fn new() -> Self { Self { master: 0, rows: [0; ROWS], selected: 0, open: false } }
    /// Rows with a program on them: (row, volume, muted, playing, pd).
    pub fn shown(&self) -> impl Iterator<Item = (usize, u8, bool, bool, u32)> + '_ {
        self.rows.iter().enumerate().filter(|(_, w)| is_mixer_answer(**w) && !refused(**w) && stream_pd(**w) != 0)
            .map(|(k, w)| (k, (*w as u8).min(100), w & 1 << 8 != 0, w & 1 << 9 != 0, stream_pd(*w)))
    }
    pub fn answer(&mut self, ask: Ask, word: u64) {
        match ask {
            Ask::Master if !refused(word) => self.master = word,
            Ask::Row(k) if (k as usize) < ROWS => self.rows[k as usize] = if refused(word) { 0 } else { word },
            _ => {}
        }
        let n = self.shown().count();
        if self.selected >= n { self.selected = n.saturating_sub(1); }
    }
    /// A key while the panel's window has the focus: `Some((row, b))` to send to Bell
    /// as `SET_STREAM` (volume | MUTED), `None` when there is nothing to change.
    /// `handled` is false for keys that are not the panel's.
    pub fn key(&mut self, code: u8) -> (bool, Option<(u8, u64)>) {
        let n = self.shown().count();
        match code {
            0x48 => { self.selected = self.selected.saturating_sub(1); (true, None) }
            0x50 => { if self.selected + 1 < n { self.selected += 1; } (true, None) }
            0x4B | 0x4D | 0x32 => {
                let Some((row, volume, muted, _, _)) = self.shown().nth(self.selected) else { return (true, None) };
                let (volume, muted) = match code {
                    0x4B => (volume.saturating_sub(PANEL_STEP), muted),
                    0x4D => ((volume + PANEL_STEP).min(100), muted),
                    _ => (volume, !muted),
                };
                (true, Some((row as u8, volume as u64 | if muted { 1 << 8 } else { 0 })))
            }
            _ => (false, None),
        }
    }
    /// The panel as text: one line per program, the chosen one marked.
    pub fn text(&self, out: &mut [u8]) -> usize {
        let mut n = 0;
        let cap = out.len();
        let mut put = |b: &[u8], n: &mut usize| for &c in b { if *n < cap { out[*n] = c; *n += 1; } };
        let num = |v: u32, buf: &mut [u8; 10]| -> usize { let mut i = 10; let mut v = v; loop { i -= 1; buf[i] = b'0' + (v % 10) as u8; v /= 10; if v == 0 { break i; } } };
        let mut d = [0u8; 10];
        put(b"SOUND  master ", &mut n);
        let i = num((self.master as u8).min(100) as u32, &mut d); put(&d[i..], &mut n);
        put(b"%", &mut n);
        if self.master & 1 << 8 != 0 { put(b" muted", &mut n); }
        put(b"\n", &mut n);
        let mut any = false;
        for (idx, (_, volume, muted, playing, pd)) in self.shown().enumerate() {
            any = true;
            put(if idx == self.selected { b"> " } else { b"  " }, &mut n);
            put(b"Program ", &mut n);
            let i = num(pd, &mut d); put(&d[i..], &mut n);
            put(b"  ", &mut n);
            let i = num(volume as u32, &mut d); put(&d[i..], &mut n);
            put(b"%", &mut n);
            if muted { put(b" muted", &mut n); }
            if playing { put(b" playing", &mut n); }
            put(b"\n", &mut n);
        }
        if any { put(b"Up/Down pick, Left/Right volume, M mute\n", &mut n); } else { put(b"  No program has sound\n", &mut n); }
        put(b"\n", &mut n);
        n
    }
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
    fn sound_panel_lists_programs_and_changes_the_chosen_one() {
        let mut p = SoundPanel::new();
        p.answer(Ask::Master, TAG | 80 | 1 << 9);
        p.answer(Ask::Row(0), TAG | 42u64 << 32 | 1 << 9 | 100);
        p.answer(Ask::Row(1), REFUSED);
        p.answer(Ask::Row(3), TAG | 43u64 << 32 | 1 << 8 | 60);
        let mut out = [0u8; 256];
        let n = p.text(&mut out);
        assert_eq!(std::str::from_utf8(&out[..n]).unwrap(),
            "SOUND  master 80%\n> Program 42  100% playing\n  Program 43  60% muted\nUp/Down pick, Left/Right volume, M mute\n\n");
        assert_eq!(p.key(0x4B), (true, Some((0, 95))));
        assert_eq!(p.key(0x50), (true, None));
        assert_eq!(p.key(0x4D), (true, Some((3, 65 | 1 << 8))));
        assert_eq!(p.key(0x32), (true, Some((3, 60))));
        assert_eq!(p.key(0x50), (true, None));
        assert_eq!(p.selected, 1);
        assert_eq!(p.key(0x24), (false, None));
    }
    #[test]
    fn stream_word_names_the_pd() {
        assert_eq!(stream_pd(TAG | 40u64 << 32 | 1 << 9 | 100), 40);
        assert!(is_mixer_answer(REFUSED) && refused(REFUSED));
        assert!(!is_mixer_answer(7));
        assert!(is_player_key(0x78) && !is_player_key(0x75));
    }
}
