// SPDX-License-Identifier: GPL-3.0-only
//! Public error receipts for unsupported, already-held Net callers only.
//! The parent authenticates the exact live identity, decodes the full typed
//! request, and selects this lane before entering any authorized source Machine.
//! No provider, ticket, entropy words, clock/high-water or policy state exists.
//! Entropy coalescing is deliberately sequence-only: differing operations or
//! tickets at that correlation all mean DENIED, never operation confirmation.
use sex_pdx::{certificate_time, entropy_seed, trust_policy};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Identity { pub pd: u32, pub task: u32 }
impl Identity {
    pub const fn valid(self) -> bool { self.pd != 0 && self.task != 0 }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Entropy, CertificateTime, Trust }
impl Kind {
    const fn index(self) -> usize {
        match self { Self::Entropy => 0, Self::CertificateTime => 1, Self::Trust => 2 }
    }
    const fn from_index(index: usize) -> Option<Self> {
        match index { 0 => Some(Self::Entropy), 1 => Some(Self::CertificateTime), 2 => Some(Self::Trust), _ => None }
    }
    fn valid_sequence(self, seq: u32) -> bool {
        match self {
            Self::Entropy => seq != 0 && seq <= entropy_seed::MAX_SEQ,
            Self::CertificateTime => certificate_time::Request { seq }.valid(),
            Self::Trust => trust_policy::Request { seq }.valid(),
        }
    }
    fn denied(self, seq: u32) -> u64 {
        match self {
            Self::Entropy => entropy_seed::error(seq, entropy_seed::DENIED),
            Self::CertificateTime => certificate_time::error(seq, certificate_time::DENIED),
            Self::Trust => trust_policy::error(seq, trust_policy::DENIED),
        }
    }
    fn stale(self, seq: u32) -> u64 {
        match self {
            Self::Entropy => entropy_seed::error(seq, entropy_seed::STALE),
            Self::CertificateTime => certificate_time::error(seq, certificate_time::STALE),
            Self::Trust => trust_policy::error(seq, trust_policy::STALE),
        }
    }
}
/// Sizing only: descriptive rows confer no right to enter this outbox. The
/// parent must first attest its actual held caller/main-task tuple. Independent
/// partitions prevent one protocol's flood from occupying another's fences.
pub const OWNERS_PER_KIND: usize = sex_pdx::app_catalog::COUNT;
pub const TOTAL_SLOTS: usize = 3 * OWNERS_PER_KIND;
const _: () = assert!(OWNERS_PER_KIND > 0);
// Zero is outside every admitted identity/sequence namespace. The three
// protocol partitions are immutable; a pending slot's word is reconstructed
// from its sequence and stable latest-DENIED floor, never source/private data.
#[derive(Clone, Copy)]
struct Slot {
    owner: Identity,
    cached_seq: u32,
    pending_seq: u32,
}
impl Slot {
    const EMPTY: Self = Self { owner: Identity { pd: 0, task: 0 }, cached_seq: 0, pending_seq: 0 };
}
pub struct Outbox { slots: [Slot; TOTAL_SLOTS], cursor: usize }
impl Outbox {
    pub const fn new() -> Self { Self { slots: [Slot::EMPTY; TOTAL_SLOTS], cursor: 0 } }
    /// False leaves every receipt, floor and cursor untouched. A different
    /// pending correlation backpressures even a cached/latest-sequence replay.
    pub fn deny(&mut self, kind: Kind, owner: Identity, seq: u32) -> bool {
        if !owner.valid() || !kind.valid_sequence(seq) { return false; }
        let start = kind.index() * OWNERS_PER_KIND;
        let partition = &self.slots[start..start + OWNERS_PER_KIND];
        let Some(relative) = partition.iter().position(|slot| slot.owner == owner)
            .or_else(|| partition.iter().position(|slot| slot.owner.pd == 0)) else { return false; };
        let slot = &mut self.slots[start + relative];
        // The pending guard must precede any floor update. Otherwise a
        // rejected newer request could change a Full-retained DENIED to STALE.
        if slot.pending_seq != 0 { return slot.pending_seq == seq; }
        slot.owner = owner;
        if seq > slot.cached_seq { slot.cached_seq = seq; }
        slot.pending_seq = seq;
        true
    }
    fn selected(&self) -> Option<usize> {
        (0..TOTAL_SLOTS).map(|offset| (self.cursor + offset) % TOTAL_SLOTS)
            .find(|&index| self.slots[index].pending_seq != 0)
    }
    pub fn pending_reply(&self) -> Option<(Identity, u64)> {
        let index = self.selected()?;
        let slot = &self.slots[index];
        let kind = Kind::from_index(index / OWNERS_PER_KIND)?;
        if !slot.owner.valid() || slot.cached_seq == 0 || slot.pending_seq > slot.cached_seq { return None; }
        let word = if slot.pending_seq < slot.cached_seq { kind.stale(slot.pending_seq) } else { kind.denied(slot.pending_seq) };
        Some((slot.owner, word))
    }
    pub fn reply_accepted(&mut self) {
        if let Some(index) = self.selected() {
            self.slots[index].pending_seq = 0;
            self.cursor = (index + 1) % TOTAL_SLOTS;
        }
    }
    /// Full retains the exact word and owner while allowing another pending
    /// protocol/owner a bounded fair turn. No accepted effect is replayed.
    pub fn reply_deferred(&mut self) {
        if let Some(index) = self.selected() { self.cursor = (index + 1) % TOTAL_SLOTS; }
    }
    /// The parent separately proves terminal custody before this call/ACK.
    /// A hint, descriptive row or a different task cannot retire an identity.
    pub fn retire(&mut self, owner: Identity) -> bool {
        if !owner.valid() { return false; }
        let mut removed = false;
        for slot in &mut self.slots {
            if slot.owner == owner { *slot = Slot::EMPTY; removed = true; }
        }
        removed
    }
}
