// SPDX-License-Identifier: GPL-3.0-only
//! Making filesystems: FAT32 (sex-disk's formatter), exFAT ([`crate::exfat`]) and sexfs.
use crate::{types, AsSexDisk, AsSexfs, BlockDevice, Content, Error, Guid, SexfsMemory, SECTOR};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsKind { Fat32, ExFat, Sexfs }

impl FsKind {
    pub const ALL: [FsKind; 3] = [FsKind::Fat32, FsKind::ExFat, FsKind::Sexfs];
    pub fn name(self) -> &'static str { match self { FsKind::Fat32 => "FAT32", FsKind::ExFat => "exFAT", FsKind::Sexfs => "sexfs" } }
    pub fn content(self) -> Content { match self { FsKind::Fat32 => Content::Fat32, FsKind::ExFat => Content::ExFat, FsKind::Sexfs => Content::Sexfs } }
    /// The partition type other systems expect for it.
    pub fn default_type(self) -> Guid { match self { FsKind::Fat32 | FsKind::ExFat => types::BASIC_DATA, FsKind::Sexfs => types::SEXFS } }
    /// Smallest volume it can be made on, in sectors.
    pub fn min_sectors(self) -> u64 {
        match self {
            // 65525 one-sector clusters, two FATs of 512 sectors and 32 reserved sectors:
            // below this a volume is FAT16 by definition.
            FsKind::Fat32 => 32 + 2 * 512 + 65525,
            FsKind::ExFat => crate::exfat::MIN_SECTORS,
            // Two superblocks and the metadata reserve, with room to spare.
            FsKind::Sexfs => 4 << 11,
        }
    }
    /// Largest volume it can be made on, in sectors.
    pub fn max_sectors(self) -> u64 {
        match self {
            FsKind::Fat32 => u32::MAX as u64,
            FsKind::ExFat => crate::exfat::MAX_SECTORS,
            FsKind::Sexfs => u64::MAX / sexfs::BLOCK as u64,
        }
    }
    /// The longest label it keeps, in characters.
    pub fn label_max(self) -> usize { match self { FsKind::Fat32 => 11, FsKind::ExFat => 11, FsKind::Sexfs => 0 } }
    /// Whether a volume of this kind follows its partition when the partition grows.
    /// sexfs sizes itself from its device at every mount (and records the new size at its
    /// next commit); FAT and exFAT keep their size in the boot sector and have no grower here.
    pub fn grows(self) -> bool { self == FsKind::Sexfs }
}

/// A label as FAT keeps it: 11 bytes, uppercase ASCII, space padded; "NO NAME" if empty.
pub fn fat_label(label: &str) -> Result<[u8; 11], ()> {
    let mut out = *b"NO NAME    ";
    if label.is_empty() { return Ok(out); }
    if label.len() > 11 { return Err(()); }
    out = [b' '; 11];
    for (o, c) in out.iter_mut().zip(label.bytes()) {
        if !c.is_ascii_graphic() && c != b' ' || b"\"*+,./:;<=>?[\\]|".contains(&c) { return Err(()); }
        *o = c.to_ascii_uppercase();
    }
    Ok(out)
}

/// Makes `kind` over all of `dev`. `serial` names the volume (FAT/exFAT serial).
pub fn format(dev: &mut dyn BlockDevice, kind: FsKind, label: &str, serial: u32) -> Result<(), Error> {
    let sectors = dev.sectors();
    if sectors < kind.min_sectors() || sectors > kind.max_sectors() { return Err(Error::Format); }
    match kind {
        FsKind::Fat32 => {
            let l = fat_label(label).map_err(|_| Error::Format)?;
            sex_disk::fat::format_fat32(&mut AsSexDisk(dev), &l).map_err(|e| if e == sex_disk::Error::Io { Error::Io } else { Error::Format })?;
            // sex-disk derives the serial from the label; give each volume its own.
            let mut bs = [0u8; SECTOR];
            for at in [0u64, 6] {
                dev.read(at, &mut bs)?;
                bs[67..71].copy_from_slice(&serial.to_le_bytes());
                dev.write(at, &bs)?;
            }
        }
        FsKind::ExFat => crate::exfat::format(dev, label, serial)?,
        FsKind::Sexfs => {
            if !label.is_empty() { return Err(Error::Format); }
            let blocks = sectors / (sexfs::BLOCK / SECTOR) as u64;
            let mut mem = SexfsMemory::new(blocks, 16);
            let fs = sexfs::Fs::format(AsSexfs(dev), mem.lend()).map_err(|e| if e == sexfs::Error::Io { Error::Io } else { Error::Format })?;
            drop(fs);
        }
    }
    dev.flush()
}
