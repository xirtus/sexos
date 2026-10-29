#[path = "../kernel/src/boot_frame_reservations.rs"] mod reservations;
use reservations::*;

#[test]
fn entire_dma_reservation_is_excluded_including_interior_and_last_page() {
    let mut owner = BootFrameReservations::new();
    owner.reserve(0x20_0000, 0x40_0000, |_| false).unwrap();
    assert!(!owner.contains(0x1f_f000));
    for page in 0..1024 { assert!(owner.contains(0x20_0000 + page*4096)); }
    assert!(!owner.contains(0x60_0000));
}

#[test]
fn interior_consumed_frame_collision_rejects_before_publication() {
    let mut owner = BootFrameReservations::new();
    let consumed = Range {start: 0x30_0000, end: 0x30_1000};
    assert!(owner.reserve(0x20_0000,0x40_0000,|range|range.overlaps(consumed)).is_err());
    assert!(!owner.contains(0x20_0000));
    assert!(!owner.contains(0x30_0000));
}

#[test]
fn overlapping_owners_and_bad_bounds_fail_without_losing_original_owner() {
    let mut owner = BootFrameReservations::new();
    owner.reserve(0x10_0000,0x4000,|_|false).unwrap();
    for (base,len) in [(0x10_1000,0x1000),(0xf_f000,0x2000),(0,0x1000),
        (0x20_0001,0x1000),(0x20_0000,0),(u64::MAX-4095,8192)] {
        assert!(owner.reserve(base,len,|_|false).is_err());
    }
    assert!(owner.contains(0x10_3000));
    owner.reserve(0x10_4000,0x1000,|_|false).unwrap();
}

#[test]
fn reservation_capacity_failure_keeps_all_existing_ranges() {
    let mut owner = BootFrameReservations::new();
    for i in 1..=32 { owner.reserve(i*8192,4096,|_|false).unwrap(); }
    assert!(owner.reserve(33*8192,4096,|_|false).is_err());
    for i in 1..=32 { assert!(owner.contains(i*8192)); }
    assert!(!owner.contains(33*8192));
}

#[test]
fn metadata_in_later_memmap_region_skips_actual_interval_not_page_count() {
    let mut owner = BootFrameReservations::new();
    // A has only one remaining page; two metadata pages are carved from B.
    owner.reserve(0x40_0000,8192,|_|false).unwrap();
    let candidates = [0x20_0000,0x40_0000,0x40_1000,0x40_2000];
    let frames: Vec<_> = candidates.into_iter().filter(|pa|!owner.contains(*pa)).collect();
    assert_eq!(frames, vec![0x20_0000,0x40_2000]);
}

#[test]
fn actual_bootstrap_planner_places_metadata_after_a_short_region() {
    let mut plan = BootstrapPlanner::new(0,8192);
    assert_eq!(plan.region(0x20_0000,4096),Ok(Some(Range {start:0x20_0000,end:0x20_1000})));
    assert_eq!(plan.metadata,None);
    assert_eq!(plan.region(0x40_0000,16384),Ok(Some(Range {start:0x40_2000,end:0x40_4000})));
    assert_eq!(plan.metadata,Some(Range {start:0x40_0000,end:0x40_2000}));
}

#[test]
fn actual_bootstrap_planner_skips_consumed_prefix_across_memmap_holes() {
    let mut plan = BootstrapPlanner::new(3,4096);
    assert_eq!(plan.region(0x10_0000,8192),Ok(None));
    assert_eq!(plan.region(0x40_0000,16384),Ok(Some(Range {start:0x40_2000,end:0x40_4000})));
    assert_eq!(plan.metadata,Some(Range {start:0x40_1000,end:0x40_2000}));
}

#[test]
fn actual_bootstrap_planner_fails_invalid_ranges_and_reports_missing_capacity() {
    let mut plan = BootstrapPlanner::new(0,8192);
    assert!(plan.region(4097,4096).is_err());
    assert!(plan.region(u64::MAX-4095,8192).is_err());
    assert_eq!(plan.region(0x10_0000,4096),Ok(Some(Range {start:0x10_0000,end:0x10_1000})));
    assert_eq!(plan.metadata,None);
}
