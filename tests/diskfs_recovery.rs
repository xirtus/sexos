// GPL-3.0. Production function bodies are extracted verbatim by the gate.
// Descriptor I/O, cache invalidation, persistence, and locking are host doubles.
#![allow(dead_code, static_mut_refs)]
use std::sync::{Mutex, MutexGuard};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

include!(env!("DISKFS_RECOVERY_SOURCE"));

#[macro_export]
macro_rules! serial_println { ($($arg:tt)*) => { { let _ = format_args!($($arg)*); } }; }
mod pdx { pub use crate::serial_println; }
struct HostLock<T>(Mutex<T>);
impl<T> HostLock<T> {
    fn write(&self) -> MutexGuard<'_, T> { self.0.lock().unwrap() }
}
static V4_BITMAP: HostLock<[u64; 3]> = HostLock(Mutex::new([0; 3]));
static mut V4_TABLE: [V4Entry; DISKFS_SLOTS] = [V4_EMPTY; DISKFS_SLOTS];
type Descriptor = ([V4Extent; DISKFS_V4_MAX_EXTENTS], usize);
static DESCRIPTORS: Mutex<[Option<Descriptor>; DISKFS_SLOTS]> = Mutex::new([None; DISKFS_SLOTS]);
static PERSIST_CALLS: AtomicUsize = AtomicUsize::new(0);
static DESCRIPTOR_WRITE_ERROR: AtomicU64 = AtomicU64::new(0);
static PERSIST_ERROR: AtomicU64 = AtomicU64::new(0);
static EVENTS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
static TEST_LOCK: Mutex<()> = Mutex::new(());
fn v4_indirect_read(i: usize, _: u64) -> Result<Descriptor, ()> {
    DESCRIPTORS.lock().unwrap()[i].ok_or(())
}
fn v4_indirect_write(i: usize, _: u64, extents: &[V4Extent], count: usize) -> Result<(), u64> {
    EVENTS.lock().unwrap().push("descriptor_reset");
    // Production CREATE must clear the old descriptor while the slot is
    // still free, before exposing its new name/generation to persistence.
    assert!(!v4_in_use(i));
    assert_eq!(count, 0);
    let error = DESCRIPTOR_WRITE_ERROR.load(Ordering::Relaxed);
    if error != 0 { return Err(error); }
    let mut descriptor = [V4Extent::default(); DISKFS_V4_MAX_EXTENTS];
    descriptor[..count].copy_from_slice(&extents[..count]);
    DESCRIPTORS.lock().unwrap()[i] = Some((descriptor, count));
    Ok(())
}
fn v4_cache_invalidate(_: usize) {
    EVENTS.lock().unwrap().push("cache_invalidate");
}
fn v4_persist(_: u64) -> Result<(), u64> {
    EVENTS.lock().unwrap().push("manifest_persist");
    PERSIST_CALLS.fetch_add(1, Ordering::Relaxed);
    let error = PERSIST_ERROR.load(Ordering::Relaxed);
    if error == 0 { Ok(()) } else { Err(error) }
}
fn events() -> Vec<&'static str> { EVENTS.lock().unwrap().clone() }
fn reset() -> MutexGuard<'static, ()> {
    let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe { V4_TABLE = [V4_EMPTY; DISKFS_SLOTS]; }
    *DESCRIPTORS.lock().unwrap() = [None; DISKFS_SLOTS];
    *V4_BITMAP.write() = [0; 3];
    PERSIST_CALLS.store(0, Ordering::Relaxed);
    DESCRIPTOR_WRITE_ERROR.store(0, Ordering::Relaxed);
    PERSIST_ERROR.store(0, Ordering::Relaxed);
    EVENTS.lock().unwrap().clear();
    guard
}
fn object(slot: usize, size: u32, block: Option<u64>) {
    unsafe {
        V4_TABLE[slot] = V4_EMPTY;
        V4_TABLE[slot].name[0] = b'a' + slot as u8;
        V4_TABLE[slot].size_bytes = size;
    }
    if let Some(block) = block {
        let mut extents = [V4Extent::default(); DISKFS_V4_MAX_EXTENTS];
        extents[0] = V4Extent {
            start_lba: (DISKFS_V4_POOL_BASE_LBA + block * DISKFS_V4_BLOCK_SECTORS) as u16,
            sector_count: DISKFS_V4_BLOCK_SECTORS as u16,
        };
        DESCRIPTORS.lock().unwrap()[slot] = Some((extents, 1));
    }
}

#[test]
fn crash_before_size_commit_reserves_extent_for_empty_owner() {
    let _guard = reset();
    // Crash point A: descriptor committed; old manifest still exposes size 0.
    object(3, 0, Some(0));
    v4_bitmap_rebuild(0);
    assert!(v4_in_use(3));
    assert_eq!(unsafe { V4_TABLE[3].size_bytes }, 0);
    assert!(v4_bitmap_test(&V4_BITMAP.write(), 0),
        "recovery exposed an owned block to a second object's allocator");
    assert_eq!(PERSIST_CALLS.load(Ordering::Relaxed), 0);
}

#[test]
fn allocation_after_crash_cannot_alias_empty_owners_descriptor() {
    let _guard = reset();
    object(3, 0, Some(0));
    v4_bitmap_rebuild(0);
    let mut next = [V4Extent::default(); DISKFS_V4_MAX_EXTENTS];
    assert_eq!(v4_allocate(&mut V4_BITMAP.write(), 1, &mut next, 0), Ok(1));
    let original = v4_indirect_read(3, 0).unwrap().0[0];
    assert_ne!(next[0].start_lba, original.start_lba,
        "different objects would read and overwrite the same block");
}

#[test]
fn fresh_empty_object_needs_no_descriptor() {
    let _guard = reset();
    object(3, 0, None);
    v4_bitmap_rebuild(0);
    assert!(v4_in_use(3));
    assert_eq!(*V4_BITMAP.write(), [0; 3]);
    assert_eq!(PERSIST_CALLS.load(Ordering::Relaxed), 0);
}

#[test]
fn committed_nonempty_object_stays_reserved_on_repeated_recovery() {
    let _guard = reset();
    object(3, 16, Some(0));
    v4_bitmap_rebuild(0);
    let first = *V4_BITMAP.write();
    v4_bitmap_rebuild(0);
    assert_eq!(*V4_BITMAP.write(), first);
    assert!(v4_bitmap_test(&first, 0));
    assert_eq!(PERSIST_CALLS.load(Ordering::Relaxed), 0);
}

#[test]
fn nonempty_object_with_unreadable_descriptor_is_dropped() {
    let _guard = reset();
    object(3, 16, None);
    v4_bitmap_rebuild(0);
    assert!(!v4_in_use(3));
    assert_eq!(PERSIST_CALLS.load(Ordering::Relaxed), 1);
}

#[test]
fn deleted_slots_do_not_reserve_old_descriptors() {
    let _guard = reset();
    object(3, 0, Some(0));
    unsafe { V4_TABLE[3] = V4_EMPTY; }
    v4_bitmap_rebuild(0);
    assert_eq!(*V4_BITMAP.write(), [0; 3]);
}

#[test]
fn duplicate_empty_descriptor_cannot_leave_two_live_owners() {
    let _guard = reset();
    object(3, 0, Some(0));
    object(4, 0, Some(0));
    v4_bitmap_rebuild(0);
    assert!(v4_in_use(3));
    assert!(!v4_in_use(4));
    assert!(v4_bitmap_test(&V4_BITMAP.write(), 0));
    assert_eq!(PERSIST_CALLS.load(Ordering::Relaxed), 1);
}

#[test]
fn misaligned_empty_descriptor_is_rejected() {
    let _guard = reset();
    object(3, 0, Some(0));
    DESCRIPTORS.lock().unwrap()[3].as_mut().unwrap().0[0].start_lba += 1;
    v4_bitmap_rebuild(0);
    assert!(!v4_in_use(3));
    assert_eq!(*V4_BITMAP.write(), [0; 3]);
    assert_eq!(PERSIST_CALLS.load(Ordering::Relaxed), 1);
}

#[test]
fn empty_reservation_is_idempotent_across_rebuilds() {
    let _guard = reset();
    object(3, 0, Some(0));
    v4_bitmap_rebuild(0);
    let first = *V4_BITMAP.write();
    v4_bitmap_rebuild(0);
    assert_eq!(*V4_BITMAP.write(), first);
    assert!(v4_bitmap_test(&first, 0));
    assert_eq!(PERSIST_CALLS.load(Ordering::Relaxed), 0);
}

#[test]
fn reused_empty_slot_cannot_evict_a_nonempty_block_owner_after_reboot() {
    let _guard = reset();
    // DELETE committed, then crashed before clearing slot 3's descriptor.
    object(3, 0, Some(0));
    unsafe { V4_TABLE[3] = V4_EMPTY; }
    // An already-live higher slot now owns the released block.
    object(4, 16, Some(0));
    assert_eq!(handle_diskfs_create(u64::from_le_bytes(*b"new\0\0\0\0\0"), 0, 0), 3);
    v4_bitmap_rebuild(0);
    assert!(v4_in_use(3));
    assert!(v4_in_use(4), "stale empty descriptor evicted committed block owner");
    assert!(v4_bitmap_test(&V4_BITMAP.write(), 0));
    assert_eq!(v4_indirect_read(3, 0).unwrap().1, 0);
    assert_eq!(PERSIST_CALLS.load(Ordering::Relaxed), 1);
}

#[test]
fn create_resets_descriptor_and_cache_before_manifest_publication() {
    let _guard = reset();
    assert_eq!(handle_diskfs_create(u64::from_le_bytes(*b"new\0\0\0\0\0"), 0, 0), 3);
    assert_eq!(events(), ["descriptor_reset", "cache_invalidate", "manifest_persist"]);
    assert_eq!(v4_indirect_read(3, 0).unwrap().1, 0);
}

#[test]
fn failed_descriptor_reset_does_not_publish_new_name_or_generation() {
    let _guard = reset();
    object(3, 0, Some(0));
    unsafe {
        V4_TABLE[3] = V4_EMPTY;
        V4_TABLE[3].gen = 17;
    }
    DESCRIPTOR_WRITE_ERROR.store(0xBAD_F00D, Ordering::Relaxed);
    assert_eq!(handle_diskfs_create(u64::from_le_bytes(*b"new\0\0\0\0\0"), 0, 0), 0xBAD_F00D);
    assert!(!v4_in_use(3));
    assert_eq!(unsafe { V4_TABLE[3].gen }, 17);
    assert_eq!(unsafe { V4_TABLE[3].size_bytes }, 0);
    assert_eq!(v4_indirect_read(3, 0).unwrap().1, 1);
    assert_eq!(PERSIST_CALLS.load(Ordering::Relaxed), 0);
    assert_eq!(events(), ["descriptor_reset"]);
}

#[test]
fn failed_delete_persist_retains_live_identity_and_descriptor_reservation() {
    let _guard = reset();
    object(3, 16, Some(0));
    unsafe { V4_TABLE[3].gen = 17; }
    let old = unsafe { V4_TABLE[3] };
    v4_bitmap_rebuild(0);
    PERSIST_ERROR.store(0xBAD_F00D, Ordering::Relaxed);
    assert_eq!(handle_diskfs_delete(3, 0), 0xBAD_F00D);
    assert!(v4_in_use(3), "failed delete exposed the durable owner slot to CREATE");
    assert_eq!(unsafe { V4_TABLE[3].name }, old.name);
    assert_eq!(unsafe { V4_TABLE[3].size_bytes }, old.size_bytes);
    assert_eq!(unsafe { V4_TABLE[3].gen }, old.gen);
    assert!(v4_bitmap_test(&V4_BITMAP.write(), 0));
    assert_eq!(v4_indirect_read(3, 0).unwrap().1, 1);
    assert_eq!(events(), ["manifest_persist"]);
}

#[test]
fn legacy_allocator_skips_every_block_touching_history_banks() {
    let mut fm=[0xFF;512];
    for block in 226..=229 {fm[32+block/8]&=!(1<<(block%8));}
    assert_eq!(sexfs_v0_freemap_alloc_block_in_range(&mut fm,226,229),Ok(229));
    assert_eq!(sexfs_v0_freemap_alloc_block_in_range(&mut fm,226,228),Err(messages::ERR_OVERFLOW));
    for block in 226..=228 {let before=fm;assert_eq!(sexfs_v0_freemap_alloc_specific(&mut fm,block),Err(messages::ERR_OVERFLOW));assert_eq!(before,fm);}
}
#[test]
fn legacy_extent_bounds_fence_both_journal_edges() {
    let mut p=SexfsObjectEntryV0Parsed {object_id:0,kind:0,flags:0,owner_pd:12,rights_generation:0,content_generation:0,metadata_generation:0,object_size_bytes:1,first_block:1800,extent_count:1,name_hash:0,content_hash:0,created_at_gen:0,modified_at_gen:0,checksum:0,in_use:true};
    for (lba,allowed) in [(1800,true),(1801,false),(1808,false),(1817,false),(1825,false),(1826,true)] {p.first_block=lba;assert_eq!(sexfs_v0_validate_extent_bounds(&p).is_ok(),allowed,"lba {lba}");}
}
#[test]
fn recovery_rejects_external_descriptor_exposing_history_sectors() {
    let _guard=reset();object(3,16,Some(0));
    DESCRIPTORS.lock().unwrap()[3].as_mut().unwrap().0[0].start_lba=1808;
    v4_bitmap_rebuild(0);assert!(!v4_in_use(3));assert_eq!(*V4_BITMAP.write(),[0;3]);
}
