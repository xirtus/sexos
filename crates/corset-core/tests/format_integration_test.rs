// SPDX-License-Identifier: GPL-3.0-only
//! Cross-crate integration tests for FAT32, exFAT, and SexFS formatting and inspection.

use corset_core::device::{MemBlockDevice, PartitionIo, SexVolumesAdapter};
use corset_core::format::{
    format_exfat, format_fat32, format_sexfs, inspect_filesystem, ExFatFormatOptions,
    Fat32FormatOptions, FilesystemType,
};
use corset_core::gpt_edit::{CreatePartitionOptions, GptEditor};
use corset_core::layout::TYPE_MICROSOFT_BASIC_DATA;
use sex_volumes::exfat::ExFat;
use sex_volumes::fat::Fat;
use sex_volumes::Volume;

#[test]
fn test_fat32_format_and_cross_mount_rw() {
    // 64 MiB total device: create a 35 MiB partition (71680 sectors @ 512B)
    let mut dev = MemBlockDevice::new_512(131072).unwrap();
    let mut editor = GptEditor::init_blank(&mut dev, None).unwrap();
    let part_idx = editor
        .create_partition(CreatePartitionOptions::new(
            TYPE_MICROSOFT_BASIC_DATA,
            "Fat32Part",
            71680,
        ))
        .unwrap();
    editor.commit(&mut dev).unwrap();

    let (base_lba, sector_count) = (
        editor.entries[part_idx].first_lba,
        editor.entries[part_idx].sector_count(),
    );

    // Format partition as FAT32 with label "SEXOSBOOT"
    {
        let mut part = PartitionIo::new(&mut dev, base_lba, sector_count).unwrap();
        format_fat32(
            &mut part,
            Fat32FormatOptions {
                volume_label: "SEXOSBOOT",
                sectors_per_cluster: Some(1),
            },
        )
        .unwrap();
    }

    // Inspect filesystem
    {
        let mut part = PartitionIo::new(&mut dev, base_lba, sector_count).unwrap();
        let inspection = inspect_filesystem(&mut part).unwrap();
        assert_eq!(inspection.fs_type, FilesystemType::Fat32);
        assert_eq!(inspection.label, "SEXOSBOOT");
    }

    // Mount using sex-volumes, create directory, write file, read back!
    {
        let mut part = PartitionIo::new(&mut dev, base_lba, sector_count).unwrap();
        let mut adapter = SexVolumesAdapter(&mut part);
        let mut vol = Fat::mount(&mut adapter).unwrap();
        assert_eq!(vol.label(), "SEXOSBOOT");

        // Write a test file in root
        vol.create(&mut adapter, b"/hello.txt").unwrap();
        let test_payload = b"SexOS BDSM Storage Stack: FAT32 cross-crate verification passed!";
        vol.write(&mut adapter, b"/hello.txt", 0, test_payload).unwrap();
        vol.sync(&mut adapter).unwrap();

        // Read it back
        let mut read_buf = [0u8; 128];
        let bytes_read = vol.read(&mut adapter, b"/hello.txt", 0, &mut read_buf).unwrap();
        assert_eq!(&read_buf[..bytes_read], test_payload);

        // List directory entries
        let entries = vol.list(&mut adapter, b"/").unwrap();
        assert!(entries.iter().any(|e| e.name == "hello.txt"));
    }
}

#[test]
fn test_exfat_format_and_cross_mount() {
    // 32 MiB partition (65536 sectors @ 512B)
    let mut dev = MemBlockDevice::new_512(65536).unwrap();
    format_exfat(
        &mut dev,
        ExFatFormatOptions {
            volume_label: "EXFAT_TEST",
        },
    )
    .unwrap();

    // Inspect filesystem
    let inspection = inspect_filesystem(&mut dev).unwrap();
    assert_eq!(inspection.fs_type, FilesystemType::ExFat);
    assert_eq!(inspection.label, "EXFAT_TEST");

    // Mount with sex-volumes
    let mut adapter = SexVolumesAdapter(&mut dev);
    let mut vol = ExFat::mount(&mut adapter).unwrap();
    assert_eq!(vol.label(), "EXFAT_TEST");

    // Create file, write data, read data
    vol.create(&mut adapter, b"/note.txt").unwrap();
    let data = b"exFAT mounted cleanly in SexOS userspace!";
    vol.write(&mut adapter, b"/note.txt", 0, data).unwrap();
    vol.sync(&mut adapter).unwrap();

    let mut buf = [0u8; 64];
    let n = vol.read(&mut adapter, b"/note.txt", 0, &mut buf).unwrap();
    assert_eq!(&buf[..n], data);
}

#[test]
fn test_sexfs_format_and_inspection() {
    // 8 MiB partition = 16384 sectors @ 512B = 2048 4KiB blocks
    let mut dev = MemBlockDevice::new_512(16384).unwrap();
    format_sexfs(&mut dev).unwrap();

    let inspection = inspect_filesystem(&mut dev).unwrap();
    assert_eq!(inspection.fs_type, FilesystemType::SexFs);
    assert_eq!(inspection.label, "SexFS");
}

struct SparseBlockDevice {
    sectors: u64,
    written: std::collections::HashMap<u64, [u8; 512]>,
}

impl SparseBlockDevice {
    fn new(sectors: u64) -> Self {
        Self {
            sectors,
            written: std::collections::HashMap::new(),
        }
    }

    fn reset(&mut self, sectors: u64) {
        self.sectors = sectors;
        self.written.clear();
    }
}

impl corset_core::device::BlockIo for SparseBlockDevice {
    fn sector_size(&self) -> u32 {
        512
    }

    fn sector_count(&self) -> u64 {
        self.sectors
    }

    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), corset_core::error::CorsetError> {
        if buf.is_empty() || buf.len() % 512 != 0 {
            return Err(corset_core::error::CorsetError::InvalidParameter("unaligned read"));
        }
        let count = (buf.len() / 512) as u64;
        if lba.checked_add(count).map_or(true, |end| end > self.sectors) {
            return Err(corset_core::error::CorsetError::InvalidParameter("read out of bounds"));
        }
        for (i, chunk) in buf.chunks_exact_mut(512).enumerate() {
            if let Some(sec) = self.written.get(&(lba + i as u64)) {
                chunk.copy_from_slice(sec);
            } else {
                chunk.fill(0);
            }
        }
        Ok(())
    }

    fn write_sectors(&mut self, lba: u64, buf: &[u8]) -> Result<(), corset_core::error::CorsetError> {
        if buf.is_empty() || buf.len() % 512 != 0 {
            return Err(corset_core::error::CorsetError::InvalidParameter("unaligned write"));
        }
        let count = (buf.len() / 512) as u64;
        if lba.checked_add(count).map_or(true, |end| end > self.sectors) {
            return Err(corset_core::error::CorsetError::InvalidParameter("write out of bounds"));
        }
        for (i, chunk) in buf.chunks_exact(512).enumerate() {
            let cur = lba + i as u64;
            if chunk.iter().all(|&b| b == 0) {
                self.written.remove(&cur);
            } else {
                let mut arr = [0u8; 512];
                arr.copy_from_slice(chunk);
                self.written.insert(cur, arr);
            }
        }
        Ok(())
    }
}

#[test]
fn test_fat32_and_exfat_every_size_33_to_255_mib() {
    let mut dev = SparseBlockDevice::new(0);
    for mib in 33u64..=255 {
        let sectors = (mib * 1024 * 1024) / 512;

        // 1. FAT32 format, inspect, mount, and write/read roundtrip (catches 64 MiB FAT oscillation)
        dev.reset(sectors);
        format_fat32(
            &mut dev,
            Fat32FormatOptions {
                volume_label: "FAT32SWEEP",
                sectors_per_cluster: None,
            },
        )
        .unwrap_or_else(|e| panic!("FAT32 format failed at {mib} MiB ({sectors} sectors): {e:?}"));

        let fat_info = inspect_filesystem(&mut dev)
            .unwrap_or_else(|e| panic!("FAT32 inspect failed at {mib} MiB: {e:?}"));
        assert_eq!(fat_info.fs_type, FilesystemType::Fat32, "expected FAT32 at {mib} MiB");
        assert_eq!(fat_info.label, "FAT32SWEEP");

        {
            let mut adapter = SexVolumesAdapter(&mut dev);
            let mut vol = Fat::mount(&mut adapter)
                .unwrap_or_else(|e| panic!("FAT32 mount failed at {mib} MiB: {e:?}"));
            vol.create(&mut adapter, b"/check.bin").unwrap();
            vol.write(&mut adapter, b"/check.bin", 0, b"ok").unwrap();
            vol.sync(&mut adapter).unwrap();
            let mut out = [0u8; 4];
            let n = vol.read(&mut adapter, b"/check.bin", 0, &mut out).unwrap();
            assert_eq!(&out[..n], b"ok");
        }

        // 2. exFAT format, inspect, mount, and write/read roundtrip (catches >128 MiB bitmap overflow)
        dev.reset(sectors);
        format_exfat(
            &mut dev,
            ExFatFormatOptions {
                volume_label: "EXFATSWEEP",
            },
        )
        .unwrap_or_else(|e| panic!("exFAT format failed at {mib} MiB ({sectors} sectors): {e:?}"));

        let exfat_info = inspect_filesystem(&mut dev)
            .unwrap_or_else(|e| panic!("exFAT inspect failed at {mib} MiB: {e:?}"));
        assert_eq!(exfat_info.fs_type, FilesystemType::ExFat, "expected exFAT at {mib} MiB");
        assert_eq!(exfat_info.label, "EXFATSWEEP");
        if mib > 128 {
            assert!(
                exfat_info.cluster_bytes.unwrap_or(0) >= 8192,
                "exFAT >128 MiB ({mib} MiB) must scale above 4 KiB clusters"
            );
        }

        {
            let mut adapter = SexVolumesAdapter(&mut dev);
            let mut vol = ExFat::mount(&mut adapter)
                .unwrap_or_else(|e| panic!("exFAT mount failed at {mib} MiB: {e:?}"));
            vol.create(&mut adapter, b"/check.bin").unwrap();
            vol.write(&mut adapter, b"/check.bin", 0, b"ok").unwrap();
            vol.sync(&mut adapter).unwrap();
            let mut out = [0u8; 4];
            let n = vol.read(&mut adapter, b"/check.bin", 0, &mut out).unwrap();
            assert_eq!(&out[..n], b"ok");
        }
    }
}

#[test]
fn test_exfat_multi_cluster_bitmap_16_gib_sparse() {
    // 16 GiB @ 32 KiB clusters => ~524,288 clusters => ~64 KiB bitmap (2 clusters chained in FAT)
    let sectors = (16 * 1024 * 1024 * 1024u64) / 512;
    let mut dev = SparseBlockDevice::new(sectors);
    format_exfat(
        &mut dev,
        ExFatFormatOptions {
            volume_label: "BIG_EXFAT",
        },
    )
    .unwrap();

    let info = inspect_filesystem(&mut dev).unwrap();
    assert_eq!(info.fs_type, FilesystemType::ExFat);
    assert_eq!(info.label, "BIG_EXFAT");
    assert_eq!(info.cluster_bytes, Some(32768));

    let mut adapter = SexVolumesAdapter(&mut dev);
    let mut vol = ExFat::mount(&mut adapter).unwrap();
    vol.create(&mut adapter, b"/large.txt").unwrap();
    vol.write(&mut adapter, b"/large.txt", 0, b"multi-cluster bitmap works").unwrap();
    vol.sync(&mut adapter).unwrap();
}
