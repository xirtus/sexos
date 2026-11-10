// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! A single bounded BOOT screenshot bank. Names/layouts are not authority.
//! Display alone writes; the first trusted Spindle reads. PD tasks share rights.
use crate::{
    memory_region::{RegionView, QUERY_VIEW, READ, USER_VA_END, WRITE},
    MemoryRegionInfo,
};
pub const DISPLAY: u32 = 1;
pub const CLIENT: u32 = crate::app_catalog::SPINDLE_TARGETS[0];
pub const SLOT: u64 = 94;
pub const REGION_ID: u32 = 0x6100;
pub const BYTES: usize = 16 * 1024 * 1024;
pub const HEADER_BYTES: usize = 4096;
pub const MAX_PIXELS: usize = (BYTES - HEADER_BYTES) / 4;
pub const OP: u64 = 0x2e0;
pub const BEGIN: u64 = 1;
pub const RELEASE: u64 = 2;
pub const OK: u8 = 0;
pub const DENIED: u8 = 1;
pub const BUSY: u8 = 2;
pub const OVERSIZE: u8 = 3;
pub const CANCELLED: u8 = 4;
pub const LEASE_TICKS: u64 = 120_000;
const MAGIC: u64 = 0x53435245454e0001;
const TAG: u64 = 0xc4;
pub fn reply(sequence: u32, status: u8, kind: u64) -> u64 {
    (TAG << 56) | ((status as u64) << 48) | (kind << 40) | sequence as u64
}
pub fn decode_reply(word: u64) -> Option<(u32, u8, u64)> {
    if word >> 56 != TAG || word & 0x000000ff00000000 != 0 {
        return None;
    }
    let status = ((word >> 48) & 255) as u8;
    let kind = (word >> 40) & 255;
    if status > CANCELLED || !matches!(kind, BEGIN | RELEASE) {
        return None;
    }
    Some((word as u32, status, kind))
}
pub fn pixels(width: usize, height: usize) -> Option<usize> {
    if width == 0 || height == 0 || width > 8192 || height > 4320 {
        return None;
    }
    width.checked_mul(height).filter(|&n| n <= MAX_PIXELS)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub epoch: u64,
    pub sequence: u32,
    pub task: u32,
    pub generation: u64,
    pub width: usize,
    pub height: usize,
    pub night: u8,
}
impl Header {
    pub fn valid(self, sequence: u32, task: u32) -> bool {
        self.epoch != 0
            && sequence != 0
            && task != 0
            && self.sequence == sequence
            && self.task == task
            && self.generation != 0
            && self.night < 4
            && pixels(self.width, self.height).is_some()
    }
}
/// Non-transferable local view of this PD's already-held grant. Resolving it
/// twice does not create two exclusive owners. No task isolation is asserted.
pub struct Bank {
    base: usize,
    writer: bool,
    _local: core::marker::PhantomData<*mut ()>,
}
impl Bank {
    /// # Safety
    /// Replies must come from the actual held Region syscalls and backing must
    /// remain mapped. Display is its sole writer; readers obey the exact lease.
    pub unsafe fn from_resolved(
        info: MemoryRegionInfo,
        view: RegionView,
        writer: bool,
    ) -> Option<Self> {
        let rights = QUERY_VIEW | READ | if writer { WRITE } else { 0 };
        let peer = if writer { CLIENT } else { DISPLAY };
        if info.region_id != REGION_ID
            || info.length != BYTES as u64
            || info.rights != rights
            || info.peer_pd != Some(peer)
            || view.region_id != REGION_ID
            || view.length != BYTES as u64
            || view.virtual_address == 0
            || view.virtual_address % 4096 != 0
            || view.dma_address.is_some()
            || view
                .virtual_address
                .checked_add(BYTES as u64)
                .is_none_or(|end| end > USER_VA_END)
        {
            return None;
        }
        Some(Self {
            base: usize::try_from(view.virtual_address).ok()?,
            writer,
            _local: core::marker::PhantomData,
        })
    }
    fn word(&self, index: usize) -> u64 {
        unsafe {
            (&*((self.base as *const core::sync::atomic::AtomicU64).add(index)))
                .load(core::sync::atomic::Ordering::SeqCst)
        }
    }
    fn put(&self, index: usize, value: u64) {
        unsafe {
            (&*((self.base as *const core::sync::atomic::AtomicU64).add(index)))
                .store(value, core::sync::atomic::Ordering::SeqCst)
        }
    }
    pub fn service_epoch(&self) -> u64 {
        self.word(7)
    }
    pub fn install_epoch(&mut self, epoch: u64) {
        assert!(self.writer && self.word(0) == 0);
        self.put(7, epoch);
    }
    pub fn header(&self) -> Option<Header> {
        if self.word(0) != MAGIC {
            return None;
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        let sequence = u32::try_from(self.word(1)).ok()?;
        let task = u32::try_from(self.word(2)).ok()?;
        let h = Header {
            epoch: self.word(7),
            sequence,
            task,
            generation: self.word(3),
            width: usize::try_from(self.word(4)).ok()?,
            height: usize::try_from(self.word(5)).ok()?,
            night: u8::try_from(self.word(6)).ok()?,
        };
        if !h.valid(sequence, task)
            || self.word(0) != MAGIC
            || self.word(1) != sequence as u64
            || self.word(7) != h.epoch
        {
            return None;
        }
        Some(h)
    }
    /// Bounded atomic read: cancellation/timeout may scrub concurrently. The
    /// client validates its header before and after each encoding turn and aborts
    /// if custody changed. No immutable Rust slice aliases a mutable shared bank.
    pub fn pixel(&self, index: usize) -> Option<u32> {
        if self.writer || index >= MAX_PIXELS {
            return None;
        }
        Some(unsafe {
            (&*(((self.base + HEADER_BYTES) as *const core::sync::atomic::AtomicU32).add(index)))
                .load(core::sync::atomic::Ordering::SeqCst)
        })
    }
    /// Scrub header and payload with matching atomic widths, even on timeout.
    pub fn scrub(&mut self) {
        assert!(self.writer);
        let epoch = self.word(7);
        self.put(0, 0);
        for n in 1..HEADER_BYTES / 8 {
            self.put(n, 0)
        }
        for n in 0..MAX_PIXELS {
            unsafe {
                (&*(((self.base + HEADER_BYTES) as *const core::sync::atomic::AtomicU32).add(n)))
                    .store(0, core::sync::atomic::Ordering::SeqCst)
            }
        }
        self.put(7, epoch);
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    }
    /// Synchronously copy a completed private frame; publish metadata last.
    pub fn publish(
        &mut self,
        header: Header,
        source: &[u32],
        mut transform: impl FnMut(u32) -> u32,
    ) -> bool {
        if !self.writer
            || self.service_epoch() != header.epoch
            || !header.valid(header.sequence, header.task)
            || pixels(header.width, header.height) != Some(source.len())
        {
            return false;
        }
        self.put(0, 0);
        for (i, &p) in source.iter().enumerate() {
            unsafe {
                (&*(((self.base + HEADER_BYTES) as *const core::sync::atomic::AtomicU32).add(i)))
                    .store(transform(p), core::sync::atomic::Ordering::SeqCst)
            }
        }
        for (i, v) in [
            header.sequence as u64,
            header.task as u64,
            header.generation,
            header.width as u64,
            header.height as u64,
            header.night as u64,
        ]
        .iter()
        .enumerate()
        {
            self.put(i + 1, *v)
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        self.put(0, MAGIC);
        true
    }
}
