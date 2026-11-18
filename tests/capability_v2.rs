//! Native tests of authority v2 (plan #405 Phase 3, D1) on the production
//! capability table: inline object slots, derivation from held roots, runtime
//! install after the seal, generation revocation and incarnation death.
extern crate alloc;
mod capability {
    use crate::capability_table::ObjectCap;
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum CapabilityData { Endpoint(ObjectCap), Notification(ObjectCap), Reply(ObjectCap), Domain(u32) }
    #[derive(Clone, Copy, Debug)]
    pub struct Capability { pub id: u32, pub data: CapabilityData }
    // Same tags as kernel/src/capability.rs (checked by kernel_tags_match below).
    impl CapabilityData {
        pub const REPLY_TAG: u8 = 4;
        pub fn object_tag(&self) -> Option<(u8, ObjectCap)> {
            match *self {
                CapabilityData::Endpoint(cap) => Some((2, cap)),
                CapabilityData::Notification(cap) => Some((3, cap)),
                CapabilityData::Reply(cap) => Some((Self::REPLY_TAG, cap)),
                _ => None,
            }
        }
        pub fn from_object_tag(tag: u8, cap: ObjectCap) -> Option<Self> {
            match tag {
                2 => Some(CapabilityData::Endpoint(cap)),
                3 => Some(CapabilityData::Notification(cap)),
                Self::REPLY_TAG => Some(CapabilityData::Reply(cap)),
                _ => None,
            }
        }
    }
}
#[path = "../kernel/src/capability_table.rs"]
mod capability_table;

use capability::CapabilityData as D;
use capability_table::{rights, CapabilityTable, Generations, MintError, ObjectCap};

fn table() -> Box<CapabilityTable> {
    // The production constructor: zeroed heap memory, never a stack copy.
    unsafe { Box::from_raw(CapabilityTable::new_boxed()) }
}

fn objects() -> &'static Generations {
    Box::leak(Box::new(Generations::new()))
}

#[test]
fn slots_are_inline_and_a_new_table_is_empty() {
    // 1,024 slots of 32 bytes, page aligned: no per-capability allocation.
    assert_eq!(core::mem::size_of::<CapabilityTable>(), 36 * 1024);
    let t = table();
    assert!(!t.is_sealed());
    assert!(t.holds_none(|_| true));
    for id in [0, 1, 1024, 1025] { assert!(t.find(id).is_none()); }
}

#[test]
fn object_capabilities_round_trip_through_inline_slots() {
    let t = table();
    let cap = ObjectCap { object: 7, generation: 0xdead_beef, rights: rights::CALL | rights::READ, badge: u64::MAX - 3 };
    let id = t.install(D::Endpoint(cap)).unwrap();
    assert_eq!(t.find(id).unwrap().data, D::Endpoint(cap));
    assert_eq!(t.object(id), Some(cap));
    let n = t.install(D::Notification(ObjectCap { badge: 1, ..cap })).unwrap();
    assert_ne!(id, n);
    assert_eq!(t.find(n).unwrap().data, D::Notification(ObjectCap { badge: 1, ..cap }));
    // BOOT records still work and are not object capabilities.
    let r = t.insert(D::Domain(9));
    assert_eq!(t.find(r).unwrap().data, D::Domain(9));
    assert_eq!(t.object(r), None);
}

#[test]
fn after_the_seal_only_derived_object_capabilities_can_be_added() {
    let t = table();
    t.insert(D::Domain(1));
    t.seal();
    let mint = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| t.insert(D::Domain(2))));
    assert!(mint.is_err(), "a resource kind can never be minted after the seal");
    assert_eq!(t.install(D::Domain(2)), Err(MintError::NotHeld));
    let cap = ObjectCap { object: 1, generation: 1, rights: rights::CALL, badge: 5 };
    let id = t.install(D::Endpoint(cap)).unwrap();
    assert_eq!(id, 2, "install takes the first free slot");
    assert_eq!(t.find(id).unwrap().data, D::Endpoint(cap));
}

#[test]
fn derivation_narrows_rights_and_keeps_badges() {
    let objs = objects();
    let server = table();
    let root = objs.create(10).unwrap();
    assert_eq!((root.rights, root.badge), (rights::ALL, 0));
    let root_slot = server.install(D::Endpoint(root)).unwrap();

    // An unbadged root mints a badged, narrower child.
    let child = server.derive(root_slot, rights::CALL | rights::GRANT, 42, objs).unwrap();
    let D::Endpoint(c) = child else { panic!() };
    assert_eq!((c.object, c.generation, c.rights, c.badge), (root.object, root.generation, rights::CALL | rights::GRANT, 42));

    let client = table();
    let cs = client.install(child).unwrap();
    // The child can pass on its own badge with fewer rights, never widen or rebadge.
    assert!(client.derive(cs, rights::CALL, 42, objs).is_ok());
    assert_eq!(client.derive(cs, rights::CALL | rights::RECEIVE, 42, objs), Err(MintError::Rights));
    assert_eq!(client.derive(cs, rights::CALL, 43, objs), Err(MintError::Badge));
    // Without GRANT nothing derives.
    let leaf = client.install(client.derive(cs, rights::CALL, 42, objs).unwrap()).unwrap();
    assert_eq!(client.derive(leaf, rights::CALL, 42, objs), Err(MintError::Rights));
    // Empty slots and BOOT records are not roots.
    assert_eq!(client.derive(999, rights::CALL, 0, objs), Err(MintError::NotHeld));
    let rec = server.insert(D::Domain(3));
    assert_eq!(server.derive(rec, 0, 0, objs), Err(MintError::NotHeld));
}

#[test]
fn reply_capabilities_never_derive_and_die_with_their_caller() {
    let objs = objects();
    let t = table();
    let caller = 77;
    let reply = ObjectCap { object: caller, generation: objs.incarnation(caller), rights: rights::WRITE | rights::GRANT, badge: 0 };
    assert!(objs.caller_live(caller, reply.generation));
    let id = t.install(D::Reply(reply)).unwrap();
    assert_eq!(t.derive(id, rights::WRITE, 0, objs), Err(MintError::Rights));
    objs.end_incarnation(caller);
    assert!(!objs.caller_live(caller, reply.generation), "an outstanding reply dies with its caller");
    assert!(objs.caller_live(caller, objs.incarnation(caller)), "the next incarnation is live");
    assert!(!objs.caller_live(caller, 0));
}

#[test]
fn bumping_a_generation_revokes_every_copy_at_once() {
    let objs = objects();
    let root = objs.create(1).unwrap();
    let tables: Vec<_> = (0..4).map(|_| table()).collect();
    let server = &tables[0];
    let rs = server.install(D::Endpoint(root)).unwrap();
    let copies: Vec<_> = tables[1..].iter().enumerate()
        .map(|(i, t)| (t, t.install(server.derive(rs, rights::CALL, i as u64 + 1, objs).unwrap()).unwrap()))
        .collect();
    for (t, id) in &copies { assert!(objs.live(&t.object(*id).unwrap())); }

    let fresh = objs.revoke(root.object).unwrap();
    assert_ne!(fresh.generation, root.generation);
    for (t, id) in &copies { assert!(!objs.live(&t.object(*id).unwrap()), "stale copy still live"); }
    assert_eq!(server.derive(rs, rights::CALL, 9, objs), Err(MintError::Stale), "a stale root mints nothing");
    // The owner re-roots from the fresh generation.
    let rs2 = server.install(D::Endpoint(fresh)).unwrap();
    assert!(server.derive(rs2, rights::CALL, 9, objs).is_ok());
}

#[test]
fn destroyed_objects_are_reused_at_a_new_generation() {
    let objs = objects();
    let a = objs.create(5).unwrap();
    assert!(objs.destroy(a.object));
    assert!(!objs.destroy(a.object));
    assert!(!objs.live(&a));
    assert_eq!(objs.owner(a.object), None);
    let b = objs.create(6).unwrap();
    assert_eq!(b.object, a.object, "first free object is reused");
    assert_ne!(b.generation, a.generation, "a reused object never revives old copies");
    assert!(objs.live(&b) && !objs.live(&a));
    assert!(objs.revoke(4095).is_none() && !objs.destroy(4095), "unowned objects cannot be touched");
    assert!(!objs.live(&ObjectCap { object: 1 << 20, generation: 1, rights: 0, badge: 0 }));
}

#[test]
fn incarnation_death_destroys_only_what_that_domain_owned() {
    let objs = objects();
    let mine: Vec<_> = (0..3).map(|_| objs.create(20).unwrap()).collect();
    let theirs = objs.create(21).unwrap();
    assert_eq!(objs.end_incarnation(20), 3);
    for cap in &mine { assert!(!objs.live(cap)); }
    assert!(objs.live(&theirs));
    assert_eq!(objs.end_incarnation(20), 0);
    assert_eq!(objs.create(u32::MAX), None, "the free marker is not a domain");
}

#[test]
fn objects_run_out_cleanly() {
    let objs = objects();
    for _ in 0..capability_table::MAX_OBJECTS { assert!(objs.create(1).is_some()); }
    assert_eq!(objs.create(1), None);
    assert_eq!(objs.end_incarnation(1), capability_table::MAX_OBJECTS as u32);
    assert!(objs.create(2).is_some());
}

#[test]
fn a_full_table_refuses_install_and_revoke_all_empties_it() {
    let t = table();
    let cap = ObjectCap { object: 1, generation: 1, rights: rights::CALL, badge: 0 };
    for _ in 0..capability_table::SLOT_COUNT { t.install(D::Endpoint(cap)).unwrap(); }
    assert_eq!(t.install(D::Endpoint(cap)), Err(MintError::Full));
    let start = std::time::Instant::now();
    assert_eq!(t.revoke_all(), capability_table::SLOT_COUNT as u32);
    eprintln!("[capability.v2] revoke_all of 1024 inline slots: {:?}", start.elapsed());
    assert!(t.holds_none(|_| true));
    assert!(t.revoke(1).is_none());
    assert_eq!(t.install(D::Endpoint(cap)), Ok(1));
}

#[test]
fn revoke_returns_the_capability_once() {
    let t = table();
    let cap = ObjectCap { object: 3, generation: 2, rights: rights::RECEIVE, badge: 8 };
    let id = t.install(D::Notification(cap)).unwrap();
    assert_eq!(t.revoke(id).map(|c| c.data), Some(D::Notification(cap)));
    assert!(t.revoke(id).is_none());
    assert!(t.find(id).is_none());
}

#[test]
fn generations_skip_zero_on_wrap() {
    assert_eq!(capability_table::next_generation(u32::MAX), 1);
    assert_eq!(capability_table::next_generation(1), 2);
    let objs = objects();
    let cap = objs.create(1).unwrap();
    let mut last = cap.generation;
    for _ in 0..3 { last = objs.revoke(cap.object).unwrap().generation; }
    assert_eq!(last, cap.generation + 3);
    let pd = 3;
    for _ in 0..5 { objs.end_incarnation(pd); }
    assert_eq!(objs.incarnation(pd), 6);
    assert_eq!(objs.incarnation(5000), 0, "out-of-range domains are never live");
    assert!(!objs.caller_live(5000, 0));
}

#[test]
fn concurrent_readers_never_see_a_torn_slot() {
    let t: &'static CapabilityTable = Box::leak(table());
    let pattern = |k: u32| ObjectCap { object: k, generation: !k, rights: k & rights::ALL, badge: (k as u64) * 0x9e37_79b9_7f4a_7c15 };
    let stop: &'static std::sync::atomic::AtomicBool = Box::leak(Box::new(std::sync::atomic::AtomicBool::new(false)));
    let readers: Vec<_> = (0..6).map(|_| std::thread::spawn(move || {
        let mut seen = 0u64;
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some(cap) = t.object(1) {
                assert_eq!(cap, pattern(cap.object), "torn read");
                seen += 1;
            }
        }
        seen
    })).collect();
    for k in 1..200_000u32 {
        t.install(D::Endpoint(pattern(k))).unwrap();
        t.revoke(1).unwrap();
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for r in readers { r.join().unwrap(); }
}

#[test]
fn concurrent_installs_never_share_a_slot() {
    let t: &'static CapabilityTable = Box::leak(table());
    let workers: Vec<_> = (0..8u32).map(|w| std::thread::spawn(move || {
        (0..128u32).map(|i| t.install(D::Endpoint(ObjectCap { object: w, generation: i, rights: 0, badge: 0 })).unwrap()).collect::<Vec<_>>()
    })).collect();
    let mut ids: Vec<u32> = workers.into_iter().flat_map(|w| w.join().unwrap()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 1024);
    assert_eq!(t.install(D::Endpoint(ObjectCap { object: 0, generation: 0, rights: 0, badge: 0 })), Err(MintError::Full));
}

#[test]
fn kernel_tags_match_and_dead_kinds_are_gone() {
    let capability = include_str!("../kernel/src/capability.rs");
    for line in ["CapabilityData::Endpoint(cap) => Some((2, cap)),", "CapabilityData::Notification(cap) => Some((3, cap)),",
                 "pub const REPLY_TAG: u8 = 4;", "2 => Some(CapabilityData::Endpoint(cap)),", "3 => Some(CapabilityData::Notification(cap)),"] {
        assert!(capability.contains(line), "{line}");
    }
    for dead in ["Socket(", "Network(", "VfsNode(", "RemoteProxy(", "Spawn(", "cheri", "CapabilityKind", "GlobalCapId", "signal_handlers"] {
        assert!(!capability.contains(dead), "{dead}");
    }
}

#[test]
fn register_indexes_are_masked_after_their_bounds_check() {
    // Audit S10 (Spectre v1): every array index taken from a caller value is
    // masked to the power-of-two length after the branch, so no speculative
    // load leaves the array. Out-of-range values still answer "absent".
    let source = include_str!("../kernel/src/capability_table.rs");
    assert!(source.contains("Some(&self.slots[spec_clamp((id as usize).wrapping_sub(1), SLOT_COUNT)])"));
    for raw in ["self.slots[id", "object_owner[object as usize]", "object_generation[object as usize]", ".get(pd as usize)", ".get(object as usize)"] {
        assert!(!source.contains(raw), "{raw}");
    }
    let t = table();
    for id in [0, 1025, 2048, 1 << 20, u32::MAX] { assert!(t.find(id).is_none() && t.object(id).is_none() && t.revoke(id).is_none()); }
    let objs = objects();
    for big in [4096u32, 4096 + 1, u32::MAX - 1] {
        assert_eq!(objs.owner(big), None);
        assert!(objs.revoke(big).is_none() && !objs.destroy(big));
        assert!(!objs.live(&ObjectCap { object: big, generation: 1, rights: 0, badge: 0 }));
    }
    assert_eq!(objs.incarnation(1024), 0);
    assert_eq!(objs.end_incarnation(1024), 0);
    assert_eq!(objs.incarnation(0), 1, "an out-of-range end never lands on a masked alias");
}

#[test]
fn speculation_clamp_is_computed_not_folded() {
    // The mask comes from cmp+sbb, so it cannot be optimised away after the
    // bounds branch the way `index & (len - 1)` is.
    let source = include_str!("../kernel/src/capability_table.rs");
    assert!(source.contains(r#""cmp {i}, {l}", "sbb {m}, {m}""#));
}
