// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! One held immutable frame. Atomic cells avoid shared Rust slice aliases.
use super::{ContentRect, Scope, BYTES, HEADER_BYTES, WIDTH, HEIGHT, PIXELS, CHUNK_PIXELS, REGION_ID};
use crate::{desktop_recording::{RoleSequence, SessionSequence, DISPLAY, TARGET},
    memory_region::{RegionView, QUERY_VIEW, READ, WRITE, USER_VA_END}, MemoryRegionInfo};
use core::sync::atomic::{AtomicU64, Ordering};
const MAGIC: u64 = 0x5245434652410001;
const COUNTER: usize = 30;
const EPOCH: usize = 31;
const HEADER_WORDS: usize = HEADER_BYTES / 8;
const PAYLOAD_WORDS: usize = (BYTES - HEADER_BYTES) / 8;
const _: () = assert!(PIXELS % 2 == 0 && CHUNK_PIXELS % 2 == 0 && HEADER_BYTES + PIXELS * 4 <= BYTES);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub scope: Scope,
    pub sequence: RoleSequence,
    pub generation: u64,
    pub publication_ms: u64,
    pub native_width: usize,
    pub native_height: usize,
    pub content: ContentRect,
    pub night: u8,
}
impl Header {
    pub fn new(scope: Scope, sequence: RoleSequence, generation: u64, publication_ms: u64,
        native_width: usize, native_height: usize, night: u8) -> Option<Self> {
        let value = Self { scope, sequence, generation, publication_ms, native_width, native_height,
            content: ContentRect::fit(native_width,native_height)?, night };
        value.valid().then_some(value)
    }
    pub fn valid(self) -> bool {
        self.scope.valid() && self.sequence.matches(self.scope.shell_epoch) && self.generation != 0
            && self.night < 4 && ContentRect::fit(self.native_width,self.native_height) == Some(self.content)
    }
    fn words(self) -> [u64; 18] {
        [MAGIC, self.scope.display_epoch, self.scope.shell_epoch, self.scope.session.word() as u64,
            self.scope.record_task as u64, self.scope.spindle_task as u64, self.sequence.word() as u64,
            self.generation, self.publication_ms, self.native_width as u64, self.native_height as u64,
            WIDTH as u64, HEIGHT as u64, self.content.x as u64, self.content.y as u64,
            self.content.width as u64, self.content.height as u64, self.night as u64]
    }
    fn decode(w: [u64; 18]) -> Option<Self> {
        if w[0] != MAGIC || w[11] != WIDTH as u64 || w[12] != HEIGHT as u64 { return None; }
        let value = Self { scope: Scope { display_epoch:w[1], shell_epoch:w[2],
            session:SessionSequence::decode(w[3])?, record_task:u32::try_from(w[4]).ok()?,
            spindle_task:u32::try_from(w[5]).ok()? }, sequence:RoleSequence::decode(w[6])?,
            generation:w[7], publication_ms:w[8], native_width:usize::try_from(w[9]).ok()?,
            native_height:usize::try_from(w[10]).ok()?, content:ContentRect {
                x:usize::try_from(w[13]).ok()?, y:usize::try_from(w[14]).ok()?,
                width:usize::try_from(w[15]).ok()?, height:usize::try_from(w[16]).ok()? },
            night:u8::try_from(w[17]).ok()? };
        value.valid().then_some(value)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot { pub token: u64, pub header: Header }

/// Local view of already held memory authority, neither Send nor Sync. Resolving
/// another view does not create another writer. Pixels use atomic64 pairs on
/// both aliases; no overlapping atomic32 or ordinary shared accesses are allowed.
pub struct Bank { base: usize, writer: bool, _local: core::marker::PhantomData<*mut ()> }
impl Bank {
    /// # Safety
    /// `info`/`view` must be actual held-Region syscall results for mapped backing.
    /// Display has the sole writer; readers obey the exact session/frame lease.
    pub unsafe fn from_resolved(info: MemoryRegionInfo, view: RegionView, writer: bool) -> Option<Self> {
        let rights = QUERY_VIEW | READ | if writer { WRITE } else { 0 };
        if info.region_id != REGION_ID || info.length != BYTES as u64 || info.rights != rights
            || info.peer_pd != Some(if writer { TARGET } else { DISPLAY })
            || view.region_id != REGION_ID || view.length != BYTES as u64
            || view.virtual_address == 0 || view.virtual_address % 4096 != 0 || view.dma_address.is_some()
            || view.virtual_address.checked_add(BYTES as u64).is_none_or(|end|end>USER_VA_END) { return None; }
        Some(Self { base:usize::try_from(view.virtual_address).ok()?, writer,
            _local:core::marker::PhantomData })
    }
    fn word(&self, index: usize) -> u64 {
        unsafe { (&*(self.base as *const AtomicU64).add(index)).load(Ordering::SeqCst) }
    }
    fn put(&self, index: usize, value: u64) {
        unsafe { (&*(self.base as *const AtomicU64).add(index)).store(value,Ordering::SeqCst) }
    }
    pub fn service_epoch(&self) -> u64 { self.word(EPOCH) }
    /// Scrub sensitive metadata/pixels, retaining only provider epoch and a
    /// nonwrapping publication counter. No permission or last-owner data survives.
    pub fn scrub(&mut self) -> bool {
        if !self.writer { return false; }
        self.put(0,0);
        for index in 1..HEADER_WORDS { if index != EPOCH && index != COUNTER { self.put(index,0); } }
        for index in 0..PAYLOAD_WORDS { self.put(HEADER_WORDS+index,0); }
        true
    }
    pub fn install_epoch(&mut self, epoch: u64) -> bool {
        if !self.writer || epoch == 0 || epoch > u32::MAX as u64
            || (self.service_epoch() != 0 && epoch <= self.service_epoch()) { return false; }
        self.scrub(); self.put(EPOCH,epoch); true
    }
    pub fn snapshot(&self) -> Option<Snapshot> {
        let token = self.word(0);
        if token == 0 || token & 1 != 0 || self.word(COUNTER) != token { return None; }
        let words = core::array::from_fn(|index|self.word(index+1));
        let header = Header::decode(words)?;
        if header.scope.display_epoch != self.service_epoch()
            || (19..HEADER_WORDS).any(|i|i!=COUNTER&&i!=EPOCH&&self.word(i)!=0)
            || self.word(0) != token || self.word(COUNTER) != token { return None; }
        Some(Snapshot { token, header })
    }
    /// Only an actual completed private frame may be supplied by Display. A
    /// held frame cannot be overwritten until the controller scrubs its lease.
    pub fn publish(&mut self, header: Header, source: &[u32]) -> bool {
        if !self.writer || !header.valid() || self.service_epoch()!=header.scope.display_epoch
            || header.native_width.checked_mul(header.native_height)!=Some(source.len())
            || self.word(0)!=0 { return false; }
        let old = self.word(COUNTER);
        let Some(next) = old.checked_add(2).filter(|value|*value!=0&&*value&1==0) else { return false; };
        self.put(COUNTER,next);
        let sample = |index:usize| {
            let (x,y)=(index%WIDTH,index/WIDTH);let r=header.content;
            if x<r.x || y<r.y || x-r.x>=r.width || y-r.y>=r.height { return 0u32; }
            let sx=(((2*(x-r.x)+1)as u64*header.native_width as u64)/(2*r.width)as u64)
                .min(header.native_width as u64-1)as usize;
            let sy=(((2*(y-r.y)+1)as u64*header.native_height as u64)/(2*r.height)as u64)
                .min(header.native_height as u64-1)as usize;
            crate::desktop_look::night(source[sy*header.native_width+sx],header.night)&0x00ff_ffff
        };
        for pair in 0..PIXELS/2 {
            self.put(HEADER_WORDS+pair,sample(pair*2)as u64|((sample(pair*2+1)as u64)<<32));
        }
        for (index,word) in header.words().into_iter().enumerate() { self.put(index+1,word); }
        self.put(0,next); true
    }
    /// Copy at most4096 pixels into one private caller-owned frame. Cancellation
    /// can invalidate a turn: callers must discard all partial private pixels
    /// unless every turn and final current-scope check succeeds before ACK.
    pub fn copy_chunk(&self, expected: Snapshot, destination: &mut [u32], offset: usize) -> Option<usize> {
        if self.writer || destination.len()!=PIXELS || offset>PIXELS || offset%2!=0
            || self.snapshot()!=Some(expected) { return None; }
        let count = CHUNK_PIXELS.min(PIXELS-offset);
        for pair in offset/2..(offset+count)/2 {
            let word=self.word(HEADER_WORDS+pair);
            destination[pair*2]=word as u32;destination[pair*2+1]=(word>>32)as u32;
        }
        (self.snapshot()==Some(expected)).then_some(count)
    }
}
