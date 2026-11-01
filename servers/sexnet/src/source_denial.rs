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
#[derive(Clone, Copy)]
struct Receipt { seq: u32, word: u64 }
#[derive(Clone, Copy)]
struct Slot {
    owner: Option<Identity>,
    cached_seq: Option<u32>,
    pending: Option<Receipt>,
}
impl Slot {
    const EMPTY: Self = Self { owner: None, cached_seq: None, pending: None };
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
        let Some(relative) = partition.iter().position(|slot| slot.owner == Some(owner))
            .or_else(|| partition.iter().position(|slot| slot.owner.is_none())) else { return false; };
        let slot = &mut self.slots[start + relative];
        if let Some(pending) = slot.pending { return pending.seq == seq; }
        // The cache always denotes the latest DENIED, so its typed word is
        // deterministic from this partition and sequence. Only the exact
        // pending receipt needs a retained word (it may instead be STALE).
        let receipt = Receipt { seq, word: match slot.cached_seq {
            Some(cached) if seq < cached => kind.stale(seq),
            _ => kind.denied(seq),
        } };
        slot.owner = Some(owner);
        slot.pending = Some(receipt);
        // A stale error is retained for delivery but never replaces the latest
        // DENIED cache/floor, including at the protocol's last finite sequence.
        if slot.cached_seq.is_none_or(|cached| seq > cached) { slot.cached_seq = Some(seq); }
        true
    }
    fn selected(&self) -> Option<usize> {
        (0..TOTAL_SLOTS).map(|offset| (self.cursor + offset) % TOTAL_SLOTS)
            .find(|&index| self.slots[index].pending.is_some())
    }
    pub fn pending_reply(&self) -> Option<(Identity, u64)> {
        let slot = &self.slots[self.selected()?];
        Some((slot.owner?, slot.pending?.word))
    }
    pub fn reply_accepted(&mut self) {
        if let Some(index) = self.selected() {
            self.slots[index].pending = None;
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
            if slot.owner == Some(owner) { *slot = Slot::EMPTY; removed = true; }
        }
        removed
    }
}
