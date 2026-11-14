// SPDX-License-Identifier: GPL-3.0-only
//! Encrypted partitions, in sex-crypt's scheme as the encrypted home uses it: the first
//! 4 KiB are the SEXCRYPT header (the volume key wrapped by an Argon2id key from the
//! password); data unit `n` (4 KiB) sits at byte `(n + 1) * 4096` and is XTS-AES-256
//! sealed under tweak `n`. A unit that is all zeros on the disk reads as zeros (never
//! written), as on a fresh disk, which is also what the home image does.
//!
//! [`CryptDev`] is the unlocked volume: a [`BlockDevice`] of 512-byte sectors over the
//! data units, so any filesystem Corset makes can live inside.
use crate::{BlockDevice, Error, SECTOR};
use alloc::boxed::Box;
use sex_crypt::{Cost, Xts, HEADER, KEY};

const UNIT: usize = 4096;
const PER_UNIT: u64 = (UNIT / SECTOR) as u64;

/// Writes a new header to `part` for a fresh random volume key. The old contents are not
/// overwritten (this is not an erase); they read as noise through the new key.
pub fn create(part: &mut dyn BlockDevice, password: &[u8], salt: &[u8; 32], master: &[u8; KEY], cost: Cost) -> Result<(), Error> {
    if part.sectors() < 2 * PER_UNIT { return Err(Error::Format); }
    let mut h: Box<[u8; HEADER]> = Box::new([0; HEADER]);
    sex_crypt::header_into(&mut h, password, salt, master, cost).map_err(|_| Error::Format)?;
    part.write(0, &h[..])?;
    part.flush()
}

/// Sectors the unlocked volume of a partition of `sectors` holds.
pub fn inner_sectors(sectors: u64) -> u64 { (sectors / PER_UNIT).saturating_sub(1) * PER_UNIT }

pub struct CryptDev<'a> { dev: &'a mut dyn BlockDevice, xts: Xts, units: u64, buf: Box<[u8; UNIT]> }

impl<'a> CryptDev<'a> {
    /// Unlocks `part` with `password`.
    pub fn open(part: &'a mut dyn BlockDevice, password: &[u8]) -> Result<Self, Error> {
        let mut h: Box<[u8; HEADER]> = Box::new([0; HEADER]);
        part.read(0, &mut h[..])?;
        let master = sex_crypt::unlock(&h[..], password).map_err(|e| match e {
            sex_crypt::Error::WrongPassword => Error::WrongPassword,
            sex_crypt::Error::NotEncrypted => Error::NotEncrypted,
            _ => Error::Corrupt,
        })?;
        Ok(Self::with_key(part, &master))
    }
    pub fn with_key(part: &'a mut dyn BlockDevice, master: &[u8; KEY]) -> Self {
        let units = (part.sectors() / PER_UNIT).saturating_sub(1);
        CryptDev { dev: part, xts: Xts::new(master), units, buf: Box::new([0; UNIT]) }
    }
    fn load(&mut self, unit: u64) -> Result<(), Error> {
        self.dev.read((unit + 1) * PER_UNIT, &mut self.buf[..])?;
        if self.buf.iter().any(|b| *b != 0) { self.xts.decrypt(unit, &mut self.buf[..]); }
        Ok(())
    }
    fn store(&mut self, unit: u64) -> Result<(), Error> {
        let mut sealed: Box<[u8; UNIT]> = self.buf.clone();
        self.xts.encrypt(unit, &mut sealed[..]);
        self.dev.write((unit + 1) * PER_UNIT, &sealed[..])
    }
}

impl BlockDevice for CryptDev<'_> {
    fn sectors(&self) -> u64 { self.units * PER_UNIT }
    fn read(&mut self, lba: u64, out: &mut [u8]) -> Result<(), Error> {
        if out.len() % SECTOR != 0 || lba + (out.len() / SECTOR) as u64 > self.sectors() { return Err(Error::Io); }
        let mut done = 0;
        while done < out.len() {
            let s = lba + (done / SECTOR) as u64;
            let (unit, at) = (s / PER_UNIT, (s % PER_UNIT) as usize * SECTOR);
            let take = (UNIT - at).min(out.len() - done);
            self.load(unit)?;
            out[done..done + take].copy_from_slice(&self.buf[at..at + take]);
            done += take;
        }
        Ok(())
    }
    fn write(&mut self, lba: u64, data: &[u8]) -> Result<(), Error> {
        if data.len() % SECTOR != 0 || lba + (data.len() / SECTOR) as u64 > self.sectors() { return Err(Error::Io); }
        let mut done = 0;
        while done < data.len() {
            let s = lba + (done / SECTOR) as u64;
            let (unit, at) = (s / PER_UNIT, (s % PER_UNIT) as usize * SECTOR);
            let take = (UNIT - at).min(data.len() - done);
            if take < UNIT { self.load(unit)?; }
            self.buf[at..at + take].copy_from_slice(&data[done..done + take]);
            self.store(unit)?;
            done += take;
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), Error> { self.dev.flush() }
}
