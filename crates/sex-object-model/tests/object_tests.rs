// SPDX-License-Identifier: GPL-3.0-only
use core::mem::{align_of, size_of};
use sex_object_model::*;

#[test]
fn test_type_sizes_and_alignments() {
    assert_eq!(size_of::<SexObjectHeader>(), 80);
    assert_eq!(align_of::<SexObjectHeader>(), 8);
    assert_eq!(size_of::<SexObjectRef>(), 16);
    assert_eq!(align_of::<SexObjectRef>(), 8);
}

#[test]
fn test_kind_roundtrips() {
    let all_kinds = [
        (SexObjectKind::RawBlob, 0),
        (SexObjectKind::AppManifest, 1),
        (SexObjectKind::AppState, 2),
        (SexObjectKind::LinenProject, 3),
        (SexObjectKind::QuilDocument, 4),
        (SexObjectKind::SpindleSession, 5),
        (SexObjectKind::BellEvent, 6),
        (SexObjectKind::CollarGrant, 8),
        (SexObjectKind::MeshFact, 9),
        (SexObjectKind::CrashReport, 10),
        (SexObjectKind::Package, 11),
    ];

    for (kind, val) in all_kinds {
        assert_eq!(kind.as_u16(), val);
        assert_eq!(SexObjectKind::from_u16(val), Some(kind));
    }

    // Reserved discriminant 7 must return None
    assert_eq!(SexObjectKind::from_u16(7), None);

    // Unknown discriminants must return None
    assert_eq!(SexObjectKind::from_u16(12), None);
    assert_eq!(SexObjectKind::from_u16(255), None);
    assert_eq!(SexObjectKind::from_u16(u16::MAX), None);
}

#[test]
fn test_object_ref_sentinels() {
    let null = SexObjectRef::NULL;
    assert!(null.is_null());
    assert_eq!(null.object_id, 0);
    assert_eq!(null.generation, 0);

    let active = SexObjectRef::new(42, 100);
    assert!(!active.is_null());
    assert_eq!(active.object_id, 42);
    assert_eq!(active.generation, 100);
}

#[test]
fn test_object_header_refs_and_flags() {
    let header = SexObjectHeader {
        object_id: 101,
        content_generation: 5,
        rights_generation: 9,
        metadata_generation: 7,
        object_size_bytes: 4096,
        first_block: 200,
        owner_pd: 1,
        kind: SexObjectKind::QuilDocument.as_u16() as u32,
        checksum: 0xABCD,
        flags: FLAG_TOMBSTONED | FLAG_SEALED,
        reserved0: 0,
        reserved1: 0,
    };

    assert_eq!(header.cap_ref(), SexObjectRef::new(101, 9));
    assert_eq!(header.object_ref(), SexObjectRef::new(101, 5));

    assert!(header.is_tombstoned());
    assert!(header.is_sealed());
    assert!(!header.is_redacted());
    assert_eq!(header.kind(), Some(SexObjectKind::QuilDocument));

    let redacted_header = SexObjectHeader {
        flags: FLAG_REDACTED,
        kind: 7, // reserved
        ..header
    };
    assert!(!redacted_header.is_tombstoned());
    assert!(!redacted_header.is_sealed());
    assert!(redacted_header.is_redacted());
    assert_eq!(redacted_header.kind(), None);

    let out_of_range_kind = SexObjectHeader {
        kind: 0x1_0000,
        ..header
    };
    assert_eq!(out_of_range_kind.kind(), None);
}
