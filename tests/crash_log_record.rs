// SPDX-License-Identifier: GPL-3.0-only
//! The crash record's sector format and the disk ring's slot choice, on the host.
#![allow(dead_code)]
pub const SEXFS_V0_META_END_LBA: u64 = 47;
pub const SEXDRIVE_AP4_MULTI_BASE_LBA: u64 = 128;
pub mod program_store { pub const DIRECTORY_LBA: u64 = 2048; }
#[path = "../crates/sex-pdx/src/crash_log.rs"] mod crash_log;
use crash_log::*;

#[test]
fn round_trip_and_checksum() {
    let mut r = Record::empty(Kind::ServiceFault);
    r.boot = 0x1122; r.seq = 7; r.pd = 4; r.rip = 0xdead; r.set_name("Input");
    r.push_message(b"page fault\x01 at 8");
    let mut bytes = [0u8; RECORD_BYTES];
    r.encode(&mut bytes);
    let back = Record::decode(&bytes).unwrap();
    assert_eq!(back.name(), b"Input");
    assert_eq!(back.message(), b"page fault? at 8");
    assert!(back.same_event(&r));
    bytes[100] ^= 1;
    assert!(Record::decode(&bytes).is_none());
}
#[test]
fn ring_slot() {
    assert_eq!(next_disk_slot(&[None, None, None]), (0, 1));
    assert_eq!(next_disk_slot(&[Some(4), Some(5), None]), (2, 6));
    assert_eq!(next_disk_slot(&[Some(7), Some(5), Some(6)]), (1, 8));
}
