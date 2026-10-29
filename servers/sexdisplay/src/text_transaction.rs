//! Display-private staging. Nothing in a partial transaction changes visible text.
use sex_pdx::text_batch::{hash, Request, CAPACITY};
#[derive(Clone, Copy)]
pub struct Stage {
    owner: u32,
    sid: u64,
    sequence: u32,
    len: usize,
    color: u32,
    coverage: u64,
    sealed: bool,
    poisoned: bool,
    reply_pending: bool,
    bytes: [u8; CAPACITY],
}
impl Stage {
    pub const EMPTY: Self = Self {
        owner: 0,
        sid: 0,
        sequence: 0,
        len: 0,
        color: 0,
        coverage: 0,
        sealed: false,
        poisoned: false,
        reply_pending: false,
        bytes: [0; CAPACITY],
    };
    pub fn diagnostic(&self)->(u32,u64,u32,usize,u64,bool,bool){(self.owner,self.sid,self.sequence,self.len,self.coverage,self.sealed,self.poisoned)}
    pub fn reset(&mut self) {
        *self = Self::EMPTY;
    }
    pub fn retire(&mut self, owner: u32) {
        if self.owner == owner {
            *self = Self::EMPTY;
        }
    }
    /// One retained completion attempt per display pass. Actual reply transport
    /// preserves sender identity. Legacy syscall 29 collapses Full and denial
    /// into nonzero status, so retain every nonzero result until admitted or
    /// authoritative surface/app cleanup cancels it. Never replay the mutation.
    pub fn poll_reply(&mut self, mut reply: impl FnMut(u32, u64) -> u64) {
        if !self.reply_pending {
            return;
        }
        let Ok(surface) = u16::try_from(self.sid) else {
            self.reply_pending = false;
            return;
        };
        let Some(value) = (sex_pdx::text_batch::Applied {
            surface,
            sequence: self.sequence,
        })
        .encode() else {
            self.reply_pending = false;
            return;
        };
        if reply(self.owner, value) == 0 {
            self.reply_pending = false;
        }
    }
    /// Caller identity and live surface ownership must already be attested by display.
    /// A new (owner,sid) binding cannot consume a previous binding's partial bytes.
    pub fn receive(
        &mut self,
        owner: u32,
        sid: u64,
        request: Request,
    ) -> Option<(usize, u32, &[u8; CAPACITY])> {
        if owner == 0 || sid == 0 {
            return None;
        }
        match request {
            Request::Begin {
                sequence,
                len,
                color,
            } => {
                if sequence == 0 || len > CAPACITY {
                    return None;
                }
                if self.owner != owner || self.sid != sid {
                    *self = Self::EMPTY;
                }
                if sequence < self.sequence {
                    return None;
                }
                if sequence == self.sequence {
                    if self.len != len || self.color != color {
                        self.poisoned = true;
                    }
                    return None;
                }
                *self = Self {
                    owner,
                    sid,
                    sequence,
                    len,
                    color,
                    ..Self::EMPTY
                };
            }
            Request::Chunk {
                sequence,
                offset,
                count,
                bytes,
            } => {
                if (self.owner, self.sid, self.sequence) != (owner, sid, sequence)
                    || self.sealed
                    || self.poisoned
                {
                    return None;
                }
                if offset >= self.len
                    || offset % 8 != 0
                    || count != (self.len - offset).min(8)
                    || bytes[count..].iter().any(|&v| v != 0)
                    || bytes[..count]
                        .iter()
                        .any(|&v| v != b'\n' && !(32..=126).contains(&v))
                {
                    self.poisoned = true;
                    return None;
                }
                let bit = 1u64 << (offset / 8);
                if self.coverage & bit != 0 && self.bytes[offset..offset + count] != bytes[..count]
                {
                    self.poisoned = true;
                    return None;
                }
                self.bytes[offset..offset + count].copy_from_slice(&bytes[..count]);
                self.coverage |= bit;
            }
            Request::Commit {
                sequence,
                hash: expected,
            } => {
                if (self.owner, self.sid, self.sequence) != (owner, sid, sequence)
                    || self.sealed
                    || self.poisoned
                {
                    return None;
                }
                let chunks = (self.len + 7) / 8;
                let complete = if chunks == 64 {
                    u64::MAX
                } else {
                    (1u64 << chunks) - 1
                };
                if self.coverage != complete || hash(&self.bytes[..self.len]) != expected {
                    self.poisoned = true;
                    return None;
                }
                self.sealed = true;
                self.reply_pending = true;
                return Some((self.len, self.color, &self.bytes));
            }
        }
        None
    }
}
