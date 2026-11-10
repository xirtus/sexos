// SPDX-License-Identifier: GPL-3.0-only
//! Disk layouts SexOS shares with other systems: GPT partition tables, the FAT EFI System
//! partition every UEFI system boots from, and the plan `install` follows to put SexOS in
//! free space without touching anything that is already there.
//!
//! Everything here works on a [`BlockDev`] of 512-byte sectors, so the same code runs in
//! Files (over the disk driver) and in host tests (over a byte vector).
#![cfg_attr(not(test), no_std)]
extern crate alloc;

pub mod crc32;
pub mod fat;
pub mod gpt;
pub mod install;
pub mod probe;
pub mod update;

pub const SECTOR: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The device refused or failed a transfer.
    Io,
    /// A structure on the disk is not what it claims to be (bad signature, checksum, bounds).
    Corrupt,
    /// A valid layout this code does not handle (FAT12, 4Kn sectors, ...).
    Unsupported,
    /// Not enough room: no free extent, directory or cluster run large enough.
    NoSpace,
    NotFound,
    Exists,
    /// A name or argument that cannot be represented.
    Invalid,
    /// An update whose signature does not check out with the release key, or whose
    /// image is not the one its signed index names.
    BadSignature,
    /// An update already waits in the other slot for its first start.
    Pending,
    /// An update no newer than the system that is running.
    Older,
}

/// A run of 512-byte sectors. `read`/`write` take whole sectors (`buf.len()` a multiple of 512).
pub trait BlockDev {
    fn sectors(&self) -> u64;
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), Error>;
    fn write(&mut self, lba: u64, buf: &[u8]) -> Result<(), Error>;
}

/// A part of another device, addressed from its own sector 0: a partition.
pub struct Window<'a, D: BlockDev + ?Sized> { pub dev: &'a mut D, pub base: u64, pub len: u64 }
impl<'a, D: BlockDev + ?Sized> Window<'a, D> {
    pub fn new(dev: &'a mut D, base: u64, len: u64) -> Result<Self, Error> {
        if base.checked_add(len).is_none_or(|end| end > dev.sectors()) { return Err(Error::Invalid); }
        Ok(Self { dev, base, len })
    }
    fn check(&self, lba: u64, bytes: usize) -> Result<(), Error> {
        if bytes % SECTOR != 0 { return Err(Error::Invalid); }
        let n = (bytes / SECTOR) as u64;
        if lba.checked_add(n).is_none_or(|end| end > self.len) { return Err(Error::Invalid); }
        Ok(())
    }
}
impl<D: BlockDev + ?Sized> BlockDev for Window<'_, D> {
    fn sectors(&self) -> u64 { self.len }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), Error> { self.check(lba, buf.len())?; self.dev.read(self.base + lba, buf) }
    fn write(&mut self, lba: u64, buf: &[u8]) -> Result<(), Error> { self.check(lba, buf.len())?; self.dev.write(self.base + lba, buf) }
}

/// A device held in memory: tests, and boot images Files already has mapped.
pub struct MemDev<B: AsRef<[u8]>>(pub B);
impl<B: AsRef<[u8]> + AsMut<[u8]>> BlockDev for MemDev<B> {
    fn sectors(&self) -> u64 { (self.0.as_ref().len() / SECTOR) as u64 }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), Error> {
        let at = (lba as usize).checked_mul(SECTOR).ok_or(Error::Invalid)?;
        let src = self.0.as_ref().get(at..at + buf.len()).ok_or(Error::Invalid)?;
        if buf.len() % SECTOR != 0 { return Err(Error::Invalid); }
        buf.copy_from_slice(src); Ok(())
    }
    fn write(&mut self, lba: u64, buf: &[u8]) -> Result<(), Error> {
        let at = (lba as usize).checked_mul(SECTOR).ok_or(Error::Invalid)?;
        if buf.len() % SECTOR != 0 { return Err(Error::Invalid); }
        self.0.as_mut().get_mut(at..at + buf.len()).ok_or(Error::Invalid)?.copy_from_slice(buf); Ok(())
    }
}
/// A read-only device over borrowed bytes (a mapped boot image).
pub struct RoDev<'a>(pub &'a [u8]);
impl BlockDev for RoDev<'_> {
    fn sectors(&self) -> u64 { (self.0.len() / SECTOR) as u64 }
    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), Error> {
        let at = (lba as usize).checked_mul(SECTOR).ok_or(Error::Invalid)?;
        if buf.len() % SECTOR != 0 { return Err(Error::Invalid); }
        buf.copy_from_slice(self.0.get(at..at + buf.len()).ok_or(Error::Invalid)?); Ok(())
    }
    fn write(&mut self, _: u64, _: &[u8]) -> Result<(), Error> { Err(Error::Io) }
}
