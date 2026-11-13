// SPDX-License-Identifier: GPL-3.0-only
//! Other systems' filesystems, as Files sees them: FAT12/16/32, exFAT and ext4 (with
//! extents) read and write; ext2/3 and NTFS read only. USB sticks, SD cards and disks from other machines mount
//! under `/media`; until USB mass storage lands, an image file on the SexOS filesystem
//! stands in for the stick.
//!
//! A volume holds no device. Each call is handed the disk, so Files can lend a volume a
//! file of its own filesystem (a loop mount) or a block device without the volume
//! outliving or owning either. Volumes keep only geometry and small caches they can
//! always rebuild by reading the disk again.
#![cfg_attr(not(test), no_std)]
extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

pub mod partition;
pub mod fat;
pub mod exfat;
pub mod ext4;
pub mod ntfs;

pub const SECTOR: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The disk refused a read or write.
    Io,
    /// The on-disk structures don't add up.
    Corrupt,
    NotFound,
    Exists,
    NotDir,
    IsDir,
    NotEmpty,
    /// A name this filesystem can't hold (too long, a forbidden character, empty).
    Name,
    NoSpace,
    /// This filesystem is mounted read only, or SexOS only reads this kind.
    ReadOnly,
    /// A feature of this volume SexOS doesn't read yet (compression, encryption...).
    Unsupported,
    TooBig,
}
pub type Result<T> = core::result::Result<T, Error>;

/// Whole 512-byte sectors. `buf.len()` is always a multiple of [`SECTOR`].
pub trait Disk {
    fn sectors(&self) -> u64;
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<()>;
    fn write(&mut self, lba: u64, buf: &[u8]) -> Result<()>;
    fn flush(&mut self) -> Result<()>;
}

/// A run of sectors of a disk: one partition.
pub struct Slice<'a> { pub disk: &'a mut dyn Disk, pub start: u64, pub len: u64 }
impl Disk for Slice<'_> {
    fn sectors(&self) -> u64 { self.len }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<()> {
        if lba + (buf.len() / SECTOR) as u64 > self.len { return Err(Error::Io); }
        self.disk.read(self.start + lba, buf)
    }
    fn write(&mut self, lba: u64, buf: &[u8]) -> Result<()> {
        if lba + (buf.len() / SECTOR) as u64 > self.len { return Err(Error::Io); }
        self.disk.write(self.start + lba, buf)
    }
    fn flush(&mut self) -> Result<()> { self.disk.flush() }
}

/// Reads `buf.len()` bytes at byte `offset`, whatever the alignment.
pub fn read_bytes(disk: &mut dyn Disk, offset: u64, buf: &mut [u8]) -> Result<()> {
    let mut sector = [0u8; SECTOR];
    let mut done = 0;
    while done < buf.len() {
        let at = offset + done as u64;
        let (lba, within) = (at / SECTOR as u64, (at % SECTOR as u64) as usize);
        let left = buf.len() - done;
        if within == 0 && left >= SECTOR {
            let whole = left / SECTOR * SECTOR;
            disk.read(lba, &mut buf[done..done + whole])?;
            done += whole;
        } else {
            disk.read(lba, &mut sector)?;
            let take = (SECTOR - within).min(left);
            buf[done..done + take].copy_from_slice(&sector[within..within + take]);
            done += take;
        }
    }
    Ok(())
}
/// Writes `data` at byte `offset`, reading first any sector it only partly covers.
pub fn write_bytes(disk: &mut dyn Disk, offset: u64, data: &[u8]) -> Result<()> {
    let mut sector = [0u8; SECTOR];
    let mut done = 0;
    while done < data.len() {
        let at = offset + done as u64;
        let (lba, within) = (at / SECTOR as u64, (at % SECTOR as u64) as usize);
        let left = data.len() - done;
        if within == 0 && left >= SECTOR {
            let whole = left / SECTOR * SECTOR;
            disk.write(lba, &data[done..done + whole])?;
            done += whole;
        } else {
            disk.read(lba, &mut sector)?;
            let take = (SECTOR - within).min(left);
            sector[within..within + take].copy_from_slice(&data[done..done + take]);
            disk.write(lba, &sector)?;
            done += take;
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { File, Dir }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    pub kind: Kind,
    pub size: u64,
    /// Seconds since 1970 UTC, 0 when the volume doesn't say.
    pub mtime: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry { pub name: String, pub kind: Kind, pub size: u64 }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format { Fat12, Fat16, Fat32, ExFat, Ext4, Ntfs }
impl Format {
    pub fn name(self) -> &'static str {
        match self { Format::Fat12 => "FAT12", Format::Fat16 => "FAT16", Format::Fat32 => "FAT32", Format::ExFat => "exFAT", Format::Ext4 => "ext4", Format::Ntfs => "NTFS" }
    }
}

/// One mounted filesystem. Paths are UTF-8 bytes relative to the volume's root, split
/// on `/`; a leading `/` and empty parts are ignored, `.` and `..` are refused.
/// Every write leaves the volume consistent for the other system once `sync` returns.
pub trait Volume {
    fn format(&self) -> Format;
    /// The volume's own name, if it has one.
    fn label(&self) -> String;
    fn read_only(&self) -> bool;
    /// The time, Unix seconds, to stamp on what changes from now on.
    fn set_now(&mut self, _unix: u64) {}
    /// (total bytes, free bytes).
    fn statfs(&mut self, disk: &mut dyn Disk) -> Result<(u64, u64)>;
    fn stat(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<Stat>;
    fn list(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<Vec<Entry>>;
    fn read(&mut self, disk: &mut dyn Disk, path: &[u8], offset: u64, buf: &mut [u8]) -> Result<usize>;
    fn write(&mut self, _disk: &mut dyn Disk, _path: &[u8], _offset: u64, _data: &[u8]) -> Result<usize> { Err(Error::ReadOnly) }
    fn create(&mut self, _disk: &mut dyn Disk, _path: &[u8]) -> Result<()> { Err(Error::ReadOnly) }
    fn mkdir(&mut self, _disk: &mut dyn Disk, _path: &[u8]) -> Result<()> { Err(Error::ReadOnly) }
    /// A file, or an empty directory.
    fn remove(&mut self, _disk: &mut dyn Disk, _path: &[u8]) -> Result<()> { Err(Error::ReadOnly) }
    /// Within this volume; `to` must not exist.
    fn rename(&mut self, _disk: &mut dyn Disk, _from: &[u8], _to: &[u8]) -> Result<()> { Err(Error::ReadOnly) }
    fn truncate(&mut self, _disk: &mut dyn Disk, _path: &[u8], _len: u64) -> Result<()> { Err(Error::ReadOnly) }
    fn sync(&mut self, disk: &mut dyn Disk) -> Result<()> { disk.flush() }
}

/// The parts of a path, refusing `.`, `..` and NUL.
pub fn parts(path: &[u8]) -> Result<Vec<&[u8]>> {
    let mut out = Vec::new();
    for p in path.split(|b| *b == b'/').filter(|p| !p.is_empty()) {
        if p == b"." || p == b".." || p.contains(&0) { return Err(Error::Name); }
        out.push(p);
    }
    Ok(out)
}
/// The parent's parts and the last name; the root has no last name.
pub fn split_last(path: &[u8]) -> Result<(Vec<&[u8]>, &[u8])> {
    let mut p = parts(path)?;
    let last = p.pop().ok_or(Error::Name)?;
    Ok((p, last))
}

/// Recognises the filesystem starting at sector 0 of `disk`.
pub fn probe(disk: &mut dyn Disk) -> Result<Box<dyn Volume>> {
    let mut boot = [0u8; SECTOR];
    disk.read(0, &mut boot)?;
    if &boot[3..11] == b"EXFAT   " { return Ok(Box::new(exfat::ExFat::mount(disk)?)); }
    if &boot[3..11] == b"NTFS    " { return Ok(Box::new(ntfs::Ntfs::mount(disk)?)); }
    if disk.sectors() > 3 {
        let mut sb = [0u8; SECTOR];
        disk.read(2, &mut sb)?;
        if u16::from_le_bytes([sb[56], sb[57]]) == 0xEF53 { return Ok(Box::new(ext4::Ext4::mount(disk)?)); }
    }
    if boot[510] == 0x55 && boot[511] == 0xAA && fat::looks_like_fat(&boot) { return Ok(Box::new(fat::Fat::mount(disk)?)); }
    Err(Error::Unsupported)
}

/// A volume found on a disk, and where.
pub struct Found { pub start: u64, pub len: u64, pub volume: Box<dyn Volume> }

/// Every volume SexOS can read on a disk: the partitions of an MBR or GPT disk, or the
/// whole disk when it has no partition table (a "superfloppy" stick).
pub fn scan(disk: &mut dyn Disk) -> Result<Vec<Found>> {
    let mut found = Vec::new();
    let sectors = disk.sectors();
    // A FAT boot sector also ends in 55 AA, so try the whole disk as a volume first.
    if let Ok(volume) = probe(disk) { found.push(Found { start: 0, len: sectors, volume }); return Ok(found); }
    for p in partition::table(disk)? {
        let mut slice = Slice { disk: &mut *disk, start: p.start, len: p.len };
        if let Ok(volume) = probe(&mut slice) { found.push(Found { start: p.start, len: p.len, volume }); }
    }
    Ok(found)
}

/// Little-endian fields.
pub(crate) fn le16(b: &[u8], at: usize) -> u16 { u16::from_le_bytes([b[at], b[at + 1]]) }
pub(crate) fn le32(b: &[u8], at: usize) -> u32 { u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) }
pub(crate) fn le64(b: &[u8], at: usize) -> u64 { u64::from_le_bytes(b[at..at + 8].try_into().unwrap()) }

/// Days from 1970-01-01 to a civil date (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
