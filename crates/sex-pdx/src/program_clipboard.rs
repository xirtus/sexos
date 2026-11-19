// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The system clipboard for ordinary programs: text, through Shell, which keeps the
//! clipboard (and its Super+V history) for every program and document view.
//!
//! - **Copy.** The program sends Shell `OP_CLIP` `COPY` (length), the text as `DATA`
//!   pieces of 16 bytes, then `END` with the text's [`hash`]. Shell answers
//!   `OP_CLIPPED` `DONE` (status).
//! - **Paste.** The program sends `OP_CLIP` `PASTE`. Shell answers `OP_CLIPPED`
//!   `BEGIN` (length), the `DATA` pieces and `END` (hash), or `DONE` with `EMPTY` or a
//!   refusal.
//!
//! Shell takes either only from the program whose window has focus and that the person
//! has just used (a key or a click in the last [`TOUCH_TICKS`]), so a program cannot
//! read or replace the clipboard behind the person's back. What a program copied goes
//! when it ends, as a document view's does.
//!
//! arg0 = phase << 56 | offset << 16 | count (DATA), or length (COPY, BEGIN), or status
//! (DONE); arg1, arg2 = the piece's bytes (DATA), or the hash (END).

/// Program -> Shell.
pub const OP_CLIP: u64 = 0x60;
/// Shell -> program.
pub const OP_CLIPPED: u64 = 0x61;

pub const COPY: u8 = 1;
pub const PASTE: u8 = 2;
pub const BEGIN: u8 = 3;
pub const DATA: u8 = 4;
pub const END: u8 = 5;
pub const DONE: u8 = 6;

/// `DONE` statuses.
pub const OK: u8 = 0;
pub const EMPTY: u8 = 1;
/// Not the focused program, or not just used.
pub const DENIED: u8 = 2;
/// Not text, a wrong length or hash, or pieces out of order.
pub const INVALID: u8 = 3;
/// Another transfer is under way; try again.
pub const BUSY: u8 = 4;

/// The most the clipboard holds (`clipboard::MAX_BYTES`).
pub const MAX_BYTES: usize = crate::clipboard::MAX_BYTES;
pub const PIECE: usize = 16;
/// How recent the person's last key or click in the program must be (ms).
pub const TOUCH_TICKS: u64 = 3_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    Copy { len: usize },
    Paste,
    Begin { len: usize },
    Data { offset: usize, count: usize, bytes: [u8; PIECE] },
    End { hash: u64 },
    Done { status: u8 },
}

impl Message {
    pub fn wire(&self) -> [u64; 3] {
        let head = |phase: u8, low: u64| (phase as u64) << 56 | low;
        match *self {
            Message::Copy { len } => [head(COPY, len as u64), 0, 0],
            Message::Paste => [head(PASTE, 0), 0, 0],
            Message::Begin { len } => [head(BEGIN, len as u64), 0, 0],
            Message::Data { offset, count, bytes } => [head(DATA, (offset as u64) << 16 | count as u64),
                u64::from_le_bytes(bytes[..8].try_into().unwrap()), u64::from_le_bytes(bytes[8..].try_into().unwrap())],
            Message::End { hash } => [head(END, 0), hash, 0],
            Message::Done { status } => [head(DONE, status as u64), 0, 0],
        }
    }
    pub fn decode(w: [u64; 3]) -> Option<Self> {
        let low = w[0] & 0x00FF_FFFF_FFFF_FFFF;
        let length = |low: u64| (1..=MAX_BYTES as u64).contains(&low).then_some(low as usize);
        Some(match (w[0] >> 56) as u8 {
            COPY if w[1] == 0 && w[2] == 0 => Message::Copy { len: length(low)? },
            PASTE if low == 0 && w[1] == 0 && w[2] == 0 => Message::Paste,
            BEGIN if w[1] == 0 && w[2] == 0 => Message::Begin { len: length(low)? },
            DATA => {
                let (offset, count) = ((low >> 16) as usize, (low & 0xFFFF) as usize);
                if offset % PIECE != 0 || offset >= MAX_BYTES || !(1..=PIECE).contains(&count) || offset + count > MAX_BYTES { return None; }
                let mut bytes = [0u8; PIECE];
                bytes[..8].copy_from_slice(&w[1].to_le_bytes());
                bytes[8..].copy_from_slice(&w[2].to_le_bytes());
                if bytes[count..].iter().any(|b| *b != 0) { return None; }
                Message::Data { offset, count, bytes }
            }
            END if low == 0 && w[2] == 0 => Message::End { hash: w[1] },
            DONE if low <= BUSY as u64 && w[1] == 0 && w[2] == 0 => Message::Done { status: low as u8 },
            _ => return None,
        })
    }
}

/// The pieces of a text, in order.
pub fn pieces(text: &[u8]) -> impl Iterator<Item = Message> + '_ {
    text.chunks(PIECE).enumerate().map(|(i, c)| {
        let mut bytes = [0u8; PIECE];
        bytes[..c.len()].copy_from_slice(c);
        Message::Data { offset: i * PIECE, count: c.len(), bytes }
    })
}

/// The hash `END` carries (`clipboard::hash`).
pub fn hash(bytes: &[u8]) -> u64 { crate::clipboard::hash(bytes) }

/// A text being put together from pieces.
pub struct Text { len: usize, have: usize, bytes: [u8; MAX_BYTES] }

impl Text {
    pub const fn new() -> Self { Self { len: 0, have: 0, bytes: [0; MAX_BYTES] } }
    /// Starts over for a text of `len` bytes.
    pub fn begin(&mut self, len: usize) { self.erase(); self.len = len.min(MAX_BYTES); }
    /// Takes one piece; pieces must come in order and stay inside the length.
    pub fn take(&mut self, offset: usize, count: usize, bytes: &[u8; PIECE]) -> bool {
        if self.len == 0 || offset != self.have || offset + count > self.len || (count < PIECE && offset + count != self.len) { return false; }
        self.bytes[offset..offset + count].copy_from_slice(&bytes[..count]);
        self.have += count;
        true
    }
    /// The whole text, if every piece came, it hashes to `hash` and it is UTF-8.
    pub fn finish(&self, hash: u64) -> Option<&[u8]> {
        let b = &self.bytes[..self.have];
        (self.len != 0 && self.have == self.len && self::hash(b) == hash && core::str::from_utf8(b).is_ok()).then_some(b)
    }
    pub fn erase(&mut self) {
        for b in &mut self.bytes[..self.have] { unsafe { core::ptr::write_volatile(b, 0) } }
        self.len = 0; self.have = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn messages_round_trip_and_bad_words_are_refused() {
        let mut bytes = [0u8; PIECE]; bytes[..5].copy_from_slice(b"hello");
        for m in [Message::Copy { len: 1 }, Message::Copy { len: MAX_BYTES }, Message::Paste, Message::Begin { len: 7 },
                  Message::Data { offset: 32, count: 5, bytes }, Message::End { hash: 0xdead_beef }, Message::Done { status: BUSY }] {
            assert_eq!(Message::decode(m.wire()), Some(m));
        }
        assert_eq!(Message::decode(Message::Copy { len: 0 }.wire()), None);
        assert_eq!(Message::decode(Message::Copy { len: MAX_BYTES + 1 }.wire()), None);
        assert_eq!(Message::decode([(DATA as u64) << 56 | 5, 0, 1 << 8]), None);
        assert_eq!(Message::decode([(DATA as u64) << 56 | 3 << 16 | 1, 0, 0]), None);
        assert_eq!(Message::decode([(DONE as u64) << 56 | 9, 0, 0]), None);
        assert_eq!(Message::decode([0x77 << 56, 0, 0]), None);
    }
    #[test]
    fn text_needs_every_piece_in_order_and_its_hash() {
        let s = "Grüße, clipboard! ".repeat(3);
        let mut t = Text::new(); t.begin(s.len());
        for m in pieces(s.as_bytes()) {
            let Message::Data { offset, count, bytes } = m else { panic!() };
            assert!(t.take(offset, count, &bytes));
        }
        assert_eq!(t.finish(hash(s.as_bytes())), Some(s.as_bytes()));
        assert_eq!(t.finish(hash(s.as_bytes()) ^ 1), None);
        let mut t = Text::new(); t.begin(s.len());
        let all: Vec<_> = pieces(s.as_bytes()).collect();
        let Message::Data { offset, count, bytes } = all[1] else { panic!() };
        assert!(!t.take(offset, count, &bytes));
        let mut t = Text::new(); t.begin(2);
        assert!(t.take(0, 2, &[0xC3, 0x28, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(t.finish(hash(&[0xC3, 0x28])), None);
    }
}
