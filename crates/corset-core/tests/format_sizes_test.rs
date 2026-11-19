// SPDX-License-Identifier: GPL-3.0-only
//! FAT32 and exFAT at sizes a stick's partitions take: each mounts in sex-volumes with
//! its label. (64 MiB of FAT32 once came out one FAT sector short; exFAT over 128 MiB
//! once had a bitmap larger than the one cluster it was given.)
use corset_core::*;
use sex_volumes::{exfat::ExFat, fat::Fat, Volume};

#[test]
fn fat32_and_exfat_mount_at_every_size() {
    for mib in [33u64, 35, 40, 64, 100, 129, 154, 200, 255] {
        let n = mib * 2048;
        let mut d = MemBlockDevice::new_512(n).unwrap();
        format_fat32(&mut d, Fat32FormatOptions { volume_label: "F", sectors_per_cluster: None }).unwrap_or_else(|e| panic!("FAT32 {mib} MiB: {e}"));
        let v = Fat::mount(&mut SexVolumesAdapter(&mut d)).unwrap();
        assert_eq!(v.label().trim(), "F");
        let mut d = MemBlockDevice::new_512(n).unwrap();
        format_exfat(&mut d, ExFatFormatOptions { volume_label: "E" }).unwrap_or_else(|e| panic!("exFAT {mib} MiB: {e}"));
        let v = ExFat::mount(&mut SexVolumesAdapter(&mut d)).unwrap();
        assert_eq!(v.label().trim(), "E");
    }
}
