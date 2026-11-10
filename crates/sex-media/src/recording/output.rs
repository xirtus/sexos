//! Bounded, ordered output writes. Seek-based container patches remain explicit
//! writes for the owner's main loop; callbacks never perform filesystem IPC.
use alloc::{collections::VecDeque, vec::Vec};

pub const TURN_BYTES: usize = 32 * 1024;
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_CHUNKS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Limit,
    Memory,
    Invalid,
    Sequence,
}

struct Chunk {
    token: u64,
    offset: u64,
    bytes: Vec<u8>,
    consumed: usize,
}

pub struct Write<'a> {
    pub token: u64,
    pub offset: u64,
    pub bytes: &'a [u8],
}

pub struct Output {
    queue: VecDeque<Chunk>,
    cursor: u64,
    length: u64,
    bytes: usize,
    limit: usize,
    sequence: u64,
}
impl Output {
    pub fn new(limit: usize) -> Self {
        Self {
            queue: VecDeque::new(),
            cursor: 0,
            length: 0,
            bytes: 0,
            limit,
            sequence: 0,
        }
    }
    pub fn empty(&self) -> bool {
        self.queue.is_empty()
    }
    pub fn len(&self) -> u64 {
        self.length
    }
    pub fn pending_bytes(&self) -> usize {
        self.bytes
    }
    pub fn write(&mut self, source: &[u8]) -> Result<(), Failure> {
        if source.is_empty() {
            return Ok(());
        }
        if self.queue.len() >= MAX_CHUNKS {
            return Err(Failure::Limit);
        }
        let end = self
            .cursor
            .checked_add(source.len() as u64)
            .filter(|&n| n <= MAX_FILE_BYTES)
            .ok_or(Failure::Limit)?;
        let total = self
            .bytes
            .checked_add(source.len())
            .filter(|&n| n <= self.limit)
            .ok_or(Failure::Limit)?;
        let token = self.sequence.checked_add(1).ok_or(Failure::Sequence)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(source.len())
            .map_err(|_| Failure::Memory)?;
        bytes.extend_from_slice(source);
        self.queue.try_reserve(1).map_err(|_| Failure::Memory)?;
        self.queue.push_back(Chunk {
            token,
            offset: self.cursor,
            bytes,
            consumed: 0,
        });
        self.sequence = token;
        self.cursor = end;
        self.length = self.length.max(end);
        self.bytes = total;
        Ok(())
    }
    /// AVIO seek constants are deliberately kept out of the filesystem layer.
    /// A seek moves only the prospective output cursor; it never issues I/O.
    pub fn seek(&mut self, offset: i64, whence: i32) -> Result<u64, Failure> {
        let whence = whence & !0x20000; // AVSEEK_FORCE does not change the base.
        if whence == 0x10000 {
            return Ok(self.length);
        } // AVSEEK_SIZE
        let base = match whence {
            0 => 0,
            1 => self.cursor,
            2 => self.length,
            _ => return Err(Failure::Invalid),
        };
        let at = base
            .checked_add_signed(offset)
            .filter(|&n| n <= MAX_FILE_BYTES)
            .ok_or(Failure::Invalid)?;
        self.cursor = at;
        Ok(at)
    }
    pub fn pending(&self) -> Option<Write<'_>> {
        let chunk = self.queue.front()?;
        let end = chunk.bytes.len().min(chunk.consumed + TURN_BYTES);
        Some(Write {
            token: chunk.token,
            offset: chunk.offset + chunk.consumed as u64,
            bytes: &chunk.bytes[chunk.consumed..end],
        })
    }
    /// Advance only the exact currently advertised turn. A short successful
    /// write gets a fresh token, so replaying its acknowledgement cannot consume
    /// another part of the same packet. The owner still authenticates its IPC.
    pub fn consume(&mut self, token: u64, length: usize) -> Result<(), Failure> {
        let chunk = self.queue.front_mut().ok_or(Failure::Invalid)?;
        let available = (chunk.bytes.len() - chunk.consumed).min(TURN_BYTES);
        if token != chunk.token || length == 0 || length > available {
            return Err(Failure::Invalid);
        }
        let next = self.sequence.checked_add(1).ok_or(Failure::Sequence)?;
        chunk.consumed += length;
        self.bytes -= length;
        self.sequence = next;
        if chunk.consumed == chunk.bytes.len() {
            self.queue.pop_front();
        } else {
            chunk.token = next;
        }
        Ok(())
    }
}
