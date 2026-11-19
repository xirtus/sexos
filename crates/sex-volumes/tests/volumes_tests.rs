// SPDX-License-Identifier: GPL-3.0-only
use sex_volumes::partition::{self, Partition};
use sex_volumes::{days_from_civil, parts, probe, split_last, Disk, Error, Result, SECTOR};

struct MemDisk {
    sectors: u64,
    data: Vec<u8>,
}

impl MemDisk {
    fn new(sectors: u64) -> Self {
        Self {
            sectors,
            data: vec![0u8; (sectors as usize) * SECTOR],
        }
    }
}

impl Disk for MemDisk {
    fn sectors(&self) -> u64 {
        self.sectors
    }

    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<()> {
        let start = (lba as usize) * SECTOR;
        let end = start + buf.len();
        if end > self.data.len() {
            return Err(Error::Io);
        }
        buf.copy_from_slice(&self.data[start..end]);
        Ok(())
    }

    fn write(&mut self, lba: u64, buf: &[u8]) -> Result<()> {
        let start = (lba as usize) * SECTOR;
        let end = start + buf.len();
        if end > self.data.len() {
            return Err(Error::Io);
        }
        self.data[start..end].copy_from_slice(buf);
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

#[test]
fn test_path_parts_and_split_last() {
    assert_eq!(
        parts(b"/foo/bar/baz.txt").unwrap(),
        vec![b"foo".as_slice(), b"bar".as_slice(), b"baz.txt".as_slice()]
    );
    assert_eq!(
        parts(b"///a//b///").unwrap(),
        vec![b"a".as_slice(), b"b".as_slice()]
    );
    assert_eq!(parts(b"/").unwrap(), Vec::<&[u8]>::new());

    // Forbidden names: '.', '..', and NUL
    assert_eq!(parts(b"/foo/./bar"), Err(Error::Name));
    assert_eq!(parts(b"/foo/../bar"), Err(Error::Name));
    assert_eq!(parts(b"/foo/\0/bar"), Err(Error::Name));

    // split_last
    let (parent, last) = split_last(b"/docs/todo.txt").unwrap();
    assert_eq!(parent, vec![b"docs".as_slice()]);
    assert_eq!(last, b"todo.txt");

    // split_last on empty path returns Err(Error::Name)
    assert_eq!(split_last(b"/"), Err(Error::Name));
}

#[test]
fn test_days_from_civil_calendar_math() {
    // 1970-01-01 is Day 0 in Unix epoch
    assert_eq!(days_from_civil(1970, 1, 1), 0);

    // 1970-01-02 is Day 1
    assert_eq!(days_from_civil(1970, 1, 2), 1);

    // 2000-01-01
    assert_eq!(days_from_civil(2000, 1, 1), 10957);

    // Leap year day 2024-02-29 to 2024-03-01 is exactly 1 day
    let feb29 = days_from_civil(2024, 2, 29);
    let mar01 = days_from_civil(2024, 3, 1);
    assert_eq!(mar01 - feb29, 1);
}

#[test]
fn test_mbr_partition_parsing() {
    let mut disk = MemDisk::new(1000);

    // No boot signature 0x55 0xAA -> empty partition list
    assert_eq!(partition::table(&mut disk).unwrap(), Vec::new());

    // Write MBR boot signature at 510..512
    disk.data[510] = 0x55;
    disk.data[511] = 0xAA;

    // Empty partition entries (all 0) -> empty partition list
    assert_eq!(partition::table(&mut disk).unwrap(), Vec::new());

    // Create a valid MBR entry at entry 0 (offset 446)
    // kind = 0x83 (Linux native), start = 100, len = 200
    disk.data[446 + 4] = 0x83;
    disk.data[446 + 8..446 + 12].copy_from_slice(&100u32.to_le_bytes());
    disk.data[446 + 12..446 + 16].copy_from_slice(&200u32.to_le_bytes());

    // Create entry 1 at offset 462 with start + len exceeding disk sectors (should be skipped)
    disk.data[462 + 4] = 0x83;
    disk.data[462 + 8..462 + 12].copy_from_slice(&900u32.to_le_bytes());
    disk.data[462 + 12..462 + 16].copy_from_slice(&300u32.to_le_bytes()); // 900+300 > 1000

    let table = partition::table(&mut disk).unwrap();
    assert_eq!(table.len(), 1);
    assert_eq!(table[0], Partition { start: 100, len: 200 });
}

#[test]
fn test_gpt_partition_parsing() {
    let mut disk = MemDisk::new(2000);

    // Protective MBR: signature + kind 0xEE at entry 0
    disk.data[510] = 0x55;
    disk.data[511] = 0xAA;
    disk.data[446 + 4] = 0xEE;

    // LBA 1: GPT Header
    let lba1 = SECTOR;
    disk.data[lba1..lba1 + 8].copy_from_slice(b"EFI PART");
    // entries_lba at offset 72 = 2
    disk.data[lba1 + 72..lba1 + 80].copy_from_slice(&2u64.to_le_bytes());
    // entry count at offset 80 = 4
    disk.data[lba1 + 80..lba1 + 84].copy_from_slice(&4u32.to_le_bytes());
    // entry size at offset 84 = 128
    disk.data[lba1 + 84..lba1 + 88].copy_from_slice(&128u32.to_le_bytes());

    // LBA 2: Partition entries (entry size 128 bytes)
    let lba2 = 2 * SECTOR;
    // Entry 0: Type GUID non-zero
    disk.data[lba2..lba2 + 16].copy_from_slice(&[1u8; 16]);
    // first_lba at offset 32 = 200
    disk.data[lba2 + 32..lba2 + 40].copy_from_slice(&200u64.to_le_bytes());
    // last_lba at offset 40 = 499 (len = 300)
    disk.data[lba2 + 40..lba2 + 48].copy_from_slice(&499u64.to_le_bytes());

    let table = partition::table(&mut disk).unwrap();
    assert_eq!(table.len(), 1);
    assert_eq!(table[0], Partition { start: 200, len: 300 });

    // Corrupt GPT signature
    disk.data[lba1..lba1 + 8].copy_from_slice(b"BAD PART");
    assert_eq!(partition::table(&mut disk).unwrap(), Vec::new());
}

#[test]
fn test_filesystem_probing() {
    let mut disk = MemDisk::new(100);

    // Blank disk -> unsupported
    assert_eq!(probe(&mut disk).map(|_| ()), Err(Error::Unsupported));

    // exFAT signature at boot sector 3..11
    disk.data[3..11].copy_from_slice(b"EXFAT   ");
    // probe recognizes exFAT (mount may fail on zeroed parameters with Corrupt/Io)
    match probe(&mut disk) {
        Err(e) => assert_ne!(e, Error::Unsupported),
        Ok(_) => {}
    }

    // NTFS signature at boot sector 3..11
    let mut ntfs_disk = MemDisk::new(100);
    ntfs_disk.data[3..11].copy_from_slice(b"NTFS    ");
    match probe(&mut ntfs_disk) {
        Err(e) => assert_ne!(e, Error::Unsupported),
        Ok(_) => {}
    }

    // ext4 signature at LBA 2 offset 56: 0xEF53
    let mut ext4_disk = MemDisk::new(100);
    let lba2 = 2 * SECTOR;
    ext4_disk.data[lba2 + 56] = 0x53;
    ext4_disk.data[lba2 + 57] = 0xEF;
    match probe(&mut ext4_disk) {
        Err(e) => assert_ne!(e, Error::Unsupported),
        Ok(_) => {}
    }
}
