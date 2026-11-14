// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Voyeur search provider protocol (register-only PDX words).
//!
//! Voyeur is the system search. A provider is a program that answers a
//! query with results taken only from what it already holds (Linen's file
//! index, Kaleidoscope's history, a password store). Privacy rule: the
//! Shell sends a provider the query text and nothing else: never another
//! provider's results, never which window is focused, never what the user
//! chose unless it chose that provider's own result. The Shell scores
//! every result itself against the query, adds its own frecency and merges
//! all providers (crate `voyeur`); a provider's own score is a hint the
//! Shell caps.
//!
//! Flow, all on routes the Shell already holds for the program:
//!
//! ```text
//! provider -> Shell  REGISTER   name, kinds it answers with
//! Shell -> provider  REGISTERED provider id, status
//! Shell -> provider  QUERY      ticket, query text (1..=3 parts)
//! provider -> Shell  RESULT     ticket, index, kind, action, hint, key, lengths
//! provider -> Shell  TEXT       ticket, index, field, 16 bytes of title/subtitle
//! provider -> Shell  DONE       ticket, count
//! Shell -> provider  CANCEL     ticket (a newer query replaced it)
//! Shell -> provider  ACTIVATE   ticket, index, action, key (the user chose it)
//! ```
//!
//! Opcodes 0x3C0..=0x3C7 (no other user of 0x3C0..=0x3CF in the tree).
//! This file has no crate imports so host tests can include it by path.

pub const REGISTER: u64 = 0x3C0;
pub const REGISTERED: u64 = 0x3C1;
pub const QUERY: u64 = 0x3C2;
pub const RESULT: u64 = 0x3C3;
pub const TEXT: u64 = 0x3C4;
pub const DONE: u64 = 0x3C5;
pub const ACTIVATE: u64 = 0x3C6;
pub const CANCEL: u64 = 0x3C7;

/// Protocol version carried in REGISTER.
pub const VERSION: u8 = 1;
/// Longest query, in bytes (three 16-byte parts).
pub const QUERY_MAX: usize = 48;
pub const TITLE_MAX: usize = 48;
pub const SUBTITLE_MAX: usize = 64;
pub const NAME_MAX: usize = 16;
/// Most results one provider gives one query.
pub const RESULTS_MAX: usize = 8;
/// Every packet one full answer can take: header + text chunks per result, and DONE.
pub const ANSWER_PACKETS: usize = RESULTS_MAX * (1 + TITLE_MAX / 16 + SUBTITLE_MAX / 16) + 1;

/// Words a provider sends or the Shell sends: requests from providers,
/// requests to them.
pub fn provider_opcode(op: u64) -> bool { matches!(op, REGISTER | RESULT | TEXT | DONE) }
pub fn shell_opcode(op: u64) -> bool { matches!(op, REGISTERED | QUERY | ACTIVATE | CANCEL) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Packet { pub opcode: u64, pub args: [u64; 3] }

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status { Ok = 0, Denied = 1, Full = 2, Invalid = 3 }
impl Status {
    pub fn decode(v: u8) -> Option<Self> { match v { 0 => Some(Self::Ok), 1 => Some(Self::Denied), 2 => Some(Self::Full), 3 => Some(Self::Invalid), _ => None } }
}

fn words16(b: &[u8]) -> (u64, u64) {
    let mut x = [0u8; 16];
    x[..b.len()].copy_from_slice(b);
    (u64::from_le_bytes(x[..8].try_into().unwrap()), u64::from_le_bytes(x[8..].try_into().unwrap()))
}
fn bytes16(a: u64, b: u64) -> [u8; 16] {
    let mut x = [0u8; 16];
    x[..8].copy_from_slice(&a.to_le_bytes());
    x[8..].copy_from_slice(&b.to_le_bytes());
    x
}
fn text_ok(b: &[u8]) -> bool { core::str::from_utf8(b).is_ok() && !b.iter().any(|&c| c < 0x20 || c == 0x7F) }

/// A provider's registration: its name (shown under its results) and the
/// kinds (`voyeur::Kind` wire values, bit per value) it answers with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Registration { name: [u8; NAME_MAX], len: u8, pub kinds: u16 }
impl Registration {
    pub fn new(name: &str, kinds: u16) -> Option<Self> {
        let b = name.as_bytes();
        if b.is_empty() || b.len() > NAME_MAX || !b.iter().all(|c| c.is_ascii_graphic() || *c == b' ') || kinds == 0 { return None; }
        let mut n = [0u8; NAME_MAX];
        n[..b.len()].copy_from_slice(b);
        Some(Self { name: n, len: b.len() as u8, kinds })
    }
    pub fn name(&self) -> &str { core::str::from_utf8(&self.name[..self.len as usize]).unwrap_or("") }
    pub fn wire(&self) -> Packet {
        let (a, b) = words16(&self.name[..self.len as usize]);
        Packet { opcode: REGISTER, args: [a, b, self.kinds as u64 | (VERSION as u64) << 56] }
    }
    pub fn decode(p: Packet) -> Option<Self> {
        let [a, b, c] = p.args;
        if p.opcode != REGISTER || c >> 56 != VERSION as u64 || c & 0x00FF_FFFF_FFFF_0000 != 0 { return None; }
        let raw = bytes16(a, b);
        let len = raw.iter().position(|&x| x == 0).unwrap_or(NAME_MAX);
        if raw[len..].iter().any(|&x| x != 0) { return None; }
        Self::new(core::str::from_utf8(&raw[..len]).ok()?, c as u16)
    }
}

pub fn registered(id: u8, status: Status) -> Packet { Packet { opcode: REGISTERED, args: [id as u64 | (status as u64) << 8, 0, 0] } }
pub fn decode_registered(p: Packet) -> Option<(u8, Status)> {
    let [a, b, c] = p.args;
    (p.opcode == REGISTERED && a >> 16 == 0 && b == 0 && c == 0).then_some(())?;
    Some((a as u8, Status::decode((a >> 8) as u8)?))
}

/// The packets that carry `query` under `ticket`; returns how many.
pub fn query_packets(ticket: u32, query: &str, out: &mut [Packet; 3]) -> usize {
    let b = query.as_bytes();
    if ticket == 0 || b.is_empty() || b.len() > QUERY_MAX { return 0; }
    let parts = b.len().div_ceil(16);
    for (i, chunk) in b.chunks(16).enumerate() {
        let (x, y) = words16(chunk);
        out[i] = Packet { opcode: QUERY, args: [ticket as u64 | (b.len() as u64) << 32 | (i as u64) << 40 | (parts as u64) << 48, x, y] };
    }
    parts
}

/// Provider side: puts the query back together from its parts.
#[derive(Clone, Copy, Debug)]
pub struct QueryReader { ticket: u32, len: u8, got: u8, buf: [u8; QUERY_MAX] }
impl QueryReader {
    pub const fn new() -> Self { Self { ticket: 0, len: 0, got: 0, buf: [0; QUERY_MAX] } }
    /// Some(ticket) once the whole query has arrived. A part of a new
    /// ticket starts over; parts must come in order.
    pub fn accept(&mut self, p: Packet) -> Option<u32> {
        let [a, x, y] = p.args;
        if p.opcode != QUERY || a >> 56 != 0 { return None; }
        let (ticket, len, part, parts) = (a as u32, (a >> 32) as u8 as usize, (a >> 40) as u8, (a >> 48) as u8);
        if ticket == 0 || len == 0 || len > QUERY_MAX || parts as usize != len.div_ceil(16) || part >= parts { return None; }
        if part == 0 { *self = Self { ticket, len: len as u8, got: 0, buf: [0; QUERY_MAX] }; }
        if self.ticket != ticket || self.len as usize != len || self.got != part { return None; }
        let at = part as usize * 16;
        let take = (len - at).min(16);
        let chunk = bytes16(x, y);
        if chunk[take..].iter().any(|&c| c != 0) { return None; }
        self.buf[at..at + take].copy_from_slice(&chunk[..take]);
        self.got += 1;
        if self.got != parts { return None; }
        if !text_ok(&self.buf[..len]) { self.ticket = 0; return None; }
        Some(ticket)
    }
    pub fn query(&self) -> &str {
        if self.got == 0 || (self.got as usize) < (self.len as usize).div_ceil(16) { return ""; }
        core::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("")
    }
}
impl Default for QueryReader { fn default() -> Self { Self::new() } }

/// One result. `key` is the provider's own handle for it, given back in
/// ACTIVATE; the Shell never interprets it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub kind: u8,
    pub action: u8,
    pub hint: u16,
    pub key: u64,
    title: [u8; TITLE_MAX],
    title_len: u8,
    subtitle: [u8; SUBTITLE_MAX],
    subtitle_len: u8,
}
impl Record {
    pub const EMPTY: Self = Self { kind: 0, action: 0, hint: 0, key: 0, title: [0; TITLE_MAX], title_len: 0, subtitle: [0; SUBTITLE_MAX], subtitle_len: 0 };
    /// None when a text is too long, has control characters, or the title is empty.
    pub fn new(kind: u8, action: u8, hint: u16, key: u64, title: &str, subtitle: &str) -> Option<Self> {
        let (t, s) = (title.as_bytes(), subtitle.as_bytes());
        if t.is_empty() || t.len() > TITLE_MAX || s.len() > SUBTITLE_MAX || !text_ok(t) || !text_ok(s) || kind == 0 || action == 0 { return None; }
        let mut r = Self { kind, action, hint, key, title_len: t.len() as u8, subtitle_len: s.len() as u8, ..Self::EMPTY };
        r.title[..t.len()].copy_from_slice(t);
        r.subtitle[..s.len()].copy_from_slice(s);
        Some(r)
    }
    pub fn title(&self) -> &str { core::str::from_utf8(&self.title[..self.title_len as usize]).unwrap_or("") }
    pub fn subtitle(&self) -> &str { core::str::from_utf8(&self.subtitle[..self.subtitle_len as usize]).unwrap_or("") }
}

fn header(ticket: u32, index: usize, r: &Record) -> Packet {
    Packet { opcode: RESULT, args: [
        ticket as u64 | (index as u64) << 32 | (r.kind as u64) << 40 | (r.action as u64) << 48,
        r.hint as u64 | (r.title_len as u64) << 16 | (r.subtitle_len as u64) << 24,
        r.key,
    ] }
}
fn text_packet(ticket: u32, index: usize, field: u8, offset: usize, chunk: &[u8]) -> Packet {
    let (x, y) = words16(chunk);
    Packet { opcode: TEXT, args: [ticket as u64 | (index as u64) << 32 | (field as u64) << 40 | (offset as u64) << 48 | (chunk.len() as u64) << 56, x, y] }
}

/// Provider side: every packet of an answer, DONE last; returns how many.
/// Results past [`RESULTS_MAX`] are left out.
pub fn answer_packets(ticket: u32, records: &[Record], out: &mut [Packet; ANSWER_PACKETS]) -> usize {
    let mut n = 0;
    let count = records.len().min(RESULTS_MAX);
    for (i, r) in records[..count].iter().enumerate() {
        out[n] = header(ticket, i, r); n += 1;
        for (field, bytes) in [(0u8, &r.title[..r.title_len as usize]), (1, &r.subtitle[..r.subtitle_len as usize])] {
            for (k, chunk) in bytes.chunks(16).enumerate() { out[n] = text_packet(ticket, i, field, k * 16, chunk); n += 1; }
        }
    }
    out[n] = Packet { opcode: DONE, args: [ticket as u64 | (count as u64) << 32, 0, 0] };
    n + 1
}

pub fn cancel(ticket: u32) -> Packet { Packet { opcode: CANCEL, args: [ticket as u64, 0, 0] } }
pub fn activate(ticket: u32, index: u8, action: u8, key: u64) -> Packet {
    Packet { opcode: ACTIVATE, args: [ticket as u64 | (index as u64) << 32 | (action as u64) << 40, key, 0] }
}
/// (ticket, index, action, key)
pub fn decode_activate(p: Packet) -> Option<(u32, u8, u8, u64)> {
    let [a, key, c] = p.args;
    (p.opcode == ACTIVATE && a >> 48 == 0 && c == 0 && a as u32 != 0).then_some((a as u32, (a >> 32) as u8, (a >> 40) as u8, key))
}
pub fn decode_cancel(p: Packet) -> Option<u32> {
    let [a, b, c] = p.args;
    (p.opcode == CANCEL && a >> 32 == 0 && a != 0 && b == 0 && c == 0).then_some(a as u32)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collect { Pending, Done(usize) }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectError { NotOurs, StaleTicket, Malformed, OutOfOrder, Incomplete }

/// Shell side: one provider's answer to the current ticket. Every word is
/// checked; text must arrive in order; DONE must name exactly the results
/// received, each complete. Anything else drops the whole answer.
#[derive(Clone, Copy, Debug)]
pub struct Collector {
    ticket: u32,
    count: usize,
    got: [(u8, u8); RESULTS_MAX],
    records: [Record; RESULTS_MAX],
    done: bool,
}
impl Collector {
    pub const fn new() -> Self { Self { ticket: 0, count: 0, got: [(0, 0); RESULTS_MAX], records: [Record::EMPTY; RESULTS_MAX], done: false } }
    /// A new query was sent under `ticket`: older answers no longer count.
    pub fn begin(&mut self, ticket: u32) { *self = Self::new(); self.ticket = ticket; }
    pub fn ticket(&self) -> u32 { self.ticket }
    pub fn done(&self) -> bool { self.done }
    /// The complete results, only once DONE arrived.
    pub fn records(&self) -> &[Record] { if self.done { &self.records[..self.count] } else { &[] } }
    fn fail(&mut self, e: CollectError) -> Result<Collect, CollectError> { let t = self.ticket; self.begin(t); Err(e) }
    pub fn accept(&mut self, p: Packet) -> Result<Collect, CollectError> {
        if !matches!(p.opcode, RESULT | TEXT | DONE) { return Err(CollectError::NotOurs); }
        let [a, b, c] = p.args;
        if self.ticket == 0 || a as u32 != self.ticket { return Err(CollectError::StaleTicket); }
        if self.done { return self.fail(CollectError::OutOfOrder); }
        match p.opcode {
            RESULT => {
                let (index, kind, action) = ((a >> 32) as u8 as usize, (a >> 40) as u8, (a >> 48) as u8);
                let (hint, tl, sl) = (b as u16, (b >> 16) as u8 as usize, (b >> 24) as u8 as usize);
                if a >> 56 != 0 || b >> 32 != 0 || kind == 0 || action == 0 || tl == 0 || tl > TITLE_MAX || sl > SUBTITLE_MAX {
                    return self.fail(CollectError::Malformed);
                }
                if index != self.count || index >= RESULTS_MAX { return self.fail(CollectError::OutOfOrder); }
                if self.count > 0 && !self.complete(self.count - 1) { return self.fail(CollectError::Incomplete); }
                self.records[index] = Record { kind, action, hint, key: c, title_len: tl as u8, subtitle_len: sl as u8, ..Record::EMPTY };
                self.got[index] = (0, 0);
                self.count += 1;
                Ok(Collect::Pending)
            }
            TEXT => {
                let (index, field, offset, count) = ((a >> 32) as u8 as usize, (a >> 40) as u8, (a >> 48) as u8 as usize, (a >> 56) as usize);
                if self.count == 0 || index != self.count - 1 || field > 1 || count == 0 || count > 16 { return self.fail(CollectError::Malformed); }
                let r = &mut self.records[index];
                let (want, got, buf): (usize, &mut u8, &mut [u8]) = if field == 0 {
                    (r.title_len as usize, &mut self.got[index].0, &mut r.title[..])
                } else {
                    if self.got[index].0 as usize != r.title_len as usize { return self.fail(CollectError::OutOfOrder); }
                    (r.subtitle_len as usize, &mut self.got[index].1, &mut r.subtitle[..])
                };
                if offset != *got as usize || offset + count > want || (offset + count != want && count != 16) { return self.fail(CollectError::OutOfOrder); }
                let chunk = bytes16(b, c);
                if chunk[count..].iter().any(|&x| x != 0) { return self.fail(CollectError::Malformed); }
                buf[offset..offset + count].copy_from_slice(&chunk[..count]);
                *got = (offset + count) as u8;
                Ok(Collect::Pending)
            }
            _ => {
                let n = (a >> 32) as usize;
                if b != 0 || c != 0 || n != self.count || (0..self.count).any(|i| !self.complete(i)) { return self.fail(CollectError::Incomplete); }
                let ok = self.records[..n].iter().all(|r| text_ok(&r.title[..r.title_len as usize]) && text_ok(&r.subtitle[..r.subtitle_len as usize]));
                if !ok { return self.fail(CollectError::Malformed); }
                self.done = true;
                Ok(Collect::Done(n))
            }
        }
    }
    fn complete(&self, i: usize) -> bool {
        let r = &self.records[i];
        self.got[i] == (r.title_len, r.subtitle_len)
    }
}
impl Default for Collector { fn default() -> Self { Self::new() } }
