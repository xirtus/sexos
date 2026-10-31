#![allow(dead_code)]
mod ipc { pub const MAX_DOMAINS: usize = 256; }
#[path="../crates/sex-pdx/src/app_catalog.rs"] mod app_catalog;
#[path="../crates/sex-pdx/src/history_recovery.rs"] mod recovery;
#[path="../kernel/src/capability_slot.rs"] mod capability_slot;
mod capability {
    #[derive(Clone, Copy)] pub enum CapabilityData { HistoryRecovery(crate::recovery::Grant), Domain(u32) }
    pub struct Cap { pub data: CapabilityData }
    pub struct ProtectionDomain { pub id: u32, pub value: Option<CapabilityData> }
    impl ProtectionDomain {
        pub fn find_capability(&self, slot: u32) -> Option<Cap> {
            if slot == 53 { self.value.map(|data| Cap { data }) } else { None }
        }
    }
}
#[path="../kernel/src/history_recovery.rs"] mod production;
use recovery::{Participant, RecoveryGrant};
fn roster<const N: usize>(targets: [u32; N]) -> RecoveryGrant<N> {
    RecoveryGrant { holder: 11, logical: 12, participants: targets.map(|target| Participant { target, task: target * 10 }), base: 1808, sectors: 18 }
}
fn grant() -> recovery::Grant { roster([12, 17]) }
fn body(raw: [u64; 5]) -> Result<[u64; 4], u64> {
    if raw[0] != 0 { assert_eq!(&raw[1..], &[0; 4]); Err(raw[0]) }
    else { Ok(raw[1..].try_into().unwrap()) }
}
fn decode<const N: usize>(g: RecoveryGrant<N>, targets: &[u32; N]) -> Result<RecoveryGrant<N>, u64> {
    recovery::decode_for(11, 12, targets, |kind| body(g.query(11, kind)))
}
#[test]
fn actual_catalog_two_and_kernel_query_roundtrip() {
    assert_eq!(app_catalog::SPINDLE_TARGETS, [12, 17]);
    let pd = capability::ProtectionDomain { id: 11, value: Some(capability::CapabilityData::HistoryRecovery(grant())) };
    assert_eq!(production::query(&pd, 53, 0), [0, 12, 11, 2, 1]);
    assert_eq!(production::query(&pd, 53, 1), [0, 1808, 18, 0, 0]);
    assert_eq!(production::query(&pd, 53, 2), [0, 12, 120, 0, 0]);
    assert_eq!(production::query(&pd, 53, 3), [0, 17, 170, 1, 0]);
    for kind in [4, u64::MAX] { assert_eq!(production::query(&pd, 53, kind), [1, 0, 0, 0, 0]); }
    let decoded = recovery::decode_for(11, 12, &app_catalog::SPINDLE_TARGETS, |kind| body(production::query(&pd, 53, kind))).unwrap();
    assert_eq!(decoded, grant());
}
#[test]
fn missing_wrong_kind_holder_and_full_width_slots_are_denied() {
    for id in [0, 12, 17, 3] {
        let pd = capability::ProtectionDomain { id, value: Some(capability::CapabilityData::HistoryRecovery(grant())) };
        for kind in [0, 1, 2, 3] { assert_eq!(production::query(&pd, 53, kind), [3, 0, 0, 0, 0]); }
    }
    for value in [None, Some(capability::CapabilityData::Domain(11))] {
        let pd = capability::ProtectionDomain { id: 11, value };
        assert_eq!(production::query(&pd, 53, 0), [1, 0, 0, 0, 0]);
    }
    let pd = capability::ProtectionDomain { id: 11, value: Some(capability::CapabilityData::HistoryRecovery(grant())) };
    for slot in [0, 52, 54, 1025, (1u64 << 32) | 53, u64::MAX] { assert_eq!(production::query(&pd, slot, 0), [1, 0, 0, 0, 0]); }
}
#[test]
fn generic_three_and_eight_rosters_preserve_middle_last_and_unequal_task_ids() {
    let three = roster([12, 17, 24]);
    assert_eq!(decode(three, &[12, 17, 24]), Ok(three));
    assert_eq!(three.participant(17), Some(Participant { target: 17, task: 170 }));
    let eight = roster([12, 17, 24, 25, 26, 27, 28, 29]);
    assert_eq!(decode(eight, &[12, 17, 24, 25, 26, 27, 28, 29]), Ok(eight));
    for (pd, task) in [(12, 120), (25, 250), (29, 290)] {
        assert!(eight.permits(11, pd, task));
        assert!(!eight.permits(11, pd, pd));
        assert!(!eight.permits(12, pd, task));
    }
    assert_eq!(eight.participant(23), None);
    assert_eq!(eight.query(11, 9), [0, 29, 290, 7, 0]);
}
#[test]
fn owned_roster_sizes_remain_bounded() {
    assert_eq!(core::mem::size_of::<Participant>(), 8);
    assert_eq!(core::mem::size_of::<RecoveryGrant<2>>(), 40);
    assert_eq!(core::mem::size_of::<RecoveryGrant<3>>(), 48);
    assert_eq!(core::mem::size_of::<RecoveryGrant<8>>(), 88);
}
#[test]
fn invalid_grants_never_expose_metadata_or_participants() {
    let g = grant();
    let mut bad = vec![RecoveryGrant { holder: 0, ..g }, RecoveryGrant { logical: 0, ..g },
        RecoveryGrant { base: u64::MAX, ..g }, RecoveryGrant { sectors: 17, ..g }];
    for p in [Participant::EMPTY, Participant { target: 12, task: 999 }, Participant { target: 99, task: 120 }] {
        let mut changed = g; changed.participants[1] = p; bad.push(changed);
    }
    for g in bad {
        assert!(!g.valid()); assert_eq!(g.participant(12), None); assert!(!g.permits(11, 12, 120));
        for kind in [0, 1, 2, 3] { assert_eq!(g.query(11, kind), [3, 0, 0, 0, 0]); }
    }
    assert!(!roster::<0>([]).valid());
    assert!(!roster(core::array::from_fn::<_, 65, _>(|i| i as u32 + 1)).valid());
    assert!(roster(core::array::from_fn::<_, 64, _>(|i| i as u32 + 1)).valid());
}
#[test]
fn decoder_rejects_header_count_version_extent_and_reserved_corruption() {
    let g = grant();
    for (kind, field, value) in [(0,0,17), (0,1,12), (0,2,1), (0,2,3), (0,2,65), (0,3,0), (0,3,2),
        (0,0,(1u64<<32)|12), (0,1,(1u64<<32)|11), (1,0,u64::MAX), (1,1,19), (1,2,1), (1,3,1)] {
        let result = recovery::decode_for(11, 12, &[12, 17], |k| {
            let mut b = body(g.query(11, k))?; if k == kind { b[field] = value; } Ok(b)
        });
        assert!(result.is_err(), "accepted kind={kind} field={field} value={value}");
    }
}
#[test]
fn decoder_rejects_middle_last_identity_order_duplicate_and_high_bits() {
    let g = roster([12, 17, 24, 25, 26, 27, 28, 29]);
    for kind in [3, 5, 9] {
        for (field, value) in [(0,0), (0,12), (0,(1u64<<32)|29), (1,0), (1,120), (1,(1u64<<32)|290), (2,0), (2,8), (3,1)] {
            let result = recovery::decode_for(11, 12, &[12, 17, 24, 25, 26, 27, 28, 29], |k| {
                let mut b = body(g.query(11, k))?; if k == kind { b[field] = value; } Ok(b)
            });
            assert!(result.is_err(), "accepted kind={kind} field={field} value={value}");
        }
    }
    assert!(decode(grant(), &[17, 12]).is_err());
    assert!(decode(grant(), &[12, 12]).is_err());
}
#[test]
fn truncation_at_any_query_returns_error_without_partial_grant() {
    let g = roster([12, 17, 24]);
    for missing in 0..5 {
        let mut calls = Vec::new();
        let result = recovery::decode_for(11, 12, &[12, 17, 24], |k| {
            calls.push(k); if k == missing { Err(77) } else { body(g.query(11, k)) }
        });
        assert_eq!(result, Err(77));
        assert_eq!(calls, (0..=missing).collect::<Vec<_>>());
    }
}
#[test]
fn malformed_expected_authority_is_not_queried_or_accepted() {
    for (holder, logical) in [(0, 12), (11, 0)] {
        assert_eq!(recovery::decode_for(holder, logical, &[12, 17], |_| panic!("must reject first")), Err(1));
    }
    assert_eq!(recovery::decode_for(11, 12, &[], |_| panic!("zero count")), Err(1));
    assert_eq!(recovery::decode_for(11, 12, &[1; 65], |_| panic!("oversized count")), Err(1));
}
