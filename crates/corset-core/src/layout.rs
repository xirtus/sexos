// SPDX-License-Identifier: GPL-3.0-only
//! GPT disk layout scanner, IEEE CRC32 verification, and visual region map.

use crate::device::{BlockIo, SECTOR_512};
use crate::error::CorsetError;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

// -----------------------------------------------------------------------------
// IEEE 802.3 CRC-32 (Reflected polynomial 0xEDB88320)
// -----------------------------------------------------------------------------

const fn crc32_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static CRC32_TABLE: [u32; 256] = crc32_table();

/// Incremental IEEE CRC-32 calculation.
#[derive(Clone, Copy, Debug)]
pub struct Crc32(u32);

impl Crc32 {
    pub const fn new() -> Self {
        Self(0xFFFF_FFFF)
    }

    pub fn update(&mut self, data: &[u8]) {
        for &b in data {
            self.0 = CRC32_TABLE[((self.0 ^ (b as u32)) & 0xFF) as usize] ^ (self.0 >> 8);
        }
    }

    pub fn finish(self) -> u32 {
        !self.0
    }
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

/// Computes the IEEE 802.3 CRC-32 checksum of `data`.
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = Crc32::new();
    c.update(data);
    c.finish()
}

// -----------------------------------------------------------------------------
// GUID Definition & Constants
// -----------------------------------------------------------------------------

/// 128-bit GUID stored in mixed-endian GPT format (first 3 components little-endian).
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Guid(pub [u8; 16]);

impl Guid {
    pub const ZERO: Guid = Guid([0; 16]);

    /// Construct from standard big-endian / human components `a-b-c-d-e`.
    pub const fn new(a: u32, b: u16, c: u16, d: [u8; 8]) -> Self {
        let a_le = a.to_le_bytes();
        let b_le = b.to_le_bytes();
        let c_le = c.to_le_bytes();
        Guid([
            a_le[0], a_le[1], a_le[2], a_le[3],
            b_le[0], b_le[1],
            c_le[0], c_le[1],
            d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7],
        ])
    }

    /// Construct from raw 16 bytes directly as in on-disk GPT.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Guid(bytes)
    }

    /// Convert to on-disk 16-byte array.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }

    pub fn is_zero(&self) -> bool {
        self.0 == [0; 16]
    }

    /// Deterministic UUID generation from a seed / entropy.
    pub fn from_entropy(mut bytes: [u8; 16]) -> Self {
        // Set UUID v4 variant
        bytes[7] = (bytes[7] & 0x0F) | 0x40; // Version 4
        bytes[8] = (bytes[8] & 0x3F) | 0x80; // Variant RFC 4122
        Guid(bytes)
    }

    /// Formats as standard canonical string `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
    pub fn to_hyphenated(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let b = &self.0;
        let order = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
        let mut out = [b'-'; 36];
        let mut at = 0;
        for (i, &idx) in order.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                at += 1;
            }
            out[at] = HEX[(b[idx] >> 4) as usize];
            out[at + 1] = HEX[(b[idx] & 15) as usize];
            at += 2;
        }
        let s = core::str::from_utf8(&out).unwrap_or("00000000-0000-0000-0000-000000000000");
        s.to_string()
    }

    /// Parses from standard canonical string `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
    pub fn parse(s: &str) -> Result<Self, CorsetError> {
        let s = s.trim();
        if s.len() != 36 {
            return Err(CorsetError::InvalidParameter("invalid GUID string length"));
        }
        let b = s.as_bytes();
        if b[8] != b'-' || b[13] != b'-' || b[18] != b'-' || b[23] != b'-' {
            return Err(CorsetError::InvalidParameter("invalid GUID hyphen placement"));
        }
        let hex_val = |c: u8| -> Result<u8, CorsetError> {
            match c {
                b'0'..=b'9' => Ok(c - b'0'),
                b'a'..=b'f' => Ok(c - b'a' + 10),
                b'A'..=b'F' => Ok(c - b'A' + 10),
                _ => Err(CorsetError::InvalidParameter("non-hex character in GUID")),
            }
        };
        let parse_byte = |hi: u8, lo: u8| -> Result<u8, CorsetError> {
            Ok((hex_val(hi)? << 4) | hex_val(lo)?)
        };

        let mut out = [0u8; 16];
        // Part 1: 4 bytes (little-endian on disk)
        out[3] = parse_byte(b[0], b[1])?;
        out[2] = parse_byte(b[2], b[3])?;
        out[1] = parse_byte(b[4], b[5])?;
        out[0] = parse_byte(b[6], b[7])?;

        // Part 2: 2 bytes (little-endian on disk)
        out[5] = parse_byte(b[9], b[10])?;
        out[4] = parse_byte(b[11], b[12])?;

        // Part 3: 2 bytes (little-endian on disk)
        out[7] = parse_byte(b[14], b[15])?;
        out[6] = parse_byte(b[16], b[17])?;

        // Part 4: 2 bytes (big-endian)
        out[8] = parse_byte(b[19], b[20])?;
        out[9] = parse_byte(b[21], b[22])?;

        // Part 5: 6 bytes (big-endian)
        out[10] = parse_byte(b[24], b[25])?;
        out[11] = parse_byte(b[26], b[27])?;
        out[12] = parse_byte(b[28], b[29])?;
        out[13] = parse_byte(b[30], b[31])?;
        out[14] = parse_byte(b[32], b[33])?;
        out[15] = parse_byte(b[34], b[35])?;

        Ok(Guid(out))
    }

    /// Human-friendly description for well-known partition types.
    pub fn human_type_name(&self) -> &'static str {
        if *self == TYPE_EFI_SYSTEM {
            "EFI System Partition"
        } else if *self == TYPE_SEXOS_BOOT {
            "SexOS Boot Slot"
        } else if *self == TYPE_SEXOS_DATA {
            "SexOS Data (SexFS)"
        } else if *self == TYPE_BDSM_ENCRYPTED {
            "BDSM Encrypted Volume"
        } else if *self == TYPE_MICROSOFT_BASIC_DATA {
            "Microsoft Basic Data (FAT/exFAT/NTFS)"
        } else if *self == TYPE_LINUX_FILESYSTEM {
            "Linux Filesystem"
        } else if *self == TYPE_LINUX_SWAP {
            "Linux Swap"
        } else if *self == TYPE_LINUX_LVM {
            "Linux LVM"
        } else if *self == TYPE_LINUX_LUKS {
            "Linux LUKS Encrypted"
        } else if self.is_zero() {
            "Unused Slot"
        } else {
            "Unknown Partition Type"
        }
    }
}

impl fmt::Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Guid({})", self.to_hyphenated())
    }
}

impl fmt::Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hyphenated())
    }
}

// Well-known Partition Type GUIDs
pub const TYPE_EFI_SYSTEM: Guid = Guid::new(
    0xC12A7328, 0xF81F, 0x11D2,
    [0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B],
);
pub const TYPE_SEXOS_BOOT: Guid = Guid::new(
    0x9B91C936, 0xD5FC, 0x4508,
    [0xA5, 0x1F, 0x2A, 0x5F, 0x03, 0xD9, 0xDB, 0x88],
);
pub const TYPE_SEXOS_DATA: Guid = Guid::new(
    0xC4DE67A4, 0xED02, 0x466C,
    [0x9A, 0xD5, 0x3F, 0x38, 0xC3, 0xC4, 0x76, 0xD6],
);
pub const TYPE_BDSM_ENCRYPTED: Guid = Guid::new(
    0xBD530001, 0x5E70, 0x4C79,
    [0x97, 0x11, 0x63, 0x72, 0x79, 0x70, 0x74, 0x31],
);
pub const TYPE_MICROSOFT_BASIC_DATA: Guid = Guid::new(
    0xEBD0A0A2, 0xB9E5, 0x4433,
    [0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7],
);
pub const TYPE_LINUX_FILESYSTEM: Guid = Guid::new(
    0x0FC63DAF, 0x8483, 0x4772,
    [0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4],
);
pub const TYPE_LINUX_SWAP: Guid = Guid::new(
    0x0657FD6D, 0xA4AB, 0x43C4,
    [0x84, 0xE5, 0x09, 0x33, 0xC8, 0x4B, 0x4F, 0x4F],
);
pub const TYPE_LINUX_LVM: Guid = Guid::new(
    0xE6D6D379, 0xF507, 0x44C2,
    [0xA2, 0x3C, 0x23, 0x8F, 0x2A, 0x3D, 0xF9, 0x28],
);
pub const TYPE_LINUX_LUKS: Guid = Guid::new(
    0xCA7D7CCB, 0x63ED, 0x4C53,
    [0x86, 0x72, 0x74, 0xCA, 0xB9, 0x55, 0xC7, 0x01],
);

// -----------------------------------------------------------------------------
// GPT On-Disk Structures
// -----------------------------------------------------------------------------

pub const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
pub const GPT_HEADER_SIZE: u32 = 92;
pub const DEFAULT_ENTRY_COUNT: u32 = 128;
pub const DEFAULT_ENTRY_SIZE: u32 = 128;

/// Parsed GPT Header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GptHeader {
    pub my_lba: u64,
    pub alternate_lba: u64,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub disk_guid: Guid,
    pub partition_entry_lba: u64,
    pub number_of_partition_entries: u32,
    pub size_of_partition_entry: u32,
    pub partition_entry_array_crc32: u32,
}

impl GptHeader {
    /// Parse and verify a GPT header from a 512-byte sector.
    pub fn parse(sector: &[u8], expected_lba: u64) -> Result<Self, CorsetError> {
        if sector.len() < SECTOR_512 as usize {
            return Err(CorsetError::GptCorrupt("sector buffer too small"));
        }
        if &sector[..8] != GPT_SIGNATURE {
            return Err(CorsetError::GptCorrupt("missing EFI PART signature"));
        }
        let read_u32 = |off: usize| u32::from_le_bytes(sector[off..off + 4].try_into().unwrap());
        let read_u64 = |off: usize| u64::from_le_bytes(sector[off..off + 8].try_into().unwrap());

        let revision = read_u32(8);
        if revision != 0x0001_0000 {
            return Err(CorsetError::GptCorrupt("unsupported GPT revision"));
        }
        let header_size = read_u32(12);
        if !(GPT_HEADER_SIZE..=SECTOR_512).contains(&header_size) {
            return Err(CorsetError::GptCorrupt("invalid header size"));
        }

        // Verify Header CRC-32 (with CRC field at 16..20 zeroed out)
        let stored_crc = read_u32(16);
        let mut check_buf = [0u8; SECTOR_512 as usize];
        check_buf[..header_size as usize].copy_from_slice(&sector[..header_size as usize]);
        check_buf[16..20].fill(0);
        let computed_crc = crc32(&check_buf[..header_size as usize]);
        if stored_crc != computed_crc {
            return Err(CorsetError::GptCorrupt("header CRC32 mismatch"));
        }

        let my_lba = read_u64(24);
        if my_lba != expected_lba {
            return Err(CorsetError::GptCorrupt("header MyLBA does not match physical LBA"));
        }
        let alternate_lba = read_u64(32);
        let first_usable_lba = read_u64(40);
        let last_usable_lba = read_u64(48);
        if first_usable_lba > last_usable_lba {
            return Err(CorsetError::GptCorrupt("first usable LBA > last usable LBA"));
        }

        let mut disk_guid_bytes = [0u8; 16];
        disk_guid_bytes.copy_from_slice(&sector[56..72]);
        let disk_guid = Guid(disk_guid_bytes);

        let partition_entry_lba = read_u64(72);
        let number_of_partition_entries = read_u32(80);
        let size_of_partition_entry = read_u32(84);
        let partition_entry_array_crc32 = read_u32(88);

        if size_of_partition_entry < 128 || size_of_partition_entry % 8 != 0 || size_of_partition_entry > 1024 {
            return Err(CorsetError::GptCorrupt("invalid partition entry size"));
        }
        if number_of_partition_entries == 0 || number_of_partition_entries > 1024 {
            return Err(CorsetError::GptCorrupt("invalid partition entry count"));
        }

        Ok(Self {
            my_lba,
            alternate_lba,
            first_usable_lba,
            last_usable_lba,
            disk_guid,
            partition_entry_lba,
            number_of_partition_entries,
            size_of_partition_entry,
            partition_entry_array_crc32,
        })
    }

    /// Number of 512-byte sectors occupied by the partition entry array.
    pub fn entry_sectors(&self) -> u64 {
        let total_bytes = (self.number_of_partition_entries as u64)
            .saturating_mul(self.size_of_partition_entry as u64);
        total_bytes.div_ceil(SECTOR_512 as u64)
    }

    /// Encode into a 512-byte sector, recomputing the header CRC32.
    pub fn encode(&self) -> [u8; SECTOR_512 as usize] {
        let mut s = [0u8; SECTOR_512 as usize];
        s[..8].copy_from_slice(GPT_SIGNATURE);
        s[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        s[12..16].copy_from_slice(&(GPT_HEADER_SIZE).to_le_bytes());
        s[24..32].copy_from_slice(&self.my_lba.to_le_bytes());
        s[32..40].copy_from_slice(&self.alternate_lba.to_le_bytes());
        s[40..48].copy_from_slice(&self.first_usable_lba.to_le_bytes());
        s[48..56].copy_from_slice(&self.last_usable_lba.to_le_bytes());
        s[56..72].copy_from_slice(&self.disk_guid.0);
        s[72..80].copy_from_slice(&self.partition_entry_lba.to_le_bytes());
        s[80..84].copy_from_slice(&self.number_of_partition_entries.to_le_bytes());
        s[84..88].copy_from_slice(&self.size_of_partition_entry.to_le_bytes());
        s[88..92].copy_from_slice(&self.partition_entry_array_crc32.to_le_bytes());

        let c = crc32(&s[..GPT_HEADER_SIZE as usize]);
        s[16..20].copy_from_slice(&c.to_le_bytes());
        s
    }
}

/// A parsed GPT partition entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartitionEntry {
    pub index: usize,
    pub type_guid: Guid,
    pub partition_guid: Guid,
    pub first_lba: u64,
    pub last_lba: u64,
    pub attributes: u64,
    pub name: String,
    pub raw: Vec<u8>,
}

impl PartitionEntry {
    /// True if the entry slot is in use (type GUID is non-zero).
    pub fn is_used(&self) -> bool {
        !self.type_guid.is_zero()
    }

    /// Number of sectors spanned by this partition.
    pub fn sector_count(&self) -> u64 {
        if self.last_lba >= self.first_lba {
            self.last_lba - self.first_lba + 1
        } else {
            0
        }
    }

    /// Size in bytes given a sector size.
    pub fn byte_size(&self, sector_size: u32) -> u64 {
        self.sector_count().saturating_mul(sector_size as u64)
    }

    /// Parse from raw bytes of one entry.
    pub fn parse(index: usize, raw: &[u8]) -> Self {
        let mut type_guid_bytes = [0u8; 16];
        let mut part_guid_bytes = [0u8; 16];
        type_guid_bytes.copy_from_slice(&raw[0..16]);
        part_guid_bytes.copy_from_slice(&raw[16..32]);
        let first_lba = u64::from_le_bytes(raw[32..40].try_into().unwrap());
        let last_lba = u64::from_le_bytes(raw[40..48].try_into().unwrap());
        let attributes = u64::from_le_bytes(raw[48..56].try_into().unwrap());

        // Decode UTF-16LE partition name (up to 36 u16 characters at 56..128)
        let mut name_u16 = Vec::new();
        for chunk in raw[56..128].chunks_exact(2) {
            let u = u16::from_le_bytes([chunk[0], chunk[1]]);
            if u == 0 {
                break;
            }
            name_u16.push(u);
        }
        let name: String = char::decode_utf16(name_u16.into_iter())
            .map(|c| c.unwrap_or('\u{FFFD}'))
            .collect();

        Self {
            index,
            type_guid: Guid(type_guid_bytes),
            partition_guid: Guid(part_guid_bytes),
            first_lba,
            last_lba,
            attributes,
            name,
            raw: raw.to_vec(),
        }
    }

    /// Create a new partition entry.
    pub fn new(
        index: usize,
        entry_size: usize,
        type_guid: Guid,
        partition_guid: Guid,
        first_lba: u64,
        last_lba: u64,
        attributes: u64,
        name: &str,
    ) -> Self {
        let mut raw = vec![0u8; entry_size];
        raw[0..16].copy_from_slice(&type_guid.0);
        raw[16..32].copy_from_slice(&partition_guid.0);
        raw[32..40].copy_from_slice(&first_lba.to_le_bytes());
        raw[40..48].copy_from_slice(&last_lba.to_le_bytes());
        raw[48..56].copy_from_slice(&attributes.to_le_bytes());

        for (i, u) in name.encode_utf16().take(36).enumerate() {
            let offset = 56 + 2 * i;
            if offset + 2 <= entry_size {
                raw[offset..offset + 2].copy_from_slice(&u.to_le_bytes());
            }
        }

        Self {
            index,
            type_guid,
            partition_guid,
            first_lba,
            last_lba,
            attributes,
            name: name.to_string(),
            raw,
        }
    }

    /// Serialize into raw bytes.
    pub fn encode(&self, entry_size: usize) -> Vec<u8> {
        let mut raw = self.raw.clone();
        if raw.len() < entry_size {
            raw.resize(entry_size, 0);
        }
        raw[0..16].copy_from_slice(&self.type_guid.0);
        raw[16..32].copy_from_slice(&self.partition_guid.0);
        raw[32..40].copy_from_slice(&self.first_lba.to_le_bytes());
        raw[40..48].copy_from_slice(&self.last_lba.to_le_bytes());
        raw[48..56].copy_from_slice(&self.attributes.to_le_bytes());

        // Update name
        raw[56..128].fill(0);
        for (i, u) in self.name.encode_utf16().take(36).enumerate() {
            let offset = 56 + 2 * i;
            if offset + 2 <= 128 {
                raw[offset..offset + 2].copy_from_slice(&u.to_le_bytes());
            }
        }
        raw
    }
}

// -----------------------------------------------------------------------------
// Protective MBR
// -----------------------------------------------------------------------------

/// Create a protective MBR sector for a disk of `total_sectors`.
pub fn create_protective_mbr(total_sectors: u64) -> [u8; SECTOR_512 as usize] {
    let mut mbr = [0u8; SECTOR_512 as usize];
    // Partition entry 1 at 446 (0x1BE)
    let e = &mut mbr[446..462];
    e[0] = 0x00; // Bootable flag
    e[1..4].copy_from_slice(&[0x00, 0x02, 0x00]); // Starting CHS
    e[4] = 0xEE; // Partition type 0xEE (GPT Protective)
    e[5..8].copy_from_slice(&[0xFF, 0xFF, 0xFF]); // Ending CHS
    e[8..12].copy_from_slice(&1u32.to_le_bytes()); // Starting LBA 1
    let size_lba = (total_sectors.saturating_sub(1)).min(0xFFFF_FFFF) as u32;
    e[12..16].copy_from_slice(&size_lba.to_le_bytes());
    mbr[510] = 0x55;
    mbr[511] = 0xAA;
    mbr
}

/// Check if sector 0 contains a valid protective MBR.
pub fn is_protective_mbr(sector: &[u8]) -> bool {
    if sector.len() < 512 || sector[510] != 0x55 || sector[511] != 0xAA {
        return false;
    }
    // Check for partition with type 0xEE in any of the 4 MBR partition slots
    for i in 0..4 {
        let offset = 446 + i * 16;
        if sector[offset + 4] == 0xEE {
            return true;
        }
    }
    false
}

// -----------------------------------------------------------------------------
// Visual Disk Map & Regions
// -----------------------------------------------------------------------------

/// Kinds of regions present on a partitioned disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiskRegionKind {
    ProtectiveMbr,
    PrimaryGptHeader,
    PrimaryGptEntries,
    Partition(PartitionEntry),
    FreeSpace,
    BackupGptEntries,
    BackupGptHeader,
    UnusableMetadata,
}

/// A contiguous slice of the disk in the visual map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskRegion {
    pub start_lba: u64,
    pub end_lba: u64,
    pub sector_count: u64,
    pub byte_start: u64,
    pub byte_len: u64,
    pub is_aligned_1mib: bool,
    pub kind: DiskRegionKind,
}

/// Information about a contiguous free space extent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreeExtent {
    pub start_lba: u64,
    pub end_lba: u64,
    pub sector_count: u64,
    pub byte_start: u64,
    pub byte_len: u64,
    pub aligned_start_lba_1mib: u64,
    pub aligned_sectors_1mib: u64,
}

/// Complete structural and visual map of a GPT-partitioned disk.
#[derive(Clone, Debug)]
pub struct DiskMap {
    pub sector_size: u32,
    pub total_sectors: u64,
    pub disk_guid: Guid,
    pub regions: Vec<DiskRegion>,
    pub partitions: Vec<PartitionEntry>,
    pub free_extents: Vec<FreeExtent>,
    pub primary_valid: bool,
    pub backup_valid: bool,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
}

impl DiskMap {
    /// Find the first free extent that can fit `sectors_needed` with 1 MiB alignment.
    pub fn find_free_extent_1mib(&self, sectors_needed: u64) -> Option<FreeExtent> {
        self.free_extents
            .iter()
            .find(|ext| ext.aligned_sectors_1mib >= sectors_needed)
            .copied()
    }

    /// Find partition entry by index.
    pub fn partition_by_index(&self, index: usize) -> Option<&PartitionEntry> {
        self.partitions.iter().find(|p| p.index == index)
    }

    /// Find partition entry by unique partition GUID.
    pub fn partition_by_guid(&self, guid: &Guid) -> Option<&PartitionEntry> {
        self.partitions.iter().find(|p| &p.partition_guid == guid)
    }
}

/// Reads the partition entry array from `dev` given `header`.
pub fn read_entries<D: BlockIo + ?Sized>(
    dev: &mut D,
    header: &GptHeader,
) -> Result<Vec<PartitionEntry>, CorsetError> {
    let entry_sectors = header.entry_sectors();
    let total_bytes = (entry_sectors as usize) * (dev.sector_size() as usize);
    let mut raw_bytes = vec![0u8; total_bytes];

    dev.read_sectors(header.partition_entry_lba, &mut raw_bytes)?;

    let used_bytes = (header.number_of_partition_entries as usize) * (header.size_of_partition_entry as usize);
    if used_bytes > raw_bytes.len() {
        return Err(CorsetError::GptCorrupt("entry array exceeds allocated sectors"));
    }

    let actual_crc = crc32(&raw_bytes[..used_bytes]);
    if actual_crc != header.partition_entry_array_crc32 {
        return Err(CorsetError::GptCorrupt("partition entry array CRC32 mismatch"));
    }

    let entry_sz = header.size_of_partition_entry as usize;
    let mut entries = Vec::with_capacity(header.number_of_partition_entries as usize);
    for (i, chunk) in raw_bytes[..used_bytes].chunks_exact(entry_sz).enumerate() {
        entries.push(PartitionEntry::parse(i, chunk));
    }
    Ok(entries)
}

/// Scans the disk layout, verifies MBR and both GPT headers/entry arrays, and builds a `DiskMap`.
pub fn scan_disk_map<D: BlockIo + ?Sized>(dev: &mut D) -> Result<DiskMap, CorsetError> {
    let sector_size = dev.sector_size();
    let total_sectors = dev.sector_count();
    if total_sectors < 68 {
        return Err(CorsetError::InvalidParameter("device too small for GPT"));
    }

    // 1. Check Protective MBR at LBA 0
    let mut sector_buf = vec![0u8; sector_size as usize];
    dev.read_sectors(0, &mut sector_buf)?;
    let _has_protective_mbr = is_protective_mbr(&sector_buf);

    // 2. Read Primary GPT at LBA 1
    dev.read_sectors(1, &mut sector_buf)?;
    let primary_header_res = GptHeader::parse(&sector_buf, 1);
    let (primary_header, primary_entries, primary_valid) = match primary_header_res {
        Ok(h) => match read_entries(dev, &h) {
            Ok(entries) => (Some(h), Some(entries), true),
            Err(_) => (Some(h), None, false),
        },
        Err(_) => (None, None, false),
    };

    // 3. Read Backup GPT at LBA (total_sectors - 1)
    let backup_lba = total_sectors - 1;
    dev.read_sectors(backup_lba, &mut sector_buf)?;
    let backup_header_res = GptHeader::parse(&sector_buf, backup_lba);
    let (backup_header, backup_entries, backup_valid) = match backup_header_res {
        Ok(h) => match read_entries(dev, &h) {
            Ok(entries) => (Some(h), Some(entries), true),
            Err(_) => (Some(h), None, false),
        },
        Err(_) => (None, None, false),
    };

    if !primary_valid && !backup_valid {
        return Err(CorsetError::GptCorrupt("both primary and backup GPT are invalid"));
    }

    // Prefer primary if valid, otherwise fallback to backup
    let active_header = if primary_valid {
        primary_header.unwrap()
    } else {
        backup_header.unwrap()
    };

    let active_entries = if primary_valid {
        primary_entries.unwrap()
    } else {
        backup_entries.unwrap()
    };

    let first_usable = active_header.first_usable_lba;
    let last_usable = active_header.last_usable_lba;
    let disk_guid = active_header.disk_guid;

    // Collect all used partitions and sort by first_lba
    let mut used_partitions: Vec<PartitionEntry> = active_entries
        .into_iter()
        .filter(|e| e.is_used())
        .collect();
    used_partitions.sort_by_key(|p| p.first_lba);

    // Check overlaps
    for w in used_partitions.windows(2) {
        if w[0].last_lba >= w[1].first_lba {
            return Err(CorsetError::PartitionOverlap {
                first: w[1].first_lba,
                last: w[1].last_lba,
                existing_first: w[0].first_lba,
                existing_last: w[0].last_lba,
            });
        }
    }

    let sectors_per_1mib = (1024 * 1024) / (sector_size as u64);
    let is_aligned = |lba: u64| lba % sectors_per_1mib == 0;

    let mut regions = Vec::new();
    let mut free_extents = Vec::new();

    // LBA 0: Protective MBR
    regions.push(DiskRegion {
        start_lba: 0,
        end_lba: 0,
        sector_count: 1,
        byte_start: 0,
        byte_len: sector_size as u64,
        is_aligned_1mib: is_aligned(0),
        kind: DiskRegionKind::ProtectiveMbr,
    });

    // LBA 1: Primary GPT Header
    regions.push(DiskRegion {
        start_lba: 1,
        end_lba: 1,
        sector_count: 1,
        byte_start: sector_size as u64,
        byte_len: sector_size as u64,
        is_aligned_1mib: is_aligned(1),
        kind: DiskRegionKind::PrimaryGptHeader,
    });

    // LBA 2..first_usable-1: Primary GPT Entries
    let entry_sectors = active_header.entry_sectors();
    let entries_start = active_header.partition_entry_lba;
    let entries_end = entries_start + entry_sectors - 1;
    regions.push(DiskRegion {
        start_lba: entries_start,
        end_lba: entries_end,
        sector_count: entry_sectors,
        byte_start: entries_start * sector_size as u64,
        byte_len: entry_sectors * sector_size as u64,
        is_aligned_1mib: is_aligned(entries_start),
        kind: DiskRegionKind::PrimaryGptEntries,
    });

    // Usable Area Scan
    let mut cursor = first_usable;
    for part in &used_partitions {
        if part.first_lba > cursor {
            // Free space before this partition
            let free_start = cursor;
            let free_end = part.first_lba - 1;
            let free_sectors = free_end - free_start + 1;

            let aligned_start = free_start.next_multiple_of(sectors_per_1mib);
            let aligned_sectors = if free_end >= aligned_start {
                free_end - aligned_start + 1
            } else {
                0
            };

            let extent = FreeExtent {
                start_lba: free_start,
                end_lba: free_end,
                sector_count: free_sectors,
                byte_start: free_start * sector_size as u64,
                byte_len: free_sectors * sector_size as u64,
                aligned_start_lba_1mib: aligned_start,
                aligned_sectors_1mib: aligned_sectors,
            };
            free_extents.push(extent);

            regions.push(DiskRegion {
                start_lba: free_start,
                end_lba: free_end,
                sector_count: free_sectors,
                byte_start: free_start * sector_size as u64,
                byte_len: free_sectors * sector_size as u64,
                is_aligned_1mib: is_aligned(free_start),
                kind: DiskRegionKind::FreeSpace,
            });
        }

        // Partition Region
        let part_sectors = part.sector_count();
        regions.push(DiskRegion {
            start_lba: part.first_lba,
            end_lba: part.last_lba,
            sector_count: part_sectors,
            byte_start: part.first_lba * sector_size as u64,
            byte_len: part_sectors * sector_size as u64,
            is_aligned_1mib: is_aligned(part.first_lba),
            kind: DiskRegionKind::Partition(part.clone()),
        });

        cursor = part.last_lba + 1;
    }

    // Trailing free space up to last_usable
    if last_usable >= cursor {
        let free_start = cursor;
        let free_end = last_usable;
        let free_sectors = free_end - free_start + 1;

        let aligned_start = free_start.next_multiple_of(sectors_per_1mib);
        let aligned_sectors = if free_end >= aligned_start {
            free_end - aligned_start + 1
        } else {
            0
        };

        let extent = FreeExtent {
            start_lba: free_start,
            end_lba: free_end,
            sector_count: free_sectors,
            byte_start: free_start * sector_size as u64,
            byte_len: free_sectors * sector_size as u64,
            aligned_start_lba_1mib: aligned_start,
            aligned_sectors_1mib: aligned_sectors,
        };
        free_extents.push(extent);

        regions.push(DiskRegion {
            start_lba: free_start,
            end_lba: free_end,
            sector_count: free_sectors,
            byte_start: free_start * sector_size as u64,
            byte_len: free_sectors * sector_size as u64,
            is_aligned_1mib: is_aligned(free_start),
            kind: DiskRegionKind::FreeSpace,
        });
    }

    // Backup GPT Entries
    let backup_entries_start = backup_lba - entry_sectors;
    let backup_entries_end = backup_lba - 1;
    regions.push(DiskRegion {
        start_lba: backup_entries_start,
        end_lba: backup_entries_end,
        sector_count: entry_sectors,
        byte_start: backup_entries_start * sector_size as u64,
        byte_len: entry_sectors * sector_size as u64,
        is_aligned_1mib: is_aligned(backup_entries_start),
        kind: DiskRegionKind::BackupGptEntries,
    });

    // Backup GPT Header
    regions.push(DiskRegion {
        start_lba: backup_lba,
        end_lba: backup_lba,
        sector_count: 1,
        byte_start: backup_lba * sector_size as u64,
        byte_len: sector_size as u64,
        is_aligned_1mib: is_aligned(backup_lba),
        kind: DiskRegionKind::BackupGptHeader,
    });

    Ok(DiskMap {
        sector_size,
        total_sectors,
        disk_guid,
        regions,
        partitions: used_partitions,
        free_extents,
        primary_valid,
        backup_valid,
        first_usable_lba: first_usable,
        last_usable_lba: last_usable,
    })
}
