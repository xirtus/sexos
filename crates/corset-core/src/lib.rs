// SPDX-License-Identifier: GPL-3.0-only
//! Corset's engine: what is on a disk, a plan of changes to it checked before anything is
//! written, and the order those changes reach the disk in so that a power cut leaves a
//! disk that still makes sense. Also the health pages disks report (NVMe SMART/Health,
//! ATA SMART), parsed into one summary.
//!
//! The flow is read, plan, apply:
//!
//! 1. [`Disk::read`] takes a snapshot of a device: its GPT (or that it has none), every
//!    partition and what each holds.
//! 2. [`Plan`] collects [`Op`]s against that snapshot. Each op is checked as it is pushed
//!    (1 MiB alignment, overlap, GPT limits, filesystem minimum sizes, what can be resized
//!    or moved) against the layout the earlier ops leave, and each gets a sentence for the
//!    confirmation screen. Nothing touches the disk.
//! 3. [`apply`] re-reads the disk, refuses if it changed since the snapshot, and writes:
//!    first every change that lands in space the old table leaves free (new filesystems,
//!    moved copies), then the partition table (backup copy first, then the primary), then
//!    the changes that land inside old partitions. Until the table write the old table is
//!    intact and describes the disk exactly as before.
//!
//! Filesystems Corset can make: FAT32 (sex-disk's formatter, as the installer uses it),
//! exFAT (here, [`exfat`]) and sexfs (SexOS's own). ext4 is read and written by
//! sex-volumes but has no formatter in the tree; it is not offered. Encryption is
//! sex-crypt's scheme, the one the encrypted home uses: a 4 KiB SEXCRYPT header, then
//! XTS-AES-256 units of 4 KiB numbered from 0 ([`crypt`]).
#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod apply;
pub mod crypt;
pub mod detect;
pub mod exfat;
pub mod format;
pub mod health;
pub mod plan;
pub mod types;

pub use apply::{apply, Progress, Report};
pub use detect::{Content, Disk, Partition};
pub use format::FsKind;
pub use plan::{Op, Plan, Refusal, Secret, Step};
pub use sex_disk::gpt::Guid;

use alloc::string::String;
use alloc::vec::Vec;

pub const SECTOR: usize = 512;
/// Partitions start and end on whole MiB.
pub const ALIGN: u64 = (1 << 20) / SECTOR as u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The device refused or failed a transfer.
    Io,
    /// What is on the disk is not what it claims to be.
    Corrupt,
    /// The disk is not what the plan was made against any more: read it again.
    Changed,
    /// A filesystem or key could not be made (too small, out of memory, bad parameters).
    Format,
    /// A password that does not open an encrypted partition.
    WrongPassword,
    /// No SEXCRYPT header where one was expected.
    NotEncrypted,
}

/// Whole 512-byte sectors; `buf.len()` is always a multiple of [`SECTOR`].
pub trait BlockDevice {
    fn sectors(&self) -> u64;
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), Error>;
    fn write(&mut self, lba: u64, buf: &[u8]) -> Result<(), Error>;
    /// Everything written so far is on the medium.
    fn flush(&mut self) -> Result<(), Error>;
}

impl<T: BlockDevice + ?Sized> BlockDevice for &mut T {
    fn sectors(&self) -> u64 { (**self).sectors() }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), Error> { (**self).read(lba, buf) }
    fn write(&mut self, lba: u64, buf: &[u8]) -> Result<(), Error> { (**self).write(lba, buf) }
    fn flush(&mut self) -> Result<(), Error> { (**self).flush() }
}

/// A disk held in memory: tests, and images.
pub struct MemDisk(pub Vec<u8>);
impl MemDisk {
    pub fn zeroed(bytes: usize) -> Self { MemDisk(alloc::vec![0; bytes]) }
    fn range(&self, lba: u64, len: usize) -> Result<core::ops::Range<usize>, Error> {
        if len % SECTOR != 0 { return Err(Error::Io); }
        let at = usize::try_from(lba).ok().and_then(|l| l.checked_mul(SECTOR)).ok_or(Error::Io)?;
        let end = at.checked_add(len).filter(|e| *e <= self.0.len()).ok_or(Error::Io)?;
        Ok(at..end)
    }
}
impl BlockDevice for MemDisk {
    fn sectors(&self) -> u64 { (self.0.len() / SECTOR) as u64 }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), Error> { let r = self.range(lba, buf.len())?; buf.copy_from_slice(&self.0[r]); Ok(()) }
    fn write(&mut self, lba: u64, buf: &[u8]) -> Result<(), Error> { let r = self.range(lba, buf.len())?; self.0[r].copy_from_slice(buf); Ok(()) }
    fn flush(&mut self) -> Result<(), Error> { Ok(()) }
}

/// Sectors `base..base + len` of a device, addressed from 0: one partition.
pub struct Window<'a> { pub dev: &'a mut dyn BlockDevice, pub base: u64, pub len: u64 }
impl<'a> Window<'a> {
    pub fn new(dev: &'a mut dyn BlockDevice, base: u64, len: u64) -> Result<Self, Error> {
        if base.checked_add(len).is_none_or(|end| end > dev.sectors()) { return Err(Error::Io); }
        Ok(Window { dev, base, len })
    }
    fn check(&self, lba: u64, bytes: usize) -> Result<(), Error> {
        if bytes % SECTOR != 0 || lba.checked_add((bytes / SECTOR) as u64).is_none_or(|end| end > self.len) { return Err(Error::Io); }
        Ok(())
    }
}
impl BlockDevice for Window<'_> {
    fn sectors(&self) -> u64 { self.len }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), Error> { self.check(lba, buf.len())?; self.dev.read(self.base + lba, buf) }
    fn write(&mut self, lba: u64, buf: &[u8]) -> Result<(), Error> { self.check(lba, buf.len())?; self.dev.write(self.base + lba, buf) }
    fn flush(&mut self) -> Result<(), Error> { self.dev.flush() }
}

/// A [`BlockDevice`] as sex-disk's `BlockDev`, for its GPT and FAT code.
pub(crate) struct AsSexDisk<'a>(pub &'a mut dyn BlockDevice);
impl sex_disk::BlockDev for AsSexDisk<'_> {
    fn sectors(&self) -> u64 { self.0.sectors() }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), sex_disk::Error> { self.0.read(lba, buf).map_err(|_| sex_disk::Error::Io) }
    fn write(&mut self, lba: u64, buf: &[u8]) -> Result<(), sex_disk::Error> { self.0.write(lba, buf).map_err(|_| sex_disk::Error::Io) }
}

/// A [`BlockDevice`] as a sexfs device of 4 KiB blocks (a trailing part block is unused).
pub struct AsSexfs<'a>(pub &'a mut dyn BlockDevice);
const PER_BLOCK: u64 = (sexfs::BLOCK / SECTOR) as u64;
impl sexfs::Device for AsSexfs<'_> {
    fn blocks(&self) -> u64 { self.0.sectors() / PER_BLOCK }
    fn read(&mut self, block: u64, out: &mut [u8; sexfs::BLOCK]) -> Result<(), ()> { self.0.read(block * PER_BLOCK, out).map_err(|_| ()) }
    fn write(&mut self, block: u64, data: &[u8; sexfs::BLOCK]) -> Result<(), ()> { self.0.write(block * PER_BLOCK, data).map_err(|_| ()) }
    fn flush(&mut self) -> Result<(), ()> { self.0.flush().map_err(|_| ()) }
    fn run_blocks(&self) -> usize { 16 }
    fn read_run(&mut self, first: u64, out: &mut [u8]) -> Result<(), ()> { self.0.read(first * PER_BLOCK, out).map_err(|_| ()) }
    fn write_run(&mut self, first: u64, data: &[u8]) -> Result<(), ()> { self.0.write(first * PER_BLOCK, data).map_err(|_| ()) }
}

/// Memory for a sexfs on a device of `blocks` blocks, from the heap.
pub struct SexfsMemory {
    pub cache: Vec<[u8; sexfs::BLOCK]>,
    pub slots: Vec<sexfs::Slot>,
    pub used: Vec<u64>,
    pub pending: Vec<u64>,
    pub scratch: alloc::boxed::Box<[u8; sexfs::BLOCK]>,
}
impl SexfsMemory {
    pub fn new(blocks: u64, cache: usize) -> Self {
        let words = (blocks as usize).div_ceil(64) + 1;
        SexfsMemory { cache: alloc::vec![[0; sexfs::BLOCK]; cache], slots: alloc::vec![sexfs::Slot::EMPTY; cache],
            used: alloc::vec![0; words], pending: alloc::vec![0; words], scratch: alloc::boxed::Box::new([0; sexfs::BLOCK]) }
    }
    pub fn lend(&mut self) -> sexfs::Memory<'_> {
        sexfs::Memory { cache: &mut self.cache, slots: &mut self.slots, used: &mut self.used, pending: &mut self.pending, scratch: &mut self.scratch }
    }
}

/// `bytes` as people read sizes: "512 B", "33 MiB", "1.5 GiB".
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut u = 0;
    let mut whole = bytes;
    while whole >= 1024 && u + 1 < UNITS.len() { whole /= 1024; u += 1; }
    if u == 0 { return alloc::format!("{bytes} B"); }
    let unit = 1u128 << (10 * u);
    let tenths = (bytes as u128 * 10 + unit / 2) / unit;
    if tenths % 10 == 0 || tenths >= 1000 { alloc::format!("{} {}", (tenths + 5) / 10, UNITS[u]) } else { alloc::format!("{}.{} {}", tenths / 10, tenths % 10, UNITS[u]) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sizes_read_well() {
        assert_eq!(size_text(512), "512 B");
        assert_eq!(size_text(33 << 20), "33 MiB");
        assert_eq!(size_text(3 << 29), "1.5 GiB");
        assert_eq!(size_text(256 << 20), "256 MiB");
    }
}
