// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Drag and drop between programs: a file or a piece of text, carried by Shell from the
//! window the person pressed in to the window they let go over.
//!
//! 1. The source, while the person holds the left button down in its window, sends
//!    Shell `OP_DRAG` `BEGIN` (kind, length; for a file, the ticket Files gave it for
//!    `fs::OFFER` of that path), then `DATA` pieces of the payload, 16 bytes each.
//!    Shell takes a drag only from the run holding the pointer.
//! 2. When the button comes up over another program's window, Shell hands a file on
//!    first: `OP_GIVE` to Files moves the offered path into the target's grants, as if
//!    the person had picked it there. Then the target gets `OP_DROP` `BEGIN` (kind,
//!    length, where), the `DATA` pieces, and `END`.
//! 3. The source hears `OP_DROP` `DONE` with arg1 = the target row, or 0 if nothing took
//!    the drop (let go over no program, or `CANCEL`led).
//!
//! arg0 = phase << 56 | kind << 48 | offset << 16 | count (DATA) or length (BEGIN);
//! arg1, arg2 = the piece's bytes (DATA), ticket (drag BEGIN), x | y << 16 (drop BEGIN).

/// Program -> Shell.
pub const OP_DRAG: u64 = 0x5B;
/// Shell -> program.
pub const OP_DROP: u64 = 0x5C;
/// Shell -> Files: arg0 = offer ticket, arg1 = source row << 32 | target row,
/// arg2 = `hash` of the path Shell carries. No reply.
pub const OP_GIVE: u64 = 0x5D;

pub const BEGIN: u8 = 1;
pub const DATA: u8 = 2;
pub const END: u8 = 3;
pub const CANCEL: u8 = 4;
pub const DONE: u8 = 5;

/// A path, `/absolute/plain`, that the target may open.
pub const FILE: u8 = 1;
pub const TEXT: u8 = 2;

/// The most a drop carries.
pub const MAX_BYTES: usize = 512;
pub const PIECE: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Message {
    Begin { kind: u8, len: usize, extra: u64 },
    Data { offset: usize, count: usize, bytes: [u8; PIECE] },
    End,
    Cancel,
    Done { target: u32 },
}

impl Message {
    pub fn wire(&self) -> [u64; 3] {
        let head = |phase: u8, kind: u8, low: u64| (phase as u64) << 56 | (kind as u64) << 48 | low;
        match *self {
            Message::Begin { kind, len, extra } => [head(BEGIN, kind, len as u64), extra, 0],
            Message::Data { offset, count, bytes } => [head(DATA, 0, (offset as u64) << 16 | count as u64),
                u64::from_le_bytes(bytes[..8].try_into().unwrap()), u64::from_le_bytes(bytes[8..].try_into().unwrap())],
            Message::End => [head(END, 0, 0), 0, 0],
            Message::Cancel => [head(CANCEL, 0, 0), 0, 0],
            Message::Done { target } => [head(DONE, 0, 0), target as u64, 0],
        }
    }
    pub fn decode(w: [u64; 3]) -> Option<Self> {
        let kind = (w[0] >> 48) as u8;
        let low = w[0] & 0xFFFF_FFFF_FFFF;
        Some(match (w[0] >> 56) as u8 {
            BEGIN if matches!(kind, FILE | TEXT) && (1..=MAX_BYTES as u64).contains(&low) => Message::Begin { kind, len: low as usize, extra: w[1] },
            DATA => {
                let (offset, count) = ((low >> 16) as usize, (low & 0xFFFF) as usize);
                if kind != 0 || offset % PIECE != 0 || offset >= MAX_BYTES || !(1..=PIECE).contains(&count) || offset + count > MAX_BYTES { return None; }
                let mut bytes = [0u8; PIECE];
                bytes[..8].copy_from_slice(&w[1].to_le_bytes());
                bytes[8..].copy_from_slice(&w[2].to_le_bytes());
                if bytes[count..].iter().any(|b| *b != 0) { return None; }
                Message::Data { offset, count, bytes }
            }
            END => Message::End,
            CANCEL => Message::Cancel,
            DONE => Message::Done { target: w[1] as u32 },
            _ => return None,
        })
    }
}

/// The pieces of a payload, in order.
pub fn pieces(payload: &[u8]) -> impl Iterator<Item = Message> + '_ {
    payload.chunks(PIECE).enumerate().map(|(i, c)| {
        let mut bytes = [0u8; PIECE];
        bytes[..c.len()].copy_from_slice(c);
        Message::Data { offset: i * PIECE, count: c.len(), bytes }
    })
}

/// Where the drop landed, for `Begin::extra`.
pub fn at(x: i16, y: i16) -> u64 { x as u16 as u64 | (y as u16 as u64) << 16 }
pub fn position(extra: u64) -> (i16, i16) { (extra as u16 as i16, (extra >> 16) as u16 as i16) }

/// The payload being put together from pieces.
#[derive(Clone, Copy)]
pub struct Payload { pub kind: u8, pub len: usize, pub extra: u64, have: usize, pub bytes: [u8; MAX_BYTES] }

impl Payload {
    pub const fn new(kind: u8, len: usize, extra: u64) -> Self { Self { kind, len, extra, have: 0, bytes: [0; MAX_BYTES] } }
    /// Takes one piece; pieces must come in order and stay inside the length.
    pub fn take(&mut self, offset: usize, count: usize, bytes: &[u8; PIECE]) -> bool {
        if offset != self.have || offset + count > self.len || (count < PIECE && offset + count != self.len) { return false; }
        self.bytes[offset..offset + count].copy_from_slice(&bytes[..count]);
        self.have += count;
        true
    }
    pub fn complete(&self) -> bool { self.have == self.len }
    pub fn as_bytes(&self) -> &[u8] { &self.bytes[..self.have] }
    /// A file payload is one plain absolute path; text is valid UTF-8.
    pub fn valid(&self) -> bool {
        let b = self.as_bytes();
        self.complete() && match self.kind {
            FILE => b.first() == Some(&b'/') && !b.iter().any(|c| *c == b' ' || *c < 0x20) && !b.split(|c| *c == b'/').any(|p| p == b".." || p == b"."),
            TEXT => core::str::from_utf8(b).is_ok(),
            _ => false,
        }
    }
}

/// FNV-1a of the path, so Files gives only the path Shell carries.
pub fn hash(bytes: &[u8]) -> u64 { bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3)) }
