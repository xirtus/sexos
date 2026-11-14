// SPDX-License-Identifier: GPL-3.0-only
//! Adversarial robustness tests: truncated buffers, misaligned transfers, corrupt sectors, zero panics!

use corset_core::crypt_flow::{
    erase_encrypted_header, format_encrypted_volume, unlock_encrypted_volume, Cost,
};
use corset_core::device::{BlockIo, MemBlockDevice, PartitionIo};
use corset_core::format::{
    format_exfat, format_fat32, format_sexfs, inspect_filesystem, ExFatFormatOptions,
    Fat32FormatOptions,
};
use corset_core::gpt_edit::{CreatePartitionOptions, GptEditor};
use corset_core::layout::{
    scan_disk_map, GptHeader, PartitionEntry, TYPE_MICROSOFT_BASIC_DATA,
};
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
fn test_adversarial_partition_io_capabilities() {
    // 100 sector device, partition spans [10..30] (20 sectors)
    let mut dev = MemBlockDevice::new_512(100).unwrap();
    let mut part = PartitionIo::new(&mut dev, 10, 20).unwrap();
    assert_eq!(part.sector_count(), 20);

    let mut buf = [0u8; 512];

    // Read within partition bounds works
    assert!(part.read_sectors(0, &mut buf).is_ok());
    assert!(part.read_sectors(19, &mut buf).is_ok());

    // Escaping past end of partition (LBA 20) is strictly rejected
    assert!(part.read_sectors(20, &mut buf).is_err());
    assert!(part.write_sectors(20, &buf).is_err());

    // Overflowing LBA is strictly rejected
    assert!(part.read_sectors(u64::MAX, &mut buf).is_err());
    assert!(part.write_sectors(u64::MAX, &buf).is_err());

    // Misaligned buffer is rejected
    assert!(part.read_sectors(0, &mut [0u8; 256]).is_err());
    assert!(part.write_sectors(0, &[0u8; 256]).is_err());
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

    // 5. Absurd partition entry count (e.g. 0xFFFF_FFFF or 5000 entries > 4096 cap)
    let mut fake_gpt = editor.header.encode();
    // Overwrite number_of_partition_entries at offset 80 with 0xFFFF_FFFF
    fake_gpt[80..84].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    assert!(GptHeader::parse(&fake_gpt, 1).is_err());

    // 6. Header size too small (< 92) or too large (> 512)
    let mut fake_hdr_sz = editor.header.encode();
    fake_hdr_sz[12..16].copy_from_slice(&91u32.to_le_bytes());
    assert!(GptHeader::parse(&fake_hdr_sz, 1).is_err());

    // 7. Backup GPT header pointing to LBA 2 for entries (must sit before backup header, NOT at LBA 2)
    let mut fake_backup = editor.header.encode();
    fake_backup[24..32].copy_from_slice(&4095u64.to_le_bytes()); // MyLBA = 4095
    fake_backup[32..40].copy_from_slice(&1u64.to_le_bytes());    // AlternateLBA = 1
    fake_backup[72..80].copy_from_slice(&2u64.to_le_bytes());    // PartitionEntryLBA = 2 (illegal for backup!)
    assert!(GptHeader::parse(&fake_backup, 4095).is_err());
}

#[test]
fn test_adversarial_utf16le_unpaired_surrogates() {
    // Raw partition entry slice of 128 bytes with unpaired surrogate 0xD800 in partition name
    let mut raw_entry = [0u8; 128];
    // Guid and LBAs
    raw_entry[0..16].copy_from_slice(&[1u8; 16]); // Non-zero type
    raw_entry[32..40].copy_from_slice(&2048u64.to_le_bytes());
    raw_entry[40..48].copy_from_slice(&4096u64.to_le_bytes());
    // Name at 56..128: 0xD800 (unpaired high surrogate) followed by 0x0000 (terminator)
    raw_entry[56] = 0x00;
    raw_entry[57] = 0xD8;
    raw_entry[58] = 0x00;
    raw_entry[59] = 0x00;

    let parsed = PartitionEntry::parse(0, &raw_entry);
    // Unpaired surrogate must be decoded to replacement character \u{FFFD} without panicking
    assert_eq!(parsed.name, "\u{FFFD}");

    // Also short slice (< 128 bytes) must parse safely without panicking
    let short_raw = [0u8; 64];
    let short_entry = PartitionEntry::parse(0, &short_raw);
    assert_eq!(short_entry.name, "");
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

    // 3. Volume too small for exFAT
    assert!(format_exfat(&mut tiny_dev, ExFatFormatOptions::default()).is_err());

    // 4. Volume too small for SexFS
    assert!(format_sexfs(&mut tiny_dev).is_err());
}

#[test]
fn test_adversarial_smart_parsing() {
    // Truncated buffers
    assert!(NvmeSmartLog::parse(&[0u8; 100]).is_err());
    assert!(AtaSmartData::parse(&[0u8; 300]).is_err());

    // All-zero buffers: valid parses without panic
    let zero_nvme = [0u8; 512];
    let nvme = NvmeSmartLog::parse(&zero_nvme).unwrap();
    assert_eq!(nvme.composite_temperature_kelvin, 0);
    assert_eq!(nvme.temperature_celsius(), -273); // Signed conversion without underflow!
    let nvme_report = nvme.analyze();
    assert_eq!(nvme_report.overall, corset_core::smart::OverallHealth::Critical); // Available spare 0 <= threshold 0
    assert_eq!(nvme_report.temperature_celsius, None); // 0 Kelvin means unrecorded temperature

    let zero_ata = [0u8; 512];
    let ata = AtaSmartData::parse(&zero_ata).unwrap();
    let ata_report = ata.analyze(None);
    assert_eq!(ata_report.overall, corset_core::smart::OverallHealth::Healthy);
}

#[test]
fn test_adversarial_crypt_workflow() {
    let mut dev = MemBlockDevice::new_512(2048).unwrap();
    let salt = [0x5Au8; 32];

    // 1. Empty passphrase rejected cleanly
    assert!(format_encrypted_volume(&mut dev, b"", &salt, None, Some(Cost::SMALL)).is_err());

    // 2. Valid volume formatting
    let master = format_encrypted_volume(&mut dev, b"SuperSecret123!", &salt, None, Some(Cost::SMALL)).unwrap();
    assert_ne!(master, [0u8; 64]);

    // 3. Out of bounds translation in EncryptedBlockIo
    {
        let mut enc_io = unlock_encrypted_volume(&mut dev, b"SuperSecret123!").unwrap();
        let usable = enc_io.sector_count();
        let mut buf = [0u8; 512];

        // Access beyond usable sector count is refused
        assert!(enc_io.read_sectors(usable, &mut buf).is_err());
        assert!(enc_io.write_sectors(usable, &buf).is_err());
        assert!(enc_io.read_sectors(u64::MAX, &mut buf).is_err());
    }

    // 4. Header erase wipes cryptographic header
    erase_encrypted_header(&mut dev).unwrap();
    assert!(unlock_encrypted_volume(&mut dev, b"SuperSecret123!").is_err());
}
