//! Desktop notifications with text.
//!
//! A program holding the Bell capability (`SLOT_BELL`) posts a notification
//! as a few `OP_BELL_POST` calls of 16 text bytes each. Bell checks the
//! sender (mute and per-program policy, by the kernel's caller PD, never a
//! name the sender claims), keeps it, and forwards it to the Shell as
//! `OP_NOTIFICATION` calls in the same chunk form, with Bell's own id and the
//! sender's PD. The Shell shows it briefly in the panel and keeps a history.
//! The text is "summary\nbody" in printable ASCII; anything else becomes '?'.

/// Program -> Bell: one chunk of a notification.
/// arg0 = `Chunk::word()`, arg1/arg2 = 16 text bytes, little-endian.
pub const OP_BELL_POST: u64 = 0xCC;
/// Bell -> Shell: one chunk of an accepted notification.
/// arg0 = `Chunk::word()` with `id` and `sender` set, arg1/arg2 as above.
pub const OP_NOTIFICATION: u64 = 0x2D8;
/// Shell -> Display: one chunk of the panel notice (same chunk form).
/// A notice with no text clears it.
pub const OP_PANEL_NOTICE: u64 = 0x2D9;

/// Most text bytes in a notification: eight chunks.
pub const TEXT_MAX: usize = 128;
pub const CHUNK_BYTES: usize = 16;
pub const CHUNKS_MAX: usize = TEXT_MAX / CHUNK_BYTES;

pub const URGENCY_LOW: u8 = 0;
pub const URGENCY_NORMAL: u8 = 1;
pub const URGENCY_CRITICAL: u8 = 2;

/// The header word of a chunk.
/// bits 0..8 index, 8 last, 9..11 urgency, 16..32 total length,
/// 32..48 Bell's notification id (0 from programs), 48..64 sender PD (0 from programs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chunk { pub index: u8, pub last: bool, pub urgency: u8, pub len: u16, pub id: u16, pub sender: u16 }
impl Chunk {
    pub fn word(&self) -> u64 {
        self.index as u64 | (self.last as u64) << 8 | ((self.urgency & 3) as u64) << 9
            | (self.len as u64) << 16 | (self.id as u64) << 32 | (self.sender as u64) << 48
    }
    pub fn decode(word: u64) -> Option<Self> {
        let c = Chunk { index: word as u8, last: word >> 8 & 1 == 1, urgency: (word >> 9 & 3) as u8,
            len: (word >> 16) as u16, id: (word >> 32) as u16, sender: (word >> 48) as u16 };
        let fits = (c.index as usize) < CHUNKS_MAX && (c.len as usize) <= TEXT_MAX
            && c.urgency <= URGENCY_CRITICAL && word >> 11 & 0x1F == 0;
        // The last chunk is the one that holds the final byte.
        let last_index = (c.len as usize).saturating_sub(1) / CHUNK_BYTES;
        (fits && c.last == (c.index as usize == last_index)).then_some(c)
    }
}

/// Text cut into calls: (header word, arg1, arg2) for each chunk.
pub fn chunks(text: &[u8], urgency: u8, id: u16, sender: u16) -> impl Iterator<Item = (u64, u64, u64)> + '_ {
    let text = &text[..text.len().min(TEXT_MAX)];
    let count = text.len().div_ceil(CHUNK_BYTES).max(1);
    (0..count).map(move |i| {
        let mut b = [0u8; CHUNK_BYTES];
        let part = &text[(i * CHUNK_BYTES).min(text.len())..((i + 1) * CHUNK_BYTES).min(text.len())];
        b[..part.len()].copy_from_slice(part);
        let head = Chunk { index: i as u8, last: i + 1 == count, urgency: urgency.min(URGENCY_CRITICAL), len: text.len() as u16, id, sender };
        (head.word(), u64::from_le_bytes(b[..8].try_into().unwrap()), u64::from_le_bytes(b[8..].try_into().unwrap()))
    })
}

/// One finished notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Text { pub bytes: [u8; TEXT_MAX], pub len: u8, pub urgency: u8, pub id: u16, pub sender: u16 }
impl Text {
    pub const EMPTY: Self = Self { bytes: [0; TEXT_MAX], len: 0, urgency: 0, id: 0, sender: 0 };
    pub fn as_bytes(&self) -> &[u8] { &self.bytes[..self.len as usize] }
    /// The first line.
    pub fn summary(&self) -> &[u8] { self.as_bytes().split(|b| *b == b'\n').next().unwrap_or(&[]) }
    /// Everything after the first line, or nothing.
    pub fn body(&self) -> &[u8] {
        let t = self.as_bytes();
        t.iter().position(|b| *b == b'\n').map_or(&[][..], |i| &t[i + 1..])
    }
}

/// Puts one sender's chunks back together. Chunks arrive in order on one
/// channel; a chunk out of order, or a new first chunk, starts over.
#[derive(Clone, Copy)]
pub struct Assembler { text: Text, next: u8 }
impl Assembler {
    pub const fn new() -> Self { Self { text: Text::EMPTY, next: 0 } }
    pub fn pending(&self) -> bool { self.next != 0 }
    /// Returns the notification once its last chunk arrives.
    pub fn push(&mut self, word: u64, a: u64, b: u64) -> Option<Text> {
        let c = Chunk::decode(word)?;
        if c.index == 0 { self.next = 0; }
        if c.index != self.next || (c.index != 0 && (c.len as u8 != self.text.len || c.id != self.text.id)) {
            self.next = 0;
            return None;
        }
        if c.index == 0 { self.text = Text { bytes: [0; TEXT_MAX], len: c.len as u8, urgency: c.urgency, id: c.id, sender: c.sender }; }
        let at = c.index as usize * CHUNK_BYTES;
        let mut raw = [0u8; CHUNK_BYTES];
        raw[..8].copy_from_slice(&a.to_le_bytes());
        raw[8..].copy_from_slice(&b.to_le_bytes());
        let end = (at + CHUNK_BYTES).min(self.text.len as usize);
        for (dst, src) in self.text.bytes[at..end].iter_mut().zip(raw) {
            *dst = if src == b'\n' || (32..=126).contains(&src) { src } else { b'?' };
        }
        if c.last { self.next = 0; Some(self.text) } else { self.next += 1; None }
    }
}

/// Posts a notification to Bell. `summary` is one line; `body` may be empty.
/// Fire and forget: a muted sender's notification is dropped by Bell.
pub fn post(summary: &[u8], body: &[u8], urgency: u8) -> bool {
    let mut text = [0u8; TEXT_MAX];
    let mut n = 0;
    for &b in summary.iter().filter(|b| **b != b'\n') { if n < TEXT_MAX { text[n] = b; n += 1; } }
    if !body.is_empty() && n < TEXT_MAX { text[n] = b'\n'; n += 1; }
    for &b in body { if n < TEXT_MAX { text[n] = b; n += 1; } }
    if n == 0 { return false; }
    let ok = chunks(&text[..n], urgency, 0, 0).all(|(w, a, b)| crate::pdx_call(crate::SLOT_BELL, OP_BELL_POST, w, a, b).0 == 0);
    ok
}
