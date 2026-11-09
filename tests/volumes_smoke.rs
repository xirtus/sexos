// SPDX-License-Identifier: GPL-3.0-only
extern crate sex_volumes as sv;
#[path = "volumes_common.rs"] mod common;
#[test] fn blank_disk_has_no_volumes() { let mut d = common::MemDisk::new(64); assert!(sv::scan(&mut d).unwrap().is_empty()); }
