// SPDX-License-Identifier: GPL-3.0-only
//! Intel RAID (IMSM, "RAID On" laptops like the x17): the metadata at the end of
//! each disk, assembled from the disk's last sectors, then every question sexdrive
//! asks of it: which member is which disk (by the serial the drive reports), is
//! the volume readable with these members present, and where does a read land.
#![cfg_attr(all(fuzzing, feature = "libfuzzer"), no_main)]
use sex_imsm::{self as imsm, MAX_MPB_SECTORS, SECTOR};
use sexos_driver_fuzz::Src;

fn run(d: &[u8]) {
    let mut s = Src::new(d);
    let present = s.u8();
    let lba = s.u64();
    let sectors = s.u32() as u64;
    let serial = s.take(20);
    let tail = s.rest();
    let _ = imsm::mpb_sectors(tail);
    let mut out = Box::new([0u8; MAX_MPB_SECTORS * SECTOR]);
    let _ = imsm::assemble(tail, &mut out);
    for m in [imsm::from_tail(tail), imsm::parse(tail)].into_iter().flatten() {
        let _ = m.disk_for_serial(serial);
        let mut n = [0u8; 16];
        let _ = imsm::normalise_serial(serial, &mut n);
        for disk in m.disks() { let _ = (disk.serial(), disk.failed()); }
        for (i, v) in m.volumes().iter().enumerate() {
            let _ = (v.name(), v.uses_disk(i), v.slot_of(i), v.readable(present));
            if let Some(e) = v.map(lba, sectors, present) { let _ = e; }
            let _ = v.map(lba % v.sectors.max(1), 1 + sectors % 256, present);
        }
        let _ = m.same_set(&m);
    }
}

fn seeds() -> Vec<Vec<u8>> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/sex-imsm/tests/fixtures");
    std::fs::read_dir(dir).map(|r| r.filter_map(|e| {
        let tail = std::fs::read(e.ok()?.path()).ok()?;
        let mut v = vec![0x03, 0, 0x10, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0];
        v.extend_from_slice(b"WD-0123456789       ");
        v.extend_from_slice(&tail);
        Some(v)
    }).collect()).unwrap_or_default()
}

sexos_driver_fuzz::driver_target!(run, seeds);
