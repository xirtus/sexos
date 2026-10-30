// SPDX-License-Identifier: GPL-3.0-only
//! Coherent data on existing BOOT Region views. No descriptor grants authority.
//! Every shared word is atomic: a sequence check alone cannot make racing
//! non-atomic payload accesses sound. SC words keep the publication proof small.
use core::sync::atomic::{AtomicU64, Ordering::SeqCst};
use crate::{document as w, document_buffer as b, memory_region as m};
const MAGIC: u64 = 0x3150_414e_5351_4553; // SEQSNAP1
const WORDS: usize = b::BYTES as usize / 8;
const DATA: usize = 512 / 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity {
    pub subject: u32,
    pub object: u8,
    pub generation: u32,
    pub sequence: u32,
}
pub struct Region { address: u64, subject: u32, writer: bool }
impl Region {
    pub fn subject(&self) -> u32 { self.subject }
    /// `info` and `view` MUST come from current-domain sex-pdx capability
    /// queries, never a message or untrusted descriptor. The BOOT mapping must
    /// remain valid for this object's lifetime. Host tests supply owned backing.
    pub unsafe fn from_resolved(info: m::MemoryRegionInfo, view: m::RegionView,
        subject: u32, writer: bool) -> Result<Self, u32>
    {
        let index = match subject { 9 => 0, 16 => 1, 18 => 2, _ => return Err(w::DENIED) };
        let rights = m::QUERY_VIEW | m::READ | if writer { m::WRITE } else { 0 };
        if info.region_id != b::FIRST_REGION_ID + index || view.region_id != info.region_id
            || info.length != b::BYTES || view.length != b::BYTES || info.rights != rights
            || info.peer_pd != Some(if writer { subject } else { 11 }) || view.dma_address.is_some()
            || view.virtual_address == 0 || view.virtual_address % 4096 != 0
            || view.virtual_address.checked_add(b::BYTES).map_or(true, |end|end > m::USER_VA_END)
        { return Err(w::DENIED); }
        Ok(Self { address: view.virtual_address, subject, writer })
    }
    fn word(&self, index: usize) -> &AtomicU64 {
        assert!(index < WORDS);
        unsafe { &*((self.address as *const AtomicU64).add(index)) }
    }
    /// Invalidate before scrubbing, then leave the entire fixed reservation zero.
    pub fn clear(&mut self) -> Result<(), u32> {
        if !self.writer { return Err(w::DENIED); }
        self.word(0).store(1, SeqCst);
        for index in 1..WORDS { self.word(index).store(0, SeqCst); }
        self.word(0).store(0, SeqCst);
        Ok(())
    }
    pub fn publish(&mut self, id: Identity, name: [u8;16], bytes: &[u8]) -> Result<(), u32> {
        if !self.writer || id.subject != self.subject || !(2..15).contains(&id.object)
            || id.sequence == 0 || id.sequence > w::SEQ_MAX || bytes.len() > w::MAX
        { return Err(w::DENIED); }
        let token = (id.sequence as u64) << 1;
        let previous = self.word(0).load(SeqCst);
        if previous & 1 != 0 { return Err(w::BUSY); }
        if previous != 0 && token <= previous { return Err(w::STALE); }
        self.word(0).store(token | 1, SeqCst);
        for index in 1..WORDS { self.word(index).store(0, SeqCst); }
        self.word(1).store(MAGIC, SeqCst);
        self.word(2).store((11u64 << 32) | id.subject as u64, SeqCst);
        self.word(3).store((id.generation as u64) << 32 | (id.object as u64) << 24 | id.sequence as u64, SeqCst);
        self.word(4).store(bytes.len() as u64, SeqCst);
        self.word(5).store(w::hash(bytes), SeqCst);
        for index in 0..2 { self.word(6+index).store(u64::from_le_bytes(name[index*8..index*8+8].try_into().unwrap()), SeqCst); }
        for (index, chunk) in bytes.chunks(8).enumerate() {
            let mut raw = [0;8]; raw[..chunk.len()].copy_from_slice(chunk);
            self.word(DATA+index).store(u64::from_le_bytes(raw), SeqCst);
        }
        self.word(0).store(token, SeqCst);
        Ok(())
    }
    pub fn copy(&self, id: Identity, length: usize, out: &mut [u8;w::MAX], name: &mut [u8;16]) -> Result<(), u32> {
        // No partial copy escapes on any validation/coherence failure.
        let result = self.copy_inner(id, length, out, name);
        if result.is_err() { out.fill(0); name.fill(0); }
        result
    }
    fn copy_inner(&self, id: Identity, length: usize, out: &mut [u8;w::MAX], name: &mut [u8;16]) -> Result<(), u32> {
        if self.writer || id.subject != self.subject || !(2..15).contains(&id.object)
            || id.sequence == 0 || id.sequence > w::SEQ_MAX || length > w::MAX
        { return Err(w::DENIED); }
        let token = (id.sequence as u64) << 1;
        if self.word(0).load(SeqCst) != token || self.word(1).load(SeqCst) != MAGIC
            || self.word(2).load(SeqCst) != (11u64 << 32) | id.subject as u64
            || self.word(3).load(SeqCst) != (id.generation as u64) << 32 | (id.object as u64) << 24 | id.sequence as u64
            || self.word(4).load(SeqCst) != length as u64
        { return Err(w::STALE); }
        let hash = self.word(5).load(SeqCst);
        for index in 0..2 { name[index*8..index*8+8].copy_from_slice(&self.word(6+index).load(SeqCst).to_le_bytes()); }
        out.fill(0);
        for (index, chunk) in out[..length].chunks_mut(8).enumerate() {
            chunk.copy_from_slice(&self.word(DATA+index).load(SeqCst).to_le_bytes()[..chunk.len()]);
        }
        if self.word(0).load(SeqCst) != token || w::hash(&out[..length]) != hash { return Err(w::STALE); }
        Ok(())
    }
}
