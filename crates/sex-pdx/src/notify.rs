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
/// A Date due notice. Bell replies after policy denial or after Shell handles
/// its last forwarded chunk. Ordinary posts remain one-way.
pub const OP_BELL_POST_CONFIRMED: u64 = 0xCF;
/// Bell's BOOT domain. Check the reply's kernel-stamped sender, not its text.
pub const BELL_PD: u32 = 10;
/// Bell -> Shell: one chunk of an accepted notification.
/// arg0 = `Chunk::word()` with `id` and `sender` set, arg1/arg2 as above.
pub const OP_NOTIFICATION: u64 = 0x2D8;
/// Shell -> Display: one chunk of the panel notice (same chunk form).
/// A notice with no text clears it.
pub const OP_PANEL_NOTICE: u64 = 0x2D9;
/// Bell -> Shell: full event identity before confirmed text chunks.
/// arg0 = Bell event ID, arg1 = Date ticket, arg2 = original sender PD.
pub const OP_NOTIFICATION_BEGIN: u64 = 0x2DB;
/// Shell -> Bell: handling of that exact Bell run's event.
/// Arguments are `Presentation::ack_words`.
pub const OP_NOTIFICATION_HANDLED: u64 = 0x2DC;

/// Most text bytes in a notification: eight chunks.
pub const TEXT_MAX: usize = 128;
pub const CHUNK_BYTES: usize = 16;
pub const CHUNKS_MAX: usize = TEXT_MAX / CHUNK_BYTES;

pub const URGENCY_LOW: u8 = 0;
pub const URGENCY_NORMAL: u8 = 1;
pub const URGENCY_CRITICAL: u8 = 2;

const RECEIPT_MARK: u64 = 0xB311;
/// Shell queued the panel/history event, a person hid or muted it, or the
/// attempt cannot be confirmed yet. Date retries only the last case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Disposition { Retry = 0, Accepted = 1, Muted = 2 }
impl Disposition {
    /// User mute settles one due notice without showing it. A transient Bell
    /// refusal leaves that due minute pending.
    pub fn settles_due(self) -> bool { self != Self::Retry }
}

/// One confirmed notice in flight. Shell echoes the kernel-stamped Bell task,
/// so an ACK queued across Bell restart cannot settle a new run's event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Presentation {
    pub event_id: u64,
    pub ticket: u32,
    pub sender: u32,
    pub bell_task: u32,
}
impl Presentation {
    pub fn new(event_id: u64, ticket: u32, sender: u32, bell_task: u32) -> Option<Self> {
        (event_id != 0 && sender != 0 && sender <= u16::MAX as u32 && bell_task != 0)
            .then_some(Self { event_id, ticket, sender, bell_task })
    }
    pub fn matches_text(self, text: &Text, bell_task: u32) -> bool {
        self.bell_task == bell_task && self.event_id as u16 == text.id && self.sender as u16 == text.sender
    }
    pub fn ack_words(self, disposition: Disposition) -> (u64, u64, u64) {
        (self.event_id, (self.bell_task as u64) << 32 | self.ticket as u64,
            (self.sender as u64) << 8 | disposition as u64)
    }
    pub fn decode_ack(a: u64, b: u64, c: u64) -> Option<(Self, Disposition)> {
        if c >> 40 != 0 { return None; }
        let disposition = match c as u8 { 0 => Disposition::Retry, 1 => Disposition::Accepted,
            2 => Disposition::Muted, _ => return None };
        let sender = (c >> 8) as u32;
        let p = Self::new(a, b as u32, sender, (b >> 32) as u32)?;
        Some((p, disposition))
    }
    pub fn accepts_ack(self, a: u64, b: u64, c: u64) -> Option<Disposition> {
        Self::decode_ack(a, b, c).and_then(|(identity, disposition)|
            (identity == self).then_some(disposition))
    }
    /// Bell must also keep the original sender run alive before replying to
    /// its PD: a new Date task can reuse both that PD and its first ticket.
    pub fn accepts_ack_for_live_tasks(self, a: u64, b: u64, c: u64,
        sender_task: u32, live_bell_task: u32, live_sender_task: u32) -> Option<Disposition> {
        (sender_task != 0 && self.bell_task == live_bell_task
            && sender_task == live_sender_task).then(|| self.accepts_ack(a, b, c)).flatten()
    }
}

pub fn receipt(ticket: u32, disposition: Disposition) -> u64 {
    RECEIPT_MARK << 48 | (ticket as u64) << 2 | disposition as u64
}
pub fn decode_receipt(word: u64) -> Option<(u32, Disposition)> {
    if word >> 48 != RECEIPT_MARK || word >> 34 & 0x3fff != 0 { return None; }
    let disposition = match word & 3 { 0 => Disposition::Retry, 1 => Disposition::Accepted,
        2 => Disposition::Muted, _ => return None };
    Some(((word >> 2) as u32, disposition))
}

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
        if c.index != self.next || (c.index != 0 && (c.len as u8 != self.text.len || c.id != self.text.id
            || c.sender != self.text.sender || c.urgency != self.text.urgency)) {
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

/// Compose the same bounded text for one-way and confirmed Bell posts.
pub fn compose(summary: &[u8], body: &[u8]) -> Option<([u8; TEXT_MAX], usize)> {
    let mut text = [0u8; TEXT_MAX];
    let mut n = 0;
    for &b in summary.iter().filter(|b| **b != b'\n') { if n < TEXT_MAX { text[n] = b; n += 1; } }
    if !body.is_empty() && n < TEXT_MAX { text[n] = b'\n'; n += 1; }
    for &b in body { if n < TEXT_MAX { text[n] = b; n += 1; } }
    (n != 0).then_some((text, n))
}

/// Posts a notification to Bell. `summary` is one line; `body` may be empty.
/// Fire and forget: a muted sender's notification is dropped by Bell.
pub fn post(summary: &[u8], body: &[u8], urgency: u8) -> bool {
    let Some((text, n)) = compose(summary, body) else { return false };
    let ok = chunks(&text[..n], urgency, 0, 0).all(|(w, a, b)| crate::pdx_call(crate::SLOT_BELL, OP_BELL_POST, w, a, b).0 == 0);
    ok
}

#[cfg(test)]
mod receipt_tests {
    use super::*;
    #[test]
    fn receipt_is_bound_to_ticket_and_policy_result() {
        for ticket in [1, 0x1234_5678, u32::MAX] {
            for outcome in [Disposition::Accepted, Disposition::Muted, Disposition::Retry] {
                assert_eq!(decode_receipt(receipt(ticket, outcome)), Some((ticket, outcome)));
            }
        }
        assert_eq!(decode_receipt(0), None);
        assert_eq!(decode_receipt(receipt(7, Disposition::Accepted) | (1 << 40)), None);
        assert_eq!(decode_receipt(receipt(7, Disposition::Retry) | 3), None);
        assert!(Disposition::Accepted.settles_due());
        assert!(Disposition::Muted.settles_due());
        assert!(!Disposition::Retry.settles_due());
    }
    #[test]
    fn assembler_rejects_changed_ticket_between_chunks() {
        let mut assembler = Assembler::new();
        let mut calls = chunks(b"0123456789abcdefg", URGENCY_CRITICAL, 7, 9);
        let first = calls.next().unwrap();
        let last = calls.next().unwrap();
        assert_eq!(assembler.push(first.0, first.1, first.2), None);
        assert_eq!(assembler.push(last.0 ^ (1 << 48), last.1, last.2), None);
        assert!(!assembler.pending());
    }

    #[test]
    fn presentation_ack_rejects_stale_bell_task_and_changed_event() {
        let p = Presentation::new(0x1_0002, 0x1234_5678, 44, 901).unwrap();
        let (a, b, c) = p.ack_words(Disposition::Accepted);
        assert_eq!(Presentation::decode_ack(a, b, c), Some((p, Disposition::Accepted)));
        assert_eq!(p.accepts_ack(a, b, c), Some(Disposition::Accepted));
        assert_eq!(p.accepts_ack_for_live_tasks(a, b, c, 801, 901, 801), Some(Disposition::Accepted));
        assert_eq!(p.accepts_ack_for_live_tasks(a, b, c, 801, 902, 801), None);
        assert_eq!(p.accepts_ack_for_live_tasks(a, b, c, 801, 901, 802), None);
        assert_eq!(Presentation::new(a, p.ticket, p.sender, 902).unwrap().accepts_ack(a, b, c), None);
        assert_eq!(Presentation::new(a + 1, p.ticket, p.sender, p.bell_task).unwrap().accepts_ack(a, b, c), None);
        assert_eq!(Presentation::new(a, p.ticket + 1, p.sender, p.bell_task).unwrap().accepts_ack(a, b, c), None);
        assert_eq!(Presentation::decode_ack(a, b, c | (1 << 40)), None);
        assert_eq!(Presentation::decode_ack(a, b, c | 3), None);
        let text = Text { id: 2, sender: 44, ..Text::EMPTY };
        assert!(p.matches_text(&text, 901));
        assert!(!p.matches_text(&text, 902));
        assert!(!p.matches_text(&Text { id: 3, ..text }, 901));
    }
}
