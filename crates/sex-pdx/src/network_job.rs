// SPDX-License-Identifier: GPL-3.0-only
//! Bounded outbound-job data carried over BOOT's existing browser network route.
//! Tickets and sequence numbers are data, never capabilities or authority.
pub const OP: u64 = 0x20C;
pub const MAX_BYTES: usize = 4096;
pub const TURN: usize = 256;
pub const MAX_SEQ: u32 = 0xFF_FFFF;
pub const MAX_TICKET: u32 = 17_535;
pub const BEGIN: u8 = 1;
pub const WRITE: u8 = 2;
pub const START: u8 = 3;
pub const STATUS: u8 = 4;
pub const READ: u8 = 5;
pub const CANCEL: u8 = 6;
pub const RELEASE: u8 = 7;
/// DNS generation selects service provenance, never new endpoint authority.
pub const BEGIN_RESOLVED: u8 = 8;
pub const INVALID: u32 = 1;
pub const DENIED: u32 = 2;
pub const STALE: u32 = 3;
pub const BUSY: u32 = 4;
pub const OFFLINE: u32 = 5;
pub const TIMEOUT: u32 = 6;
pub const IO: u32 = 7;
pub const EXHAUSTED: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub seq: u32,
    pub kind: u8,
    pub offset: u16,
    pub length: u16,
    pub ticket: u32,
    pub a: u64,
}
impl Request {
    pub const fn encode(self) -> [u64; 3] {
        [(self.seq as u64) | ((self.kind as u64) << 24)
            | ((self.offset as u64) << 32) | ((self.length as u64) << 48),
            self.ticket as u64, self.a]
    }
    pub fn decode(args: [u64; 3]) -> Result<Self, u32> {
        let r = Self { seq: (args[0] & MAX_SEQ as u64) as u32,
            kind: (args[0] >> 24) as u8, offset: (args[0] >> 32) as u16,
            length: (args[0] >> 48) as u16, ticket: args[1] as u32, a: args[2] };
        if args[1] > u32::MAX as u64 || r.seq == 0 || !(BEGIN..=BEGIN_RESOLVED).contains(&r.kind) {
            return Err(INVALID);
        }
        Ok(r)
    }
    pub fn valid(self) -> bool {
        if self.seq == 0 || self.seq > MAX_SEQ { return false; }
        match self.kind {
            BEGIN => self.ticket == 0 && self.offset == 0 && self.a == 0
                && self.length != 0 && self.length as usize <= MAX_BYTES,
            BEGIN_RESOLVED => self.ticket == 0 && self.offset == 0
                && self.a != 0 && self.a <= MAX_TICKET as u64
                && self.length != 0 && self.length as usize <= MAX_BYTES,
            WRITE => self.length != 0 && self.length <= 8
                && (self.length == 8 || self.a >> (self.length * 8) == 0),
            READ => self.a == 0 && self.length != 0 && self.length <= 4,
            START | STATUS | CANCEL | RELEASE => self.offset == 0 && self.length == 0 && self.a == 0,
            _ => false,
        }
    }
}
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { Idle = 0, Staging = 1, Connecting = 2, Sending = 3,
    Receiving = 4, Complete = 5, Failed = 6, Cancelled = 7 }
impl State {
    pub fn decode(value: u8) -> Option<Self> {
        Some(match value { 0 => Self::Idle, 1 => Self::Staging, 2 => Self::Connecting,
            3 => Self::Sending, 4 => Self::Receiving, 5 => Self::Complete,
            6 => Self::Failed, 7 => Self::Cancelled, _ => return None })
    }
}
pub const fn status(state: State, response_len: usize) -> u32 {
    ((state as u32) << 24) | (response_len as u32 & MAX_SEQ)
}
pub fn decode_status(value: u32) -> Option<(State, usize)> {
    Some((State::decode((value >> 24) as u8)?, (value & MAX_SEQ) as usize))
}
pub const fn reply(seq: u32, value: u32) -> u64 {
    (0x78u64 << 56) | ((seq as u64 & MAX_SEQ as u64) << 32) | value as u64
}
pub const fn error(seq: u32, code: u32) -> u64 {
    (0xE8u64 << 56) | ((seq as u64 & MAX_SEQ as u64) << 32) | code as u64
}
/// Exact-sequence decoding; success values may contain arbitrary response bytes.
pub fn decode_reply(word: u64, seq: u32) -> Result<u32, u32> {
    if seq == 0 || seq > MAX_SEQ || (word >> 32) as u32 & MAX_SEQ != seq { return Err(INVALID); }
    match word >> 56 { 0x78 => Ok(word as u32), 0xE8 => Err(word as u32), _ => Err(INVALID) }
}
