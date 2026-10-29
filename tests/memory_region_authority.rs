// SPDX-License-Identifier: GPL-3.0-only
#[path = "../crates/sex-pdx/src/memory_region.rs"]
mod memory_region;
use memory_region::*;

fn dma() -> RegionGrant {
    RegionGrant { region_id: 1, virtual_base: 0x4000_0000_0000, length: 0x4000,
        rights: QUERY_VIEW | READ | WRITE | DMA_ADDRESS, dma_base: Some(0x1000_0000), peer_pd: None }
}
fn shared() -> RegionGrant {
    RegionGrant { dma_base: None, peer_pd: Some(11), rights: QUERY_VIEW | READ | WRITE, ..dma() }
}
fn request() -> ViewRequest { ViewRequest { offset: 0, length: PAGE_BYTES, rights: QUERY_VIEW | READ } }

#[test]
fn missing_or_wrong_kind_capability_denies_even_a_well_formed_request() {
    assert_eq!(resolve_view(None, request()), Err(RegionError::InvalidCapability));
}

#[test]
fn metadata_requires_valid_query_capability_and_reports_boot_peer() {
    assert_eq!(resolve_info(None), Err(RegionError::InvalidCapability));
    let grant = shared();
    assert_eq!(resolve_info(Some(&grant)), Ok(MemoryRegionInfo {
        region_id: grant.region_id, length: grant.length, rights: grant.rights, peer_pd: Some(11),
    }));
    let no_query = RegionGrant { rights: READ, ..grant };
    assert_eq!(resolve_info(Some(&no_query)), Err(RegionError::PermissionDenied));
    let malformed = RegionGrant { length: 0, ..grant };
    assert_eq!(resolve_info(Some(&malformed)), Err(RegionError::InvalidCapability));
}

#[test]
fn full_register_rights_cannot_alias_lower_authorized_bits() {
    let grant = dma();
    assert!(resolve_register_view(Some(&grant), 0, PAGE_BYTES, (QUERY_VIEW | READ) as u64).is_ok());
    for high_bit in [8, 16, 32, 63] {
        assert_eq!(resolve_register_view(Some(&grant), 0, PAGE_BYTES,
            (1u64 << high_bit) | (QUERY_VIEW | READ) as u64), Err(RegionError::InvalidRequest));
    }
}
#[test]
fn exact_view_is_idempotent_and_does_not_mutate_the_grant() {
    let grant = shared();
    let before = grant;
    let first = resolve_view(Some(&grant), request()).unwrap();
    for _ in 0..1000 { assert_eq!(resolve_view(Some(&grant), request()), Ok(first)); }
    assert_eq!(grant, before);
    assert_eq!(first.virtual_address, grant.virtual_base);
    assert_eq!(first.dma_address, None);
}
#[test]
fn query_permission_is_separate_from_cpu_access_and_dma_address_exposure() {
    let mut grant = dma();
    grant.rights &= !QUERY_VIEW;
    assert_eq!(resolve_view(Some(&grant), request()), Err(RegionError::PermissionDenied));
    let grant = dma();
    assert_eq!(resolve_view(Some(&grant), request()).unwrap().dma_address, None);
    let req = ViewRequest { rights: QUERY_VIEW | DMA_ADDRESS, ..request() };
    assert_eq!(resolve_view(Some(&grant), req).unwrap().dma_address, grant.dma_base);
    assert_eq!(resolve_view(Some(&shared()), req), Err(RegionError::PermissionDenied));
}
#[test]
fn readonly_shared_view_cannot_be_upgraded_to_write() {
    let grant = RegionGrant { rights: QUERY_VIEW | READ, ..shared() };
    assert!(resolve_view(Some(&grant), request()).is_ok());
    assert_eq!(resolve_view(Some(&grant), ViewRequest { rights: QUERY_VIEW | WRITE, ..request() }), Err(RegionError::PermissionDenied));
}
#[test]
fn exact_end_allowed_but_crossing_or_wrapping_range_denied() {
    let grant = dma();
    let last = ViewRequest { offset: grant.length - 1, length: 1, ..request() };
    assert!(resolve_view(Some(&grant), last).is_ok());
    for req in [ViewRequest { length: 2, ..last }, ViewRequest { offset: u64::MAX, length: 2, ..request() }, ViewRequest { offset: grant.length, ..request() }] {
        assert_eq!(resolve_view(Some(&grant), req), Err(RegionError::OutOfRange));
    }
}
#[test]
fn zero_length_and_unknown_or_missing_query_right_are_invalid() {
    for req in [ViewRequest { length: 0, ..request() }, ViewRequest { rights: QUERY_VIEW | 0x80, ..request() }, ViewRequest { rights: READ, ..request() }] {
        assert_eq!(resolve_view(Some(&dma()), req), Err(RegionError::InvalidRequest));
    }
}
#[test]
fn malformed_boot_metadata_cannot_escape_the_region_arithmetic() {
    for grant in [
        RegionGrant { region_id: 0, ..dma() },
        RegionGrant { virtual_base: u64::MAX - 4095, ..dma() },
        RegionGrant { virtual_base: USER_VA_END - PAGE_BYTES, ..dma() },
        RegionGrant { virtual_base: 1, ..dma() },
        RegionGrant { length: 0, ..dma() },
        RegionGrant { length: u64::MAX, ..dma() },
        RegionGrant { dma_base: Some(PHYSICAL_END - PAGE_BYTES), ..dma() },
        RegionGrant { dma_base: Some(1), ..dma() },
        RegionGrant { dma_base: None, ..dma() },
        RegionGrant { rights: QUERY_VIEW | READ, ..dma() },
    ] {
        assert_eq!(resolve_view(Some(&grant), request()), Err(RegionError::InvalidCapability));
    }
}
#[test]
fn physical_translation_uses_only_the_authorized_region_offset() {
    let grant = dma();
    let req = ViewRequest { offset: 4096, length: 512, rights: QUERY_VIEW | READ | DMA_ADDRESS };
    let view = resolve_view(Some(&grant), req).unwrap();
    assert_eq!(view.virtual_address, grant.virtual_base + 4096);
    assert_eq!(view.dma_address, Some(grant.dma_base.unwrap() + 4096));
    assert_eq!(view.length, 512);
}
#[test]
fn peer_access_binds_authenticated_sender_without_numeric_aliases() {
    let grant = shared();
    assert!(resolve_peer_view(Some(&grant), 11, request()).is_ok());
    for caller in [0, 1, 267, 1035, u32::MAX] {
        assert_eq!(resolve_peer_view(Some(&grant), caller, request()), Err(RegionError::WrongPeer));
    }
    assert_eq!(resolve_peer_view(Some(&dma()), 11, request()), Err(RegionError::WrongPeer));
}
#[test]
fn knowing_another_regions_addresses_does_not_supply_its_grant() {
    let first = dma();
    let second = RegionGrant { region_id: 2, virtual_base: first.virtual_base + first.length, dma_base: Some(first.dma_base.unwrap() + first.length), ..first };
    assert_eq!(resolve_view(Some(&first), ViewRequest { offset: second.virtual_base - first.virtual_base, ..request() }), Err(RegionError::OutOfRange));
    assert_eq!(resolve_view(None, request()), Err(RegionError::InvalidCapability));
}
