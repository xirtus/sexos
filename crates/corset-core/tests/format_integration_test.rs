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
