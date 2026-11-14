// SPDX-License-Identifier: GPL-3.0-only
//! Partition tables (MBR with logical partitions, GPT through `sex-disk`) and
//! filesystem signatures, so Loom can hand each partition to its own
//! filesystem code.

use alloc::string::String;
use alloc::vec::Vec;

use sex_disk::gpt::{Gpt, Guid};

use crate::util::{le16, le32, read_exact, read_padded, utf16be};
use crate::{ByteSource, Error, IoError, Result};

const SECTOR: u64 = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Mbr,
    Gpt,
}

/// The type code the table stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartType {
    Mbr(u8),
    /// GUID bytes as stored on disk (mixed-endian).
    Gpt([u8; 16]),
}

/// A filesystem recognised by its signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsKind {
    Fat12,
    Fat16,
    Fat32,
    ExFat,
    Ntfs,
    Ext2,
    Ext3,
    Ext4,
    Btrfs,
    Xfs,
    F2fs,
    Iso9660,
    Udf,
    Swap,
    Luks,
    Lvm,
    Apfs,
    HfsPlus,
    Unknown,
}

impl FsKind {
    pub fn label(self) -> &'static str {
        match self {
            FsKind::Fat12 => "FAT12",
            FsKind::Fat16 => "FAT16",
            FsKind::Fat32 => "FAT32",
            FsKind::ExFat => "exFAT",
            FsKind::Ntfs => "NTFS",
            FsKind::Ext2 => "ext2",
            FsKind::Ext3 => "ext3",
            FsKind::Ext4 => "ext4",
            FsKind::Btrfs => "Btrfs",
            FsKind::Xfs => "XFS",
            FsKind::F2fs => "F2FS",
            FsKind::Iso9660 => "ISO 9660",
            FsKind::Udf => "UDF",
            FsKind::Swap => "Linux swap",
            FsKind::Luks => "LUKS encrypted",
            FsKind::Lvm => "LVM physical volume",
            FsKind::Apfs => "APFS",
            FsKind::HfsPlus => "HFS+",
            FsKind::Unknown => "unknown",
        }
    }
}

/// One partition, in bytes from the start of the disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// 1-based; MBR logical partitions count from 5 as other systems number them.
    pub index: u32,
    pub scheme: Scheme,
    pub offset: u64,
    pub len: u64,
    pub part_type: PartType,
    /// What the type code means ("EFI System", "Linux filesystem", ...).
    pub type_label: &'static str,
    /// GPT partition name (empty for MBR).
    pub name: String,
    /// MBR active flag, or GPT legacy-BIOS-bootable attribute.
    pub bootable: bool,
    /// What the partition holds, by signature.
    pub fs: FsKind,
}

/// A window of another source: one partition.
pub struct Slice<'a, D: ?Sized> {
    dev: &'a D,
    offset: u64,
    len: u64,
}

impl<'a, D: ByteSource + ?Sized> Slice<'a, D> {
    pub fn new(dev: &'a D, offset: u64, len: u64) -> Self {
        let end = offset.saturating_add(len).min(dev.len());
        let offset = offset.min(end);
        Slice { dev, offset, len: end - offset }
    }
    pub fn offset(&self) -> u64 {
        self.offset
    }
}

impl<D: ByteSource + ?Sized> ByteSource for Slice<'_, D> {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> core::result::Result<usize, IoError> {
        if offset >= self.len {
            return Ok(0);
        }
        let n = ((self.len - offset) as usize).min(buf.len());
        self.dev.read_at(self.offset + offset, &mut buf[..n])
    }
}

/// `sex-disk`'s block device interface over a byte source.
struct Adapter<'a, D: ?Sized>(&'a D);

impl<D: ByteSource + ?Sized> sex_disk::BlockDev for Adapter<'_, D> {
    fn sectors(&self) -> u64 {
        self.0.len() / SECTOR
    }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> core::result::Result<(), sex_disk::Error> {
        let at = lba.checked_mul(SECTOR).ok_or(sex_disk::Error::Invalid)?;
        read_exact(self.0, at, buf).map_err(|_| sex_disk::Error::Io)
    }
    fn write(&mut self, _: u64, _: &[u8]) -> core::result::Result<(), sex_disk::Error> {
        Err(sex_disk::Error::Io)
    }
}

/// Partitions of a disk with 512-byte sectors: GPT when the MBR is protective
/// (or absent with a GPT header at LBA 1), else MBR primary and logical
/// partitions. A disk without a table gives an empty list.
pub fn read_partitions<D: ByteSource + ?Sized>(dev: &D) -> Result<Vec<Partition>> {
    let total = dev.len() / SECTOR;
    if total < 2 {
        return Ok(Vec::new());
    }
    let mbr = read_padded(dev, 0, 1024)?;
    let has_sig = mbr[510] == 0x55 && mbr[511] == 0xAA;
    let entries: Vec<&[u8]> = (0..4).map(|i| &mbr[446 + 16 * i..462 + 16 * i]).collect();
    let valid_mbr = has_sig && entries.iter().all(|e| e[0] == 0 || e[0] == 0x80);
    let protective = valid_mbr && entries.iter().any(|e| e[4] == 0xEE);
    if protective || (!valid_mbr && &mbr[512..520] == b"EFI PART") {
        return gpt(dev);
    }
    if !valid_mbr {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut logical = 5;
    for (i, e) in entries.iter().enumerate() {
        let ty = e[4];
        let start = le32(e, 8) as u64;
        let count = le32(e, 12) as u64;
        if ty == 0 || count == 0 || start == 0 || start >= total {
            continue;
        }
        if matches!(ty, 0x05 | 0x0F | 0x85) {
            ebr_chain(dev, start, total, &mut logical, &mut out)?;
            continue;
        }
        out.push(mbr_part(dev, i as u32 + 1, ty, start, count, e[0] == 0x80, total));
    }
    Ok(out)
}

fn mbr_part<D: ByteSource + ?Sized>(dev: &D, index: u32, ty: u8, start: u64, count: u64, boot: bool, total: u64) -> Partition {
    let count = count.min(total - start);
    let offset = start * SECTOR;
    let len = count * SECTOR;
    Partition {
        index,
        scheme: Scheme::Mbr,
        offset,
        len,
        part_type: PartType::Mbr(ty),
        type_label: mbr_label(ty),
        name: String::new(),
        bootable: boot,
        fs: probe_filesystem(dev, offset, len),
    }
}

fn ebr_chain<D: ByteSource + ?Sized>(dev: &D, ext: u64, total: u64, logical: &mut u32, out: &mut Vec<Partition>) -> Result<()> {
    let mut ebr = ext;
    for _ in 0..128 {
        if ebr >= total {
            break;
        }
        let s = read_padded(dev, ebr * SECTOR, 512)?;
        if s[510] != 0x55 || s[511] != 0xAA {
            break;
        }
        let a = &s[446..462];
        let b = &s[462..478];
        let (ty, start, count) = (a[4], le32(a, 8) as u64, le32(a, 12) as u64);
        if ty != 0 && count > 0 && ebr + start < total {
            out.push(mbr_part(dev, *logical, ty, ebr + start, count, a[0] == 0x80, total));
            *logical += 1;
        }
        let next = le32(b, 8) as u64;
        if !matches!(b[4], 0x05 | 0x0F | 0x85) || next == 0 {
            break;
        }
        ebr = ext + next;
    }
    Ok(())
}

fn gpt<D: ByteSource + ?Sized>(dev: &D) -> Result<Vec<Partition>> {
    let mut a = Adapter(dev);
    let g = Gpt::read(&mut a).map_err(|e| match e {
        sex_disk::Error::Io => Error::Corrupt("unreadable GPT"),
        _ => Error::Corrupt("damaged GPT (both copies)"),
    })?;
    let total = dev.len() / SECTOR;
    let mut out = Vec::new();
    for (i, e) in g.used() {
        if e.first() > e.last() || e.first() >= total {
            continue;
        }
        let offset = e.first() * SECTOR;
        let len = (e.last().min(total - 1) - e.first() + 1) * SECTOR;
        let raw = &e.raw;
        let attrs = u64::from_le_bytes(raw[48..56].try_into().unwrap());
        let name = utf16le_z(&raw[56..raw.len().min(128)]);
        let t = e.type_guid();
        out.push(Partition {
            index: i as u32 + 1,
            scheme: Scheme::Gpt,
            offset,
            len,
            part_type: PartType::Gpt(t.0),
            type_label: gpt_label(t),
            name,
            bootable: attrs & 4 != 0,
            fs: probe_filesystem(dev, offset, len),
        });
    }
    Ok(out)
}

fn utf16le_z(b: &[u8]) -> String {
    let mut be: Vec<u8> = Vec::new();
    for c in b.chunks_exact(2) {
        if c == [0, 0] {
            break;
        }
        be.extend_from_slice(&[c[1], c[0]]);
    }
    utf16be(&be)
}

fn mbr_label(t: u8) -> &'static str {
    match t {
        0x01 => "FAT12",
        0x04 | 0x06 | 0x0E => "FAT16",
        0x0B | 0x0C => "FAT32",
        0x07 => "NTFS/exFAT",
        0x11 | 0x14 | 0x16 | 0x1B | 0x1C | 0x1E => "Hidden FAT",
        0x17 => "Hidden NTFS",
        0x27 => "Windows recovery",
        0x82 => "Linux swap",
        0x83 => "Linux",
        0x8E => "Linux LVM",
        0xFD => "Linux RAID",
        0xA5 => "FreeBSD",
        0xA6 => "OpenBSD",
        0xA9 => "NetBSD",
        0xAF => "Apple HFS+",
        0xEF => "EFI System",
        0xEE => "GPT protective",
        0xDA => "Non-filesystem data",
        _ => "Unknown",
    }
}

const fn g(a: u32, b: u16, c: u16, d: [u8; 8]) -> Guid {
    Guid::new(a, b, c, d)
}

const GPT_TYPES: &[(Guid, &str)] = &[
    (sex_disk::gpt::TYPE_ESP, "EFI System"),
    (g(0x21686148, 0x6449, 0x6E6F, [0x74, 0x4E, 0x65, 0x65, 0x64, 0x45, 0x46, 0x49]), "BIOS boot"),
    (g(0xE3C9E316, 0x0B5C, 0x4DB8, [0x81, 0x7D, 0xF9, 0x2D, 0xF0, 0x02, 0x15, 0xAE]), "Microsoft reserved"),
    (g(0xEBD0A0A2, 0xB9E5, 0x4433, [0x87, 0xC0, 0x68, 0xB6, 0xB7, 0x26, 0x99, 0xC7]), "Microsoft basic data"),
    (g(0xDE94BBA4, 0x06D1, 0x4D40, [0xA1, 0x6A, 0xBF, 0xD5, 0x01, 0x79, 0xD6, 0xAC]), "Windows recovery"),
    (g(0x0FC63DAF, 0x8483, 0x4772, [0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D, 0xE4]), "Linux filesystem"),
    (g(0x4F68BCE3, 0xE8CD, 0x4DB1, [0x96, 0xE7, 0xFB, 0xCA, 0xF9, 0x84, 0xB7, 0x09]), "Linux root (x86-64)"),
    (g(0xB921B045, 0x1DF0, 0x41C3, [0xAF, 0x44, 0x4C, 0x6F, 0x28, 0x0D, 0x3F, 0xAE]), "Linux root (ARM64)"),
    (g(0x933AC7E1, 0x2EB4, 0x4F13, [0xB8, 0x44, 0x0E, 0x14, 0xE2, 0xAE, 0xF9, 0x15]), "Linux home"),
    (g(0xBC13C2FF, 0x59E6, 0x4262, [0xA3, 0x52, 0xB2, 0x75, 0xFD, 0x6F, 0x71, 0x72]), "Linux extended boot"),
    (g(0x0657FD6D, 0xA4AB, 0x43C4, [0x84, 0xE5, 0x09, 0x33, 0xC8, 0x4B, 0x4F, 0x4F]), "Linux swap"),
    (g(0xE6D6D379, 0xF507, 0x44C2, [0xA2, 0x3C, 0x23, 0x8F, 0x2A, 0x3D, 0xF9, 0x28]), "Linux LVM"),
    (g(0xA19D880F, 0x05FC, 0x4D3B, [0xA0, 0x06, 0x74, 0x3F, 0x0F, 0x84, 0x91, 0x1E]), "Linux RAID"),
    (g(0xCA7D7CCB, 0x63ED, 0x4C53, [0x86, 0x1C, 0x17, 0x42, 0x53, 0x60, 0x59, 0xCC]), "Linux LUKS"),
    (g(0x48465300, 0x0000, 0x11AA, [0xAA, 0x11, 0x00, 0x30, 0x65, 0x43, 0xEC, 0xAC]), "Apple HFS+"),
    (g(0x7C3457EF, 0x0000, 0x11AA, [0xAA, 0x11, 0x00, 0x30, 0x65, 0x43, 0xEC, 0xAC]), "Apple APFS"),
    (g(0x516E7CB4, 0x6ECF, 0x11D6, [0x8F, 0xF8, 0x00, 0x02, 0x2D, 0x09, 0x71, 0x2B]), "FreeBSD"),
    (g(0x516E7CB6, 0x6ECF, 0x11D6, [0x8F, 0xF8, 0x00, 0x02, 0x2D, 0x09, 0x71, 0x2B]), "FreeBSD UFS"),
    (g(0x516E7CBA, 0x6ECF, 0x11D6, [0x8F, 0xF8, 0x00, 0x02, 0x2D, 0x09, 0x71, 0x2B]), "FreeBSD ZFS"),
    (sex_disk::gpt::TYPE_SEXOS_BOOT, "SexOS system"),
    (sex_disk::gpt::TYPE_SEXOS_DATA, "SexOS store"),
];

fn gpt_label(t: Guid) -> &'static str {
    GPT_TYPES.iter().find(|(id, _)| *id == t).map(|(_, l)| *l).unwrap_or("Unknown")
}

/// The filesystem in `len` bytes at `offset` of `src`, by signature.
pub fn probe_filesystem<D: ByteSource + ?Sized>(src: &D, offset: u64, len: u64) -> FsKind {
    let s = Slice::new(src, offset, len);
    probe(&s).unwrap_or(FsKind::Unknown)
}

fn probe<D: ByteSource + ?Sized>(s: &D) -> Result<FsKind> {
    let b = read_padded(s, 0, 4096)?;
    if b.starts_with(b"LUKS\xba\xbe") {
        return Ok(FsKind::Luks);
    }
    if b.starts_with(b"XFSB") {
        return Ok(FsKind::Xfs);
    }
    if &b[3..11] == b"NTFS    " {
        return Ok(FsKind::Ntfs);
    }
    if &b[3..11] == b"EXFAT   " {
        return Ok(FsKind::ExFat);
    }
    if let Some(f) = fat_kind(&b) {
        return Ok(f);
    }
    if le16(&b, 1024 + 56) == 0xEF53 {
        let compat = le32(&b, 1024 + 92);
        let incompat = le32(&b, 1024 + 96);
        return Ok(if incompat & (0x40 | 0x80 | 0x200) != 0 {
            FsKind::Ext4
        } else if compat & 4 != 0 {
            FsKind::Ext3
        } else {
            FsKind::Ext2
        });
    }
    if le32(&b, 1024) == 0xF2F5_2010 {
        return Ok(FsKind::F2fs);
    }
    if &b[1024..1026] == b"H+" || &b[1024..1026] == b"HX" {
        return Ok(FsKind::HfsPlus);
    }
    if &b[32..36] == b"NXSB" {
        return Ok(FsKind::Apfs);
    }
    if &b[512..520] == b"LABELONE" {
        return Ok(FsKind::Lvm);
    }
    if &b[4086..4096] == b"SWAPSPACE2" || &b[4086..4096] == b"SWAP-SPACE" {
        return Ok(FsKind::Swap);
    }
    if &read_padded(s, 65536 + 64, 8)?[..] == b"_BHRfS_M" {
        return Ok(FsKind::Btrfs);
    }
    if crate::udf::detect(s)? {
        return Ok(FsKind::Udf);
    }
    if crate::iso::detect(s)? {
        return Ok(FsKind::Iso9660);
    }
    Ok(FsKind::Unknown)
}

fn fat_kind(b: &[u8]) -> Option<FsKind> {
    if !matches!(b[0], 0xEB | 0xE9) {
        return None;
    }
    let bps = le16(b, 11) as u64;
    let spc = b[13] as u64;
    let rsvd = le16(b, 14) as u64;
    let nfats = b[16] as u64;
    if !matches!(bps, 512 | 1024 | 2048 | 4096) || spc == 0 || !spc.is_power_of_two() || rsvd == 0 || !matches!(nfats, 1 | 2) {
        return None;
    }
    let root_ents = le16(b, 17) as u64;
    let total = match le16(b, 19) {
        0 => le32(b, 32) as u64,
        n => n as u64,
    };
    let fatsz = match le16(b, 22) {
        0 => le32(b, 36) as u64,
        n => n as u64,
    };
    let root_secs = (root_ents * 32).div_ceil(bps);
    let data = total.checked_sub(rsvd + nfats * fatsz + root_secs)?;
    let clusters = data / spc;
    Some(if clusters < 4085 {
        FsKind::Fat12
    } else if clusters < 65525 {
        FsKind::Fat16
    } else {
        FsKind::Fat32
    })
}
