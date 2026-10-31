//! Per-domain capability table. BOOT populates it and seals it; after the seal
//! no capability can be inserted, so no runtime path mints authority.
//! Revocation is the one permitted post-seal change: it only removes. A
//! revoked slot never becomes valid again, because nothing can refill it.
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};

use crate::capability::{Capability, CapabilityData};

/// A Lock-Free, Wait-Free Capability Table using an Atomic Array.
#[repr(align(4096))]
pub struct CapabilityTable {
    pub slots: [AtomicPtr<Capability>; 1024],
    next_slot: AtomicU32,
    sealed: AtomicBool,
}

impl CapabilityTable {
    pub const fn new() -> Self {
        const INIT: AtomicPtr<Capability> = AtomicPtr::new(ptr::null_mut());
        Self {
            slots: [INIT; 1024],
            next_slot: AtomicU32::new(0),
            sealed: AtomicBool::new(false),
        }
    }

    pub fn seal(&self) {
        self.sealed.store(true, Ordering::SeqCst);
    }

    pub fn insert(&self, data: CapabilityData) -> u32 {
        if self.sealed.load(Ordering::Acquire) {
            panic!("UCGM: Attempted to insert capability into sealed table");
        }
        let slot = self.next_slot.fetch_add(1, Ordering::SeqCst) % 1024;
        let id = slot + 1;
        let cap = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Capability { id, data, cheri_meta: 0 }));
        let old = self.slots[slot as usize].swap(cap, Ordering::AcqRel);
        if !old.is_null() {
            // RCU reclamation omitted for brevity
        }
        id
    }

    pub fn insert_at(&self, id: u32, data: CapabilityData) {
        if self.sealed.load(Ordering::Acquire) {
            panic!("UCGM: Attempted to insert_at into sealed table");
        }
        if id == 0 || id > 1024 { return; }
        let slot = (id - 1) as usize;
        let cap = alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Capability { id, data, cheri_meta: 0 }));
        let old = self.slots[slot].swap(cap, Ordering::AcqRel);
        if !old.is_null() {
            // RCU reclamation omitted
        }
    }

    pub fn find(&self, id: u32) -> Option<Capability> {
        if id == 0 || id > 1024 { return None; }
        let slot = (id - 1) as usize;
        let ptr = self.slots[slot].load(Ordering::Acquire);
        if ptr.is_null() {
            None
        } else {
            unsafe { Some(*ptr) }
        }
    }

    /// Remove one capability. A lookup that copied it before the swap is
    /// ordered before the revocation; every later lookup sees an empty slot.
    /// The record itself is retained, never freed, so a concurrent copy
    /// cannot read reclaimed memory.
    pub fn revoke(&self, id: u32) -> Option<Capability> {
        if id == 0 || id > 1024 { return None; }
        let old = self.slots[(id - 1) as usize].swap(ptr::null_mut(), Ordering::AcqRel);
        if old.is_null() { None } else { unsafe { Some(*old) } }
    }

    /// Revoke everything a terminal incarnation held; returns the count.
    pub fn revoke_all(&self) -> u32 {
        (1..=1024).filter(|&id| self.revoke(id).is_some()).count() as u32
    }

    /// True when no held capability satisfies `covers`. This inspects the
    /// actual slots; it is a check, not an assertion that revocation happened.
    pub fn holds_none(&self, covers: impl Fn(&CapabilityData) -> bool) -> bool {
        (1..=1024).filter_map(|id| self.find(id)).all(|cap| !covers(&cap.data))
    }
}
