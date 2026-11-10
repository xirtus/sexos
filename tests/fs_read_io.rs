#![allow(dead_code)]
#[path="../crates/sex-pdx/src/app_catalog.rs"] mod app_catalog;
#[path="../crates/sex-pdx/src/fs.rs"] mod fs;
use fs::ReadIoStats as Stats;

#[test]
fn stats_wire_is_exact_and_rejects_partial_or_extra_records() {
    let value = Stats { requests: 1, device_calls: 0x1234, device_bytes: 0x12345678 };
    let mut out = [0xaa; 32];
    value.encode(&mut out).unwrap();
    assert_eq!(&out[..24], &[1,0,0,0,0,0,0,0, 0x34,0x12,0,0,0,0,0,0, 0x78,0x56,0x34,0x12,0,0,0,0]);
    assert_eq!(&out[24..], &[0xaa; 8]);
    assert_eq!(Stats::decode(&out[..24]), Some(value));
    for n in 0..32 { if n != 24 { assert_eq!(Stats::decode(&out[..n]), None); } }
    let mut short = [0xaa; 23]; assert_eq!(value.encode(&mut short), None); assert_eq!(short, [0xaa; 23]);
}
#[test]
fn reset_or_reordered_counters_cannot_wrap_into_a_large_read() {
    let before = Stats { requests: 10, device_calls: 30, device_bytes: 8192 };
    let after = Stats { requests: 12, device_calls: 34, device_bytes: 16384 };
    assert_eq!(after.since(before), Some(Stats { requests: 2, device_calls: 4, device_bytes: 8192 }));
    for reset in [Stats::ZERO, Stats { requests: 9, ..after }, Stats { device_calls: 29, ..after }, Stats { device_bytes: 4096, ..after }] {
        assert_eq!(reset.since(before), None);
    }
}
