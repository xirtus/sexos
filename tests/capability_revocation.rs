//! Native tests of the production capability table's sealed revocation.
extern crate alloc;
mod capability {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum CapabilityData { Domain(u32), Region(u64), Other }
    #[derive(Clone, Copy, Debug)]
    pub struct Capability { pub id: u32, pub data: CapabilityData, pub cheri_meta: u64 }
}
#[path = "../kernel/src/capability_table.rs"]
mod capability_table;

use capability::CapabilityData;
use capability_table::CapabilityTable;

fn sealed(data: &[CapabilityData]) -> Box<CapabilityTable> {
    let table = Box::new(CapabilityTable::new());
    for value in data { table.insert(*value); }
    table.seal();
    table
}

#[test]
fn revocation_prevents_every_later_lookup_without_disturbing_others() {
    let table = sealed(&[CapabilityData::Domain(9), CapabilityData::Region(0x1000), CapabilityData::Domain(15)]);
    assert_eq!(table.revoke(2).map(|c| c.data), Some(CapabilityData::Region(0x1000)));
    assert!(table.find(2).is_none());
    assert!(table.revoke(2).is_none(), "second revocation has nothing to return");
    assert_eq!(table.find(1).map(|c| c.data), Some(CapabilityData::Domain(9)));
    assert_eq!(table.find(3).map(|c| c.data), Some(CapabilityData::Domain(15)));
}

#[test]
fn revocation_never_creates_authority() {
    let table = sealed(&[CapabilityData::Domain(9)]);
    for id in [0, 2, 1024, 1025, u32::MAX] {
        assert!(table.revoke(id).is_none());
        assert!(table.find(id).is_none());
    }
    assert_eq!(table.revoke_all(), 1);
    assert_eq!(table.revoke_all(), 0);
    assert!(table.holds_none(|_| true));
}

#[test]
fn sealed_table_cannot_refill_a_revoked_slot() {
    let table = sealed(&[CapabilityData::Domain(9)]);
    table.revoke(1);
    let refill = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| table.insert_at(1, CapabilityData::Domain(9))));
    assert!(refill.is_err());
    let mint = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| table.insert(CapabilityData::Other)));
    assert!(mint.is_err());
    assert!(table.find(1).is_none());
}

#[test]
fn holds_none_inspects_actual_slots() {
    let table = sealed(&[CapabilityData::Domain(9), CapabilityData::Region(0x2000)]);
    let region = |d: &CapabilityData| matches!(d, CapabilityData::Region(_));
    assert!(!table.holds_none(region));
    table.revoke(2);
    assert!(table.holds_none(region));
    assert!(!table.holds_none(|_| true));
}

#[test]
fn concurrent_lookups_see_the_capability_or_nothing() {
    let table: &'static CapabilityTable = Box::leak(sealed(&[CapabilityData::Domain(15)]));
    let readers: Vec<_> = (0..8).map(|_| std::thread::spawn(move || {
        let mut absent_seen = false;
        for _ in 0..20_000 {
            match table.find(1) {
                Some(cap) => { assert!(!absent_seen, "revoked slot reappeared"); assert_eq!(cap.data, CapabilityData::Domain(15)); }
                None => absent_seen = true,
            }
        }
    })).collect();
    table.revoke(1);
    for reader in readers { reader.join().unwrap(); }
    assert!(table.find(1).is_none());
}

#[test]
fn stub_revocation_proof_is_gone() {
    let source = include_str!("../kernel/src/capability_table.rs");
    let capability = include_str!("../kernel/src/capability.rs");
    assert!(!source.contains("verify_revocation") && !capability.contains("verify_revocation"));
    let retire = include_str!("../kernel/src/app_resources.rs");
    assert!(retire.contains("(*target.cap_table).revoke_all()"));
    assert!(retire.contains("(*target.cap_table).holds_none(|_|true)"));
}
