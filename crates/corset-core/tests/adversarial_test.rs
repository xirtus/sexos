// SPDX-License-Identifier: GPL-3.0-only
//! Adversarial robustness tests: truncated buffers, misaligned transfers, corrupt sectors, zero panics!

use corset_core::device::{BlockIo, MemBlockDevice};
use corset_core::format::{format_fat32, inspect_filesystem, Fat32FormatOptions};
use corset_core::gpt_edit::{CreatePartitionOptions, GptEditor};
use corset_core::layout::{scan_disk_map, GptHeader, TYPE_MICROSOFT_BASIC_DATA};
use corset_core::smart::{AtaSmartData, NvmeSmartLog};

#[test]
fn test_adversarial_block_io() {
    let mut dev = MemBlockDevice::new_512(100).unwrap();

    // 1. Misaligned buffer (not a multiple of 512 bytes)
    let mut bad_buf = [0u8; 100];
    assert!(dev.read_sectors(0, &mut bad_buf).is_err());
    assert!(dev.write_sectors(0, &bad_buf).is_err());

    // 2. Out of bounds LBA
    let mut good_buf = [0u8; 512];
    assert!(dev.read_sectors(100, &mut good_buf).is_err());
    assert!(dev.read_sectors(99, &mut [0u8; 1024]).is_err());
    assert!(dev.read_sectors(u64::MAX - 2, &mut good_buf).is_err());

    // 3. Byte helpers out of bounds
    assert!(dev.read_bytes_at(100 * 512, &mut [0u8; 1]).is_err());
    assert!(dev.write_bytes_at(u64::MAX, &[1, 2, 3]).is_err());
}

#[test]
fn test_adversarial_gpt_parsing() {
    // 1. Truncated sector
    let short_buf = [0u8; 256];
    assert!(GptHeader::parse(&short_buf, 1).is_err());

    // 2. Garbage sector
    let garbage = [0xFFu8; 512];
    assert!(GptHeader::parse(&garbage, 1).is_err());

    // 3. Scan disk on blank zeroed device returns error, never panics
    let mut dev = MemBlockDevice::new_512(4096).unwrap();
    assert!(scan_disk_map(&mut dev).is_err());
    assert!(GptEditor::open(&mut dev).is_err());

    // 4. Zero sector count partition creation
    let mut editor = GptEditor::init_blank(&mut dev, None).unwrap();
    let zero_opts = CreatePartitionOptions::new(TYPE_MICROSOFT_BASIC_DATA, "Zero", 0);
    assert!(editor.create_partition(zero_opts).is_err());
}

#[test]
fn test_adversarial_filesystem_inspect_and_format() {
    // 1. Device with all 0xFF bytes
    let mut dev = MemBlockDevice::from_bytes(vec![0xFFu8; 4096 * 512], 512).unwrap();
    let res = inspect_filesystem(&mut dev).unwrap();
    assert_eq!(res.fs_type, corset_core::format::FilesystemType::Unknown);

    // 2. Volume too small for FAT32
    let mut tiny_dev = MemBlockDevice::new_512(1000).unwrap();
    assert!(format_fat32(&mut tiny_dev, Fat32FormatOptions::default()).is_err());
}

#[test]
fn test_adversarial_smart_parsing() {
    // Truncated buffers
    assert!(NvmeSmartLog::parse(&[0u8; 100]).is_err());
    assert!(AtaSmartData::parse(&[0u8; 300]).is_err());

    // All-zero buffers: valid parses without panic
    let zero_nvme = [0u8; 512];
    let nvme = NvmeSmartLog::parse(&zero_nvme).unwrap();
    let nvme_report = nvme.analyze();
    assert_eq!(nvme_report.overall, corset_core::smart::OverallHealth::Critical); // Available spare 0 <= threshold 0

    let zero_ata = [0u8; 512];
    let ata = AtaSmartData::parse(&zero_ata).unwrap();
    let ata_report = ata.analyze(None);
    assert_eq!(ata_report.overall, corset_core::smart::OverallHealth::Healthy);
}
