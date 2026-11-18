//! Per-domain capability table (authority v2, plan #405 Phase 3, D1).
//!
//! Every slot is 32 bytes stored inline, no `Box` per capability. A slot holds
//! either a v2 object capability (Endpoint, Notification, Reply: object,
//! generation, rights, badge, all inline) or a pointer to one immutable record
//! of a BOOT-wired resource kind (PCI lease, region, grants), allocated once by
//! BOOT before the seal and never freed.
//!
//! BOOT populates the table and seals it. After the seal the legacy `insert`
//! paths still refuse, so resource authority stays exactly what BOOT wired. The
//! one runtime path that adds authority is `install` of an object capability
//! derived with `derive` from a capability the minter already holds with the
//! GRANT right; the chain always starts at a BOOT-held root, so nothing exists
//! without BOOT sanction.
//!
//! Revocation: removing a slot (`revoke`) or bumping an object's generation
//! (`OBJECTS.revoke`), which kills every copy in every table at once, because
//! an object capability is honoured only while its generation is live.
//! A domain's death bumps its incarnation and destroys the objects it owned
//! (`end_incarnation`), so no new call and no pending reply survives it.
use core::sync::atomic::{fence, AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::capability::{Capability, CapabilityData};

/// Rights bits carried by every v2 object capability. A derived capability
/// holds a subset of its parent's rights.
pub mod rights {
    pub const READ: u32 = 1 << 0;
    pub const WRITE: u32 = 1 << 1;
    /// May derive and grant a capability with a subset of these rights.
    pub const GRANT: u32 = 1 << 2;
    /// Endpoint: may Call it. Notification: may signal it.
    pub const CALL: u32 = 1 << 3;
    /// Endpoint: may receive on it. Notification: may wait on it.
    pub const RECEIVE: u32 = 1 << 4;
    pub const ALL: u32 = READ | WRITE | GRANT | CALL | RECEIVE;
}

/// A v2 kernel-object capability: which object, which generation of it, what
/// the holder may do and the badge the kernel delivers with each use. Valid
/// only while `generation` equals the object's live generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectCap {
    pub object: u32,
    pub generation: u32,
    pub rights: u32,
    pub badge: u64,
}

/// Why a derivation or install was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MintError {
    /// No such slot, or the slot holds no object capability.
    NotHeld,
    /// The source lacks GRANT, or asked for rights it does not hold.
    Rights,
    /// The source is badged and the derivation tried to change the badge.
    Badge,
    /// The source's object generation is no longer live.
    Stale,
    /// The destination table has no free slot.
    Full,
}

impl ObjectCap {
    /// A child of `self` with `rights` narrowed and, for an unbadged parent
    /// only, a fresh `badge`. A badged parent can only pass its own badge on,
    /// so a server can tell its clients apart and a client cannot pose as
    /// another. Deriving never widens rights and never revives a dead object.
    pub fn derive(&self, rights: u32, badge: u64) -> Result<ObjectCap, MintError> {
        if self.rights & rights::GRANT == 0 || rights & !self.rights != 0 {
            return Err(MintError::Rights);
        }
        if self.badge != 0 && badge != self.badge {
            return Err(MintError::Badge);
        }
        Ok(ObjectCap { rights, badge, ..*self })
    }

    pub fn allows(&self, needed: u32) -> bool {
        self.rights & needed == needed
    }
}

// ---------------------------------------------------------------------------
// Object and incarnation generations.

/// Kernel objects (endpoints, notifications) that capabilities can name.
pub const MAX_OBJECTS: usize = 4096;
/// Domain ids whose incarnations are counted (matches ipc::MAX_DOMAINS).
pub const MAX_INCARNATIONS: usize = 1024;
const FREE: u32 = u32::MAX;

/// The live generation of every kernel object and of every domain's current
/// incarnation. Generation 0 is never live, so a zeroed capability is dead.
/// The words are kernel-written only and readable without a lock, so a
/// server can check a reply's liveness with one load.
pub struct Generations {
    object_generation: [AtomicU32; MAX_OBJECTS],
    object_owner: [AtomicU32; MAX_OBJECTS],
    incarnation: [AtomicU32; MAX_INCARNATIONS],
}

pub static OBJECTS: Generations = Generations::new();

impl Generations {
    pub const fn new() -> Self {
        Self {
            object_generation: [const { AtomicU32::new(1) }; MAX_OBJECTS],
            object_owner: [const { AtomicU32::new(FREE) }; MAX_OBJECTS],
            incarnation: [const { AtomicU32::new(1) }; MAX_INCARNATIONS],
        }
    }

    /// Allocate an object owned by domain `owner`. Returns the root
    /// capability with every right and no badge; the owner hands out
    /// derived copies of it.
    pub fn create(&self, owner: u32) -> Option<ObjectCap> {
        if owner == FREE { return None; }
        for (object, slot) in self.object_owner.iter().enumerate() {
            if slot.compare_exchange(FREE, owner, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                let generation = self.object_generation[object].load(Ordering::Acquire);
                return Some(ObjectCap { object: object as u32, generation, rights: rights::ALL, badge: 0 });
            }
        }
        None
    }

    pub fn owner(&self, object: u32) -> Option<u32> {
        if object as usize >= MAX_OBJECTS { return None; }
        let owner = self.object_owner[spec_clamp(object as usize, MAX_OBJECTS)].load(Ordering::Acquire);
        (owner != FREE).then_some(owner)
    }

    /// True while `cap` names a live object at its current generation.
    pub fn live(&self, cap: &ObjectCap) -> bool {
        let object = spec_clamp(cap.object as usize, MAX_OBJECTS);
        (cap.object as usize) < MAX_OBJECTS
            && self.object_owner[object].load(Ordering::Acquire) != FREE
            && self.object_generation[object].load(Ordering::Acquire) == cap.generation
    }

    /// Revoke every capability to `object` at once; the object lives on at a
    /// new generation and its owner mints fresh roots from the returned one.
    pub fn revoke(&self, object: u32) -> Option<ObjectCap> {
        self.owner(object)?;
        let generation = self.bump(&self.object_generation[spec_clamp(object as usize, MAX_OBJECTS)]);
        Some(ObjectCap { object, generation, rights: rights::ALL, badge: 0 })
    }

    /// Revoke every capability to `object` and free it for reuse.
    pub fn destroy(&self, object: u32) -> bool {
        if self.owner(object).is_none() { return false; }
        let index = spec_clamp(object as usize, MAX_OBJECTS);
        self.bump(&self.object_generation[index]);
        self.object_owner[index].store(FREE, Ordering::Release);
        true
    }

    /// The current incarnation generation of domain `pd`; reply capabilities
    /// to a caller carry it.
    pub fn incarnation(&self, pd: u32) -> u32 {
        if pd as usize >= MAX_INCARNATIONS { return 0; }
        self.incarnation[spec_clamp(pd as usize, MAX_INCARNATIONS)].load(Ordering::Acquire)
    }

    /// True while a reply capability's caller incarnation is still alive.
    pub fn caller_live(&self, pd: u32, generation: u32) -> bool {
        generation != 0 && self.incarnation(pd) == generation
    }

    /// A domain incarnation ended: every reply owed to it dies, and every
    /// object it owned is destroyed, so every capability naming one, in any
    /// table, is dead. Returns how many objects were destroyed.
    pub fn end_incarnation(&self, pd: u32) -> u32 {
        if (pd as usize) < MAX_INCARNATIONS { self.bump(&self.incarnation[spec_clamp(pd as usize, MAX_INCARNATIONS)]); }
        if pd == FREE { return 0; }
        let mut destroyed = 0;
        for (object, owner) in self.object_owner.iter().enumerate() {
            if owner.load(Ordering::Acquire) == pd && self.destroy(object as u32) { destroyed += 1; }
        }
        destroyed
    }

    fn bump(&self, word: &AtomicU32) -> u32 {
        let mut current = word.load(Ordering::Acquire);
        loop {
            let next = next_generation(current);
            match word.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return next,
                Err(seen) => current = seen,
            }
        }
    }
}

/// Spectre v1 (audit S10): an index from a register is bounds-checked by a
/// branch, which can be mispredicted. After the check the index is ANDed with
/// a mask computed by `cmp`+`sbb` (all ones when in range, else zero), so even
/// a speculative load stays inside the array. Plain Rust masking would be
/// folded away by the optimizer, which already knows the index is in range.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn spec_clamp(index: usize, len: usize) -> usize {
    let mask: usize;
    // SAFETY: register-only arithmetic, no memory or stack access.
    unsafe {
        core::arch::asm!("cmp {i}, {l}", "sbb {m}, {m}", i = in(reg) index, l = in(reg) len, m = out(reg) mask,
                         options(pure, nomem, nostack));
    }
    index & mask
}

#[cfg(not(target_arch = "x86_64"))]
#[inline(always)]
fn spec_clamp(index: usize, len: usize) -> usize {
    index & (len - 1)
}
const _: () = assert!(SLOT_COUNT.is_power_of_two() && MAX_OBJECTS.is_power_of_two() && MAX_INCARNATIONS.is_power_of_two());

/// The generation after `g`, skipping 0 on wrap so a zeroed capability
/// stays dead.
pub fn next_generation(g: u32) -> u32 {
    if g == u32::MAX { 1 } else { g + 1 }
}

// ---------------------------------------------------------------------------
// The table.

pub const SLOT_COUNT: usize = 1024;

const TAG_EMPTY: u64 = 0;
const TAG_RECORD: u64 = 1;
// Tags 2.. are object kinds, as CapabilityData::object_tag numbers them.
const TAG_MASK: u64 = 0xff;
/// The high half of `head` is a write sequence: odd while a writer owns it.
const SEQ_ONE: u64 = 1 << 32;

/// One inline 32-byte slot: `head` = sequence << 32 | tag. For an object
/// capability `a` = generation << 32 | object, `b` = rights, `c` = badge; for
/// a BOOT record `a` is the record's address.
#[repr(C, align(32))]
struct Slot {
    head: AtomicU64,
    a: AtomicU64,
    b: AtomicU64,
    c: AtomicU64,
}

impl Slot {
    /// Seqlock read: a consistent (tag, a, b, c), or None when empty.
    fn read(&self) -> Option<(u64, u64, u64, u64)> {
        loop {
            let head = self.head.load(Ordering::Acquire);
            if head & SEQ_ONE != 0 { core::hint::spin_loop(); continue; }
            if head & TAG_MASK == TAG_EMPTY { return None; }
            let (a, b, c) = (self.a.load(Ordering::Relaxed), self.b.load(Ordering::Relaxed), self.c.load(Ordering::Relaxed));
            fence(Ordering::Acquire);
            if self.head.load(Ordering::Relaxed) == head { return Some((head & TAG_MASK, a, b, c)); }
        }
    }

    /// Own the slot for writing if `accept(tag)`; returns the head to publish
    /// from. Writers to one slot are serialised by this CAS.
    fn lock(&self, accept: impl Fn(u64) -> bool) -> Option<u64> {
        loop {
            let head = self.head.load(Ordering::Acquire);
            if head & SEQ_ONE != 0 { core::hint::spin_loop(); continue; }
            if !accept(head & TAG_MASK) { return None; }
            if self.head.compare_exchange_weak(head, head.wrapping_add(SEQ_ONE), Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                // Seqlock writer: the odd head is ordered before the data stores.
                fence(Ordering::Release);
                return Some(head.wrapping_add(SEQ_ONE));
            }
        }
    }

    fn publish(&self, locked: u64, tag: u64, a: u64, b: u64, c: u64) {
        self.a.store(a, Ordering::Relaxed);
        self.b.store(b, Ordering::Relaxed);
        self.c.store(c, Ordering::Relaxed);
        self.head.store((locked.wrapping_add(SEQ_ONE) & !TAG_MASK) | tag, Ordering::Release);
    }
}

/// A lock-free capability table of 1,024 inline slots (32 KiB).
#[repr(align(4096))]
pub struct CapabilityTable {
    slots: [Slot; SLOT_COUNT],
    next_slot: AtomicU32,
    sealed: AtomicBool,
}

impl CapabilityTable {
    /// A new empty table straight on the heap. All-zero bytes are an empty,
    /// unsealed table, so the 32 KiB never pass through a kernel stack.
    pub fn new_boxed() -> *mut Self {
        let layout = core::alloc::Layout::new::<Self>();
        let table = unsafe { alloc::alloc::alloc_zeroed(layout) } as *mut Self;
        if table.is_null() { alloc::alloc::handle_alloc_error(layout); }
        table
    }

    pub fn seal(&self) {
        self.sealed.store(true, Ordering::SeqCst);
    }

    pub fn is_sealed(&self) -> bool {
        self.sealed.load(Ordering::Acquire)
    }

    fn slot(&self, id: u32) -> Option<&Slot> {
        if id == 0 || id as usize > SLOT_COUNT { return None; }
        Some(&self.slots[spec_clamp((id as usize).wrapping_sub(1), SLOT_COUNT)])
    }

    /// BOOT only: place `data` in the next slot (wrapping, as before).
    pub fn insert(&self, data: CapabilityData) -> u32 {
        if self.is_sealed() {
            panic!("CAP: Attempted to insert capability into sealed table");
        }
        let id = self.next_slot.fetch_add(1, Ordering::SeqCst) % SLOT_COUNT as u32 + 1;
        self.store(id, data);
        id
    }

    /// BOOT only: place `data` in slot `id`, replacing what was there.
    pub fn insert_at(&self, id: u32, data: CapabilityData) {
        if self.is_sealed() {
            panic!("CAP: Attempted to insert_at into sealed table");
        }
        if self.slot(id).is_some() { self.store(id, data); }
    }

    fn store(&self, id: u32, data: CapabilityData) {
        let Some(slot) = self.slot(id) else { return };
        let locked = slot.lock(|_| true).expect("accepts any tag");
        match data.object_tag() {
            Some((tag, cap)) => slot.publish(locked, tag as u64, Self::pack(&cap), cap.rights as u64, cap.badge),
            None => {
                // One immutable record per BOOT grant. A replaced or revoked
                // record is retained, never freed, so a racing copy is safe.
                let record = alloc::boxed::Box::leak(alloc::boxed::Box::new(Capability { id, data }));
                slot.publish(locked, TAG_RECORD, record as *const Capability as u64, 0, 0);
            }
        }
    }

    fn pack(cap: &ObjectCap) -> u64 {
        (cap.generation as u64) << 32 | cap.object as u64
    }

    fn unpack(a: u64, b: u64, c: u64) -> ObjectCap {
        ObjectCap { object: a as u32, generation: (a >> 32) as u32, rights: b as u32, badge: c }
    }

    fn decode(id: u32, tag: u64, a: u64, b: u64, c: u64) -> Option<Capability> {
        if tag == TAG_RECORD {
            return Some(unsafe { *(a as *const Capability) });
        }
        CapabilityData::from_object_tag(tag as u8, Self::unpack(a, b, c)).map(|data| Capability { id, data })
    }

    pub fn find(&self, id: u32) -> Option<Capability> {
        let (tag, a, b, c) = self.slot(id)?.read()?;
        Self::decode(id, tag, a, b, c)
    }

    /// The object capability in slot `id`, whatever its kind. Liveness is the
    /// caller's check (`OBJECTS.live`), made at the moment of use.
    pub fn object(&self, id: u32) -> Option<ObjectCap> {
        let (tag, a, b, c) = self.slot(id)?.read()?;
        (tag > TAG_RECORD).then(|| Self::unpack(a, b, c))
    }

    /// Derive a capability from the live object capability in slot `id`
    /// (see `ObjectCap::derive`), ready to `install` in a receiving table.
    /// A reply capability is one-shot and never derives.
    pub fn derive(&self, id: u32, rights: u32, badge: u64, objects: &Generations) -> Result<CapabilityData, MintError> {
        let (tag, a, b, c) = self.slot(id).and_then(Slot::read).ok_or(MintError::NotHeld)?;
        if tag <= TAG_RECORD { return Err(MintError::NotHeld); }
        if tag as u8 == CapabilityData::REPLY_TAG { return Err(MintError::Rights); }
        let cap = Self::unpack(a, b, c);
        if !objects.live(&cap) { return Err(MintError::Stale); }
        CapabilityData::from_object_tag(tag as u8, cap.derive(rights, badge)?).ok_or(MintError::NotHeld)
    }

    /// Runtime grant (D1): put an object capability in the first free slot,
    /// sealed or not. Only object kinds are accepted, so a BOOT-wired resource
    /// can never be minted after the seal.
    pub fn install(&self, data: CapabilityData) -> Result<u32, MintError> {
        let (tag, cap) = data.object_tag().ok_or(MintError::NotHeld)?;
        for (index, slot) in self.slots.iter().enumerate() {
            if let Some(locked) = slot.lock(|t| t == TAG_EMPTY) {
                slot.publish(locked, tag as u64, Self::pack(&cap), cap.rights as u64, cap.badge);
                return Ok(index as u32 + 1);
            }
        }
        Err(MintError::Full)
    }

    /// Remove one capability. A lookup that read it before the removal is
    /// ordered before it; every later lookup sees an empty slot. Before the
    /// seal BOOT may refill the slot; after it only `install` can, and only
    /// with a newly derived object capability.
    pub fn revoke(&self, id: u32) -> Option<Capability> {
        let slot = self.slot(id)?;
        let locked = slot.lock(|t| t != TAG_EMPTY)?;
        let (a, b, c) = (slot.a.load(Ordering::Relaxed), slot.b.load(Ordering::Relaxed), slot.c.load(Ordering::Relaxed));
        slot.publish(locked, TAG_EMPTY, 0, 0, 0);
        Self::decode(id, locked & TAG_MASK, a, b, c)
    }

    /// Revoke everything a terminal incarnation held; returns the count.
    pub fn revoke_all(&self) -> u32 {
        (1..=SLOT_COUNT as u32).filter(|&id| self.revoke(id).is_some()).count() as u32
    }

    /// True when no held capability satisfies `covers`. This inspects the
    /// actual slots; it is a check, not an assertion that revocation happened.
    pub fn holds_none(&self, covers: impl Fn(&CapabilityData) -> bool) -> bool {
        (1..=SLOT_COUNT as u32).filter_map(|id| self.find(id)).all(|cap| !covers(&cap.data))
    }
}
