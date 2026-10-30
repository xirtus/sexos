//! Display-private staging. Nothing in a partial transaction changes visible text.
use sex_pdx::text_batch::{hash, selected_hash, Request, CAPACITY};
#[derive(Clone, Copy)]
pub struct Stage {
    owner: u32,
    sid: u64,
    sequence: u32,
    len: usize,
    color: u32,
    coverage: u64,
    selection_start: u16,
    selection_end: u16,
    selection_present: bool,
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
        selection_start: 0,
        selection_end: 0,
        selection_present: false,
        sealed: false,
        poisoned: false,
        reply_pending: false,
        bytes: [0; CAPACITY],
    };
    pub fn diagnostic(&self)->(u32,u64,u32,usize,u64,bool,bool){(self.owner,self.sid,self.sequence,self.len,self.coverage,self.sealed,self.poisoned)}
    pub fn reset(&mut self) {
        // Explicitly erase inline text and selection before terminal reuse.
        for byte in &mut self.bytes { unsafe { core::ptr::write_volatile(byte, 0); } }
        unsafe {
            core::ptr::write_volatile(&mut self.selection_start, 0);
            core::ptr::write_volatile(&mut self.selection_end, 0);
            core::ptr::write_volatile(&mut self.selection_present, false);
        }
        *self = Self::EMPTY;
    }
    /// Only expose selection after the same transaction's successful commit.
    pub fn selection(&self) -> (u16, u16) {
        if self.sealed { (self.selection_start, self.selection_end) } else { (0, 0) }
    }
    fn complete(&self) -> u64 {
        let chunks = (self.len + 7) / 8;
        if chunks == 64 { u64::MAX } else { (1u64 << chunks) - 1 }
    }
    pub fn retire(&mut self, owner: u32) {
        if self.owner == owner {
            self.reset();
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
                    self.reset();
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
                self.reset();
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
                if self.selection_present || offset >= self.len
                    || offset % 8 != 0
                    || count != (self.len - offset).min(8)
                    || bytes[count..].iter().any(|&v| v != 0)
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
            Request::Select { sequence, start, end } => {
                if (self.owner, self.sid, self.sequence) != (owner, sid, sequence)
                    || self.sealed || self.poisoned { return None; }
                if self.coverage != self.complete() || start > end || end > self.len
                    || !sex_pdx::text_encoding::is_boundary(&self.bytes[..self.len],start)
                    || !sex_pdx::text_encoding::is_boundary(&self.bytes[..self.len],end)
                    || (start == end && start != 0)
                    || (self.selection_present && (self.selection_start as usize, self.selection_end as usize) != (start, end)) {
                    self.poisoned = true;
                    return None;
                }
                self.selection_start = start as u16;
                self.selection_end = end as u16;
                self.selection_present = true;
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
                let integrity = if self.selection_present {
                    selected_hash(&self.bytes[..self.len], self.selection_start as usize, self.selection_end as usize)
                } else { Some(hash(&self.bytes[..self.len])) };
                if self.coverage != self.complete() || integrity != Some(expected)
                    || !sex_pdx::text_encoding::valid_text(&self.bytes[..self.len]) {
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
