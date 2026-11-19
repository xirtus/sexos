// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Shell to the Whisper voice (`apps/whisper-voice`): text to speak.
//!
//! The screen reader lives in Shell; the voice is a confined program that holds
//! nothing but its own sound ring. Shell sends it text as a run of `OP_SPEAK`
//! messages, each carrying up to [`PIECE_BYTES`] bytes of UTF-8 in `arg1`/`arg2`
//! (little-endian) and a header in `arg0`:
//!
//! - bits 0..8: bytes of text in this piece (0..=16);
//! - bit 8 [`FIRST`]: a new utterance. It stops what the voice is saying and drops
//!   the sound already queued (a screen reader speaks the newest focus, not a backlog);
//! - bit 9 [`LAST`]: the text is whole; speak it;
//! - bits 16..32: speaking rate in words a minute (read from the FIRST piece; 0 means
//!   the voice's default).
//!
//! A single piece with FIRST|LAST and no text is "stop talking". Text past
//! [`MAX_TEXT`] bytes is dropped. The voice only accepts these from Shell's PD; a
//! piece that does not continue the utterance being gathered is ignored.
//!
//! Opcode 0x2DE (0x2DD is Display's magnifier and colour filters, access_view).

pub const OP_SPEAK: u64 = 0x2DE;
pub const PIECE_BYTES: usize = 16;
pub const MAX_TEXT: usize = 1024;
pub const FIRST: u64 = 1 << 8;
pub const LAST: u64 = 1 << 9;

/// The `(arg0, arg1, arg2)` of each message that sends `text` at `wpm` (0 = default).
/// Empty text gives the one "stop" piece.
pub fn pieces(text: &[u8], wpm: u16) -> impl Iterator<Item = [u64; 3]> + '_ {
    let text = &text[..text.len().min(MAX_TEXT)];
    let count = text.len().div_ceil(PIECE_BYTES).max(1);
    (0..count).map(move |i| {
        let chunk = &text[(i * PIECE_BYTES).min(text.len())..((i + 1) * PIECE_BYTES).min(text.len())];
        let mut bytes = [0u8; PIECE_BYTES];
        bytes[..chunk.len()].copy_from_slice(chunk);
        let mut head = chunk.len() as u64 | (wpm as u64) << 16;
        if i == 0 { head |= FIRST; }
        if i + 1 == count { head |= LAST; }
        [head, u64::from_le_bytes(bytes[..8].try_into().unwrap()), u64::from_le_bytes(bytes[8..].try_into().unwrap())]
    })
}

/// What a completed run of pieces asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request<'a> {
    Speak { text: &'a [u8], wpm: u16 },
    Stop,
}

/// The voice's side: gathers pieces into an utterance.
pub struct Gather {
    text: [u8; MAX_TEXT],
    len: usize,
    wpm: u16,
    open: bool,
}

impl Default for Gather { fn default() -> Self { Self::new() } }

impl Gather {
    pub const fn new() -> Gather { Gather { text: [0; MAX_TEXT], len: 0, wpm: 0, open: false } }
    /// True when `arg0` starts a new utterance (the voice stops talking at once).
    pub const fn starts(arg0: u64) -> bool { arg0 & FIRST != 0 }
    /// Takes one piece; returns the request once the utterance is whole.
    pub fn push(&mut self, arg0: u64, arg1: u64, arg2: u64) -> Option<Request<'_>> {
        if arg0 & FIRST != 0 {
            self.len = 0;
            self.wpm = (arg0 >> 16) as u16;
            self.open = true;
        } else if !self.open {
            return None;
        }
        let n = ((arg0 & 0xFF) as usize).min(PIECE_BYTES);
        let mut bytes = [0u8; PIECE_BYTES];
        bytes[..8].copy_from_slice(&arg1.to_le_bytes());
        bytes[8..].copy_from_slice(&arg2.to_le_bytes());
        let room = MAX_TEXT - self.len;
        let take = n.min(room);
        self.text[self.len..self.len + take].copy_from_slice(&bytes[..take]);
        self.len += take;
        if arg0 & LAST == 0 { return None; }
        self.open = false;
        if self.len == 0 { return Some(Request::Stop); }
        Some(Request::Speak { text: &self.text[..self.len], wpm: self.wpm })
    }
}
