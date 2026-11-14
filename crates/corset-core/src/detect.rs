// SPDX-License-Identifier: GPL-3.0-only
//! A snapshot of a disk: its partition table, its partitions, and what each one holds,
//! told by signature only (nothing is mounted).
use crate::{types, AsSexDisk, BlockDevice, Error, Guid, SECTOR};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_disk::gpt::Gpt;
use sex_disk::probe::{self, Layout};

/// What a partition (or a whole disk without a table) holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    /// Its first 8 KiB are zero: nothing anyone would miss.
    Empty,
    /// Something Corset does not recognise.
    Unknown,
    Fat12,
    Fat16,
    Fat32,
    ExFat,
    /// ext2, ext3 or ext4.
    Ext,
    Ntfs,
    Sexfs,
    /// A sex-crypt volume; what is inside is known only with the password.
    Encrypted,
    /// A LUKS volume (Linux's encryption).
    Luks,
    LinuxSwap,
}
impl Content {
    pub fn name(self) -> &'static str {
        match self {
            Content::Empty => "empty", Content::Unknown => "unknown data", Content::Fat12 => "FAT12", Content::Fat16 => "FAT16",
            Content::Fat32 => "FAT32", Content::ExFat => "exFAT", Content::Ext => "ext4", Content::Ntfs => "NTFS",
            Content::Sexfs => "sexfs", Content::Encrypted => "encrypted (sex-crypt)", Content::Luks => "encrypted (LUKS)",
            Content::LinuxSwap => "Linux swap",
        }
    }
    /// Holds something a person may want: a change to it destroys data.
    pub fn holds_data(self) -> bool { self != Content::Empty }
}

/// What `dev` (a partition, or a whole disk) starts with.
pub fn content(dev: &mut dyn BlockDevice) -> Result<Content, Error> {
    let n = dev.sectors().min(16) as usize;
    if n == 0 { return Ok(Content::Empty); }
    let mut b = vec![0u8; n * SECTOR];
    dev.read(0, &mut b)?;
    let s0 = &b[..SECTOR];
    let le64 = |at: usize| b.get(at..at + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()));
    if &s0[..8] == sex_crypt::MAGIC { return Ok(Content::Encrypted); }
    if &s0[..6] == b"LUKS\xba\xbe" { return Ok(Content::Luks); }
    // sexfs keeps two superblock slots, blocks 0 and 1; the first commit lands in 1.
    const SEXFS_MAGIC: u64 = 0x3142_5346_5845_53;
    if le64(0) == Some(SEXFS_MAGIC) || le64(sexfs::BLOCK) == Some(SEXFS_MAGIC) { return Ok(Content::Sexfs); }
    if &s0[3..11] == b"EXFAT   " { return Ok(Content::ExFat); }
    if &s0[3..11] == b"NTFS    " { return Ok(Content::Ntfs); }
    if b.len() >= 3 * SECTOR && b[1024 + 56] == 0x53 && b[1024 + 57] == 0xEF { return Ok(Content::Ext); }
    if b.len() >= 4096 && &b[4096 - 10..4096] == b"SWAPSPACE2" { return Ok(Content::LinuxSwap); }
    if s0[510] == 0x55 && s0[511] == 0xAA {
        if &s0[82..90] == b"FAT32   " { return Ok(Content::Fat32); }
        if &s0[54..62] == b"FAT16   " { return Ok(Content::Fat16); }
        if &s0[54..62] == b"FAT12   " { return Ok(Content::Fat12); }
    }
    Ok(if b.iter().all(|x| *x == 0) { Content::Empty } else { Content::Unknown })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Partition {
    /// Its entry in the GPT's array: how ops name it.
    pub slot: usize,
    pub first: u64,
    pub sectors: u64,
    pub type_guid: Guid,
    pub part_guid: Guid,
    pub name: String,
    pub attrs: u64,
    pub content: Content,
}
impl Partition {
    pub fn end(&self) -> u64 { self.first + self.sectors }
    pub fn bytes(&self) -> u64 { self.sectors * SECTOR as u64 }
    pub fn type_name(&self) -> &'static str { types::name(self.type_guid) }
}

/// A disk as it was when read.
#[derive(Clone, Debug)]
pub struct Disk {
    /// What the person calls it (model and size, say): the word they type to confirm.
    pub name: String,
    pub sectors: u64,
    pub layout: Layout,
    /// Its GPT, when it has a readable one.
    pub gpt: Option<Gpt>,
    pub partitions: Vec<Partition>,
    /// What the whole disk holds when it has no partition table.
    pub whole: Content,
    pub(crate) fingerprint: u32,
}

/// CRC-32 of the regions a partition table lives in: the first 34 and last 33 sectors.
pub(crate) fn fingerprint(dev: &mut dyn BlockDevice) -> Result<u32, Error> {
    let total = dev.sectors();
    let mut c = sex_disk::crc32::Crc32::new();
    let head = total.min(34);
    let mut b = vec![0u8; head as usize * SECTOR];
    dev.read(0, &mut b)?;
    c.update(&b);
    let tail = total.saturating_sub(head).min(33);
    if tail > 0 {
        let mut b = vec![0u8; tail as usize * SECTOR];
        dev.read(total - tail, &mut b)?;
        c.update(&b);
    }
    c.update(&total.to_le_bytes());
    Ok(c.finish())
}

fn utf16_name(raw: &[u8]) -> String {
    let units: Vec<u16> = raw[56..128.min(raw.len())].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|u| *u != 0).collect();
    String::from_utf16_lossy(&units)
}

impl Disk {
    pub fn read(dev: &mut dyn BlockDevice, name: &str) -> Result<Disk, Error> {
        let sectors = dev.sectors();
        let layout = probe::classify(&mut AsSexDisk(&mut *dev)).map_err(|e| if e == sex_disk::Error::Io { Error::Io } else { Error::Corrupt })?;
        let gpt = match Gpt::read(&mut AsSexDisk(&mut *dev)) {
            Ok(g) if g.check().is_ok() => Some(g),
            Ok(_) => None,
            Err(sex_disk::Error::Io) => return Err(Error::Io),
            Err(_) => None,
        };
        let mut partitions = Vec::new();
        if let Some(g) = &gpt {
            for (slot, e) in g.used() {
                let mut w = crate::Window::new(dev, e.first(), e.sectors())?;
                let content = content(&mut w)?;
                partitions.push(Partition { slot, first: e.first(), sectors: e.sectors(), type_guid: e.type_guid(), part_guid: e.part_guid(),
                    name: utf16_name(&e.raw), attrs: u64::from_le_bytes(e.raw[48..56].try_into().unwrap()), content });
            }
            partitions.sort_by_key(|p| p.first);
        }
        let whole = if gpt.is_none() && layout == Layout::Blank { Content::Empty } else if gpt.is_none() { content(dev)? } else { Content::Unknown };
        let whole = if whole == Content::Empty && matches!(layout, Layout::Foreign(_) | Layout::LegacySexos) { Content::Unknown } else { whole };
        Ok(Disk { name: name.into(), sectors, layout, gpt, partitions, whole, fingerprint: fingerprint(dev)? })
    }
    pub fn bytes(&self) -> u64 { self.sectors * SECTOR as u64 }
    /// The disk holds a SexOS system (its slots or store): Corset leaves it alone.
    pub fn is_system(&self) -> bool {
        matches!(self.layout, Layout::Sexos { .. } | Layout::LegacySexos) || self.partitions.iter().any(|p| types::is_system(p.type_guid))
    }
}
