// SPDX-License-Identifier: GPL-3.0-only
//! Unit & integration tests for GPT disk layout scanner, editor, alignment, and backup recovery.

use corset_core::device::MemBlockDevice;
use corset_core::error::CorsetError;
use corset_core::gpt_edit::{CreatePartitionOptions, GptEditor};
use corset_core::layout::{
    scan_disk_map, DiskRegionKind, TYPE_BDSM_ENCRYPTED, TYPE_EFI_SYSTEM,
    TYPE_MICROSOFT_BASIC_DATA,
};

#[test]
fn test_blank_gpt_init_and_scan() {
    // 64 MiB device = 131,072 sectors @ 512B
    let mut dev = MemBlockDevice::new_512(131072).unwrap();
    let mut editor = GptEditor::init_blank(&mut dev, None).unwrap();
    editor.commit(&mut dev).unwrap();

    let map = scan_disk_map(&mut dev).unwrap();
    assert_eq!(map.total_sectors, 131072);
    assert_eq!(map.sector_size, 512);
    assert!(map.primary_valid);
    assert!(map.backup_valid);
    assert_eq!(map.partitions.len(), 0);

    // Initial map should have MBR, Primary Header, Primary Entries, FreeSpace, Backup Entries, Backup Header
    assert_eq!(map.regions.len(), 6);
    assert_eq!(map.regions[0].kind, DiskRegionKind::ProtectiveMbr);
    assert_eq!(map.regions[1].kind, DiskRegionKind::PrimaryGptHeader);
    assert_eq!(map.regions[2].kind, DiskRegionKind::PrimaryGptEntries);
    assert_eq!(map.regions[3].kind, DiskRegionKind::FreeSpace);
    assert_eq!(map.regions[4].kind, DiskRegionKind::BackupGptEntries);
    assert_eq!(map.regions[5].kind, DiskRegionKind::BackupGptHeader);

    // Free space should span from first usable (34) to last usable (131038)
    let free = &map.free_extents[0];
    assert_eq!(free.start_lba, 34);
    assert_eq!(free.end_lba, 131038);
    // 1 MiB alignment: next multiple of 2048 from 34 is 2048
    assert_eq!(free.aligned_start_lba_1mib, 2048);
}

#[test]
fn test_create_partitions_alignment_and_overlap_refusal() {
    let mut dev = MemBlockDevice::new_512(131072).unwrap();
    let mut editor = GptEditor::init_blank(&mut dev, None).unwrap();

    // 1. Create EFI System Partition (8 MiB = 16384 sectors) with 1 MiB auto-alignment
    let p1_idx = editor
        .create_partition(CreatePartitionOptions::new(TYPE_EFI_SYSTEM, "EFI System", 16384))
        .unwrap();
    assert_eq!(p1_idx, 0);
    assert_eq!(editor.entries[0].first_lba, 2048);
    assert_eq!(editor.entries[0].last_lba, 2048 + 16384 - 1);
    assert_eq!(editor.entries[0].sector_count(), 16384);

    // 2. Create Microsoft Basic Data partition (35 MiB = 71680 sectors) with 1 MiB auto-alignment
    let p2_idx = editor
        .create_partition(CreatePartitionOptions::new(TYPE_MICROSOFT_BASIC_DATA, "SexOS Data", 71680))
        .unwrap();
    assert_eq!(p2_idx, 1);
    // Next 1 MiB boundary after 2048 + 16384 - 1 = 18431 is 18432
    assert_eq!(editor.entries[1].first_lba, 18432);
    assert_eq!(editor.entries[1].last_lba, 18432 + 71680 - 1);

    // 3. Create BDSM Encrypted Partition (16 MiB = 32768 sectors)
    let p3_idx = editor
        .create_partition(CreatePartitionOptions::new(TYPE_BDSM_ENCRYPTED, "Encrypted Home", 32768))
        .unwrap();
    assert_eq!(p3_idx, 2);
    assert_eq!(editor.entries[2].first_lba, 18432 + 71680);
    assert_eq!(editor.entries[2].sector_count(), 32768);

    // 4. Overlap refusal test: attempt to create a partition overlapping partition 1
    let mut bad_opts = CreatePartitionOptions::new(TYPE_EFI_SYSTEM, "Overlapping", 4096);
    bad_opts.first_lba = Some(3000); // inside [2048..18431]
    bad_opts.align_1mib = false;
    let overlap_err = editor.create_partition(bad_opts).unwrap_err();
    match overlap_err {
        CorsetError::PartitionOverlap { first, last, .. } => {
            assert_eq!(first, 3000);
            assert_eq!(last, 3000 + 4096 - 1);
        }
        other => panic!("expected PartitionOverlap, got {:?}", other),
    }

    // Commit changes
    editor.commit(&mut dev).unwrap();

    // Verify map reflects the 3 partitions
    let map = scan_disk_map(&mut dev).unwrap();
    assert_eq!(map.partitions.len(), 3);
    assert_eq!(map.partitions[0].name, "EFI System");
    assert_eq!(map.partitions[0].type_guid, TYPE_EFI_SYSTEM);
    assert_eq!(map.partitions[1].name, "SexOS Data");
    assert_eq!(map.partitions[1].type_guid, TYPE_MICROSOFT_BASIC_DATA);
    assert_eq!(map.partitions[2].name, "Encrypted Home");
    assert_eq!(map.partitions[2].type_guid, TYPE_BDSM_ENCRYPTED);
}

#[test]
fn test_partition_rename_flags_grow_and_shrink() {
    let mut dev = MemBlockDevice::new_512(131072).unwrap();
    let mut editor = GptEditor::init_blank(&mut dev, None).unwrap();

    let p0 = editor
        .create_partition(CreatePartitionOptions::new(TYPE_MICROSOFT_BASIC_DATA, "OldName", 10000))
        .unwrap();
    assert_eq!(editor.entries[p0].first_lba, 2048);
    assert_eq!(editor.entries[p0].sector_count(), 10000);

    // Rename
    editor.rename_partition(p0, "NewSexyName").unwrap();
    assert_eq!(editor.entries[p0].name, "NewSexyName");

    // Change flags
    editor.change_partition_flags(p0, 0x0000_0000_0000_0001).unwrap();
    assert_eq!(editor.entries[p0].attributes, 1);

    // Grow partition by 5000 sectors into trailing free space
    let new_size = editor.grow_partition(p0, 5000).unwrap();
    assert_eq!(new_size, 15000);
    assert_eq!(editor.entries[p0].sector_count(), 15000);

    // Shrink partition by 3000 sectors (min 8000 allowed)
    let shrunk_size = editor.shrink_partition(p0, 3000, 8000).unwrap();
    assert_eq!(shrunk_size, 12000);
    assert_eq!(editor.entries[p0].sector_count(), 12000);

    // Attempt shrink below minimum should be refused
    let shrink_err = editor.shrink_partition(p0, 6000, 8000).unwrap_err();
    match shrink_err {
        CorsetError::PartitionShrinkTooSmall { requested, minimum } => {
            assert_eq!(requested, 6000);
            assert_eq!(minimum, 8000);
        }
        other => panic!("expected PartitionShrinkTooSmall, got {:?}", other),
    }

    // Delete partition
    editor.delete_partition(p0).unwrap();
    assert!(!editor.entries[p0].is_used());

    editor.commit(&mut dev).unwrap();
    let map = scan_disk_map(&mut dev).unwrap();
    assert_eq!(map.partitions.len(), 0);
}

#[test]
fn test_corrupt_primary_gpt_recovers_from_backup() {
    let mut dev = MemBlockDevice::new_512(131072).unwrap();
    let mut editor = GptEditor::init_blank(&mut dev, None).unwrap();
    editor
        .create_partition(CreatePartitionOptions::new(TYPE_EFI_SYSTEM, "RecoveryTest", 4096))
        .unwrap();
    editor.commit(&mut dev).unwrap();

    // Intentionally corrupt Primary GPT Header at LBA 1
    dev.corrupt_bytes(512, &[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x00, 0x00, 0x00]);

    // GptEditor::open should detect bad primary and recover from backup at end of disk!
    let mut recovered_editor = GptEditor::open(&mut dev).unwrap();
    assert!(recovered_editor.recovered_from_backup);
    assert_eq!(recovered_editor.entries[0].name, "RecoveryTest");

    // Recommitting should re-write valid primary header
    recovered_editor.commit(&mut dev).unwrap();

    // Now scan_disk_map should see both primary and backup valid again!
    let map = scan_disk_map(&mut dev).unwrap();
    assert!(map.primary_valid);
    assert!(map.backup_valid);
    assert_eq!(map.partitions.len(), 1);
    assert_eq!(map.partitions[0].name, "RecoveryTest");
}
