// SPDX-License-Identifier: GPL-3.0-only
#[path = "../crates/sex-pdx/src/memory_region.rs"]
#[allow(dead_code)]
mod memory_region;
#[path = "../crates/sex-pdx/src/dma_pool.rs"]
mod dma_pool;
use dma_pool::*;
use memory_region::*;

fn view() -> RegionView { RegionView { region_id: 1, virtual_address: 0x4000_0000_0000, length: 4 * PAGE_BYTES, dma_address: Some(0x1000_0000) } }
fn pool() -> DmaRegionPool { DmaRegionPool::new(view()).unwrap() }
fn lease() -> DmaLease { DmaLease::new(pool().reserve(PAGE_BYTES).unwrap()) }

#[test]
fn initialized_subregions_are_disjoint_and_page_aligned() {
    let mut pool = pool();
    let first = pool.reserve(16).unwrap();
    let second = pool.reserve(4097).unwrap();
    assert_eq!(first.length, PAGE_BYTES);
    assert_eq!(second.length, PAGE_BYTES * 2);
    assert_eq!(second.virtual_address, first.virtual_address + first.length);
    assert_eq!(second.physical_address, first.physical_address + first.length);
    assert_eq!(pool.used(), PAGE_BYTES * 3);
}
#[test]
fn exhausted_or_invalid_reservation_leaves_cursor_unchanged() {
    let mut pool = pool();
    assert_eq!(pool.reserve(0), Err(DmaError::InvalidSize));
    assert_eq!(pool.reserve(u64::MAX), Err(DmaError::InvalidSize));
    assert_eq!(pool.used(), 0);
    pool.reserve(PAGE_BYTES * 4).unwrap();
    assert_eq!(pool.reserve(1), Err(DmaError::Exhausted));
    assert_eq!(pool.used(), PAGE_BYTES * 4);
}
#[test]
fn malformed_or_non_dma_region_cannot_be_a_pool() {
    for region in [RegionView { dma_address: None, ..view() }, RegionView { length: 0, ..view() }, RegionView { virtual_address: USER_VA_END, ..view() }, RegionView { dma_address: Some(u64::MAX - 4095), ..view() }] {
        assert!(matches!(DmaRegionPool::new(region), Err(DmaError::InvalidRegion)));
    }
}
#[test]
fn completed_requests_reuse_fixed_backing_without_consuming_pool() {
    let mut pool = pool();
    let backing = pool.reserve(PAGE_BYTES).unwrap();
    let mut lease = DmaLease::new(backing);
    for tag in 0..100_000 {
        assert_eq!(lease.prepare(tag), Ok(backing));
        lease.mark_submitted(tag).unwrap();
        lease.complete(tag).unwrap();
    }
    assert_eq!(pool.used(), PAGE_BYTES);
}
#[test]
fn one_buffer_cannot_be_prepared_or_submitted_by_two_transactions() {
    let mut lease = lease();
    lease.prepare(7).unwrap();
    assert_eq!(lease.prepare(8), Err(DmaError::Busy));
    assert_eq!(lease.mark_submitted(8), Err(DmaError::InvalidTransition));
    lease.mark_submitted(7).unwrap();
    assert_eq!(lease.prepare(8), Err(DmaError::Busy));
}
#[test]
fn before_submission_failure_can_release_the_buffer() {
    let mut lease = lease();
    let first = lease.prepare(7).unwrap();
    lease.abort_before_submission(7).unwrap();
    assert_eq!(lease.prepare(8), Ok(first));
}
#[test]
fn after_submission_abort_or_wrong_completion_cannot_release_buffer() {
    let mut lease = lease();
    lease.prepare(7).unwrap();
    lease.mark_submitted(7).unwrap();
    assert_eq!(lease.abort_before_submission(7), Err(DmaError::InvalidTransition));
    assert_eq!(lease.complete(8), Err(DmaError::InvalidTransition));
    assert_eq!(lease.prepare(8), Err(DmaError::Busy));
    lease.complete(7).unwrap();
    assert!(lease.prepare(8).is_ok());
}
#[test]
fn timeout_quarantine_prevents_reuse_even_after_late_completion() {
    let mut lease = lease();
    lease.prepare(7).unwrap();
    lease.mark_submitted(7).unwrap();
    lease.quarantine();
    assert!(lease.is_quarantined());
    assert_eq!(lease.complete(7), Err(DmaError::InvalidTransition));
    assert_eq!(lease.abort_before_submission(7), Err(DmaError::InvalidTransition));
    assert_eq!(lease.prepare(8), Err(DmaError::Busy));
}
