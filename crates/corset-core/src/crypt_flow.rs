// SPDX-License-Identifier: GPL-3.0-only
//! High-level BDSM encrypted volume workflow wrapping `sex-crypt`.
//! Provides Argon2id + XTS-AES-256 partition formatting, passphrase unlock into
//! sector-translating `EncryptedBlockIo`, passphrase rewrapping, and cryptographic header erasure.

use crate::device::BlockIo;
use crate::error::{CorsetError, IoError, IoErrorKind};
use alloc::vec;

pub use sex_crypt::Cost;

/// Size of the on-disk encrypted volume header in bytes.
pub const CRYPT_HEADER_BYTES: usize = sex_crypt::HEADER; // 4096 bytes
/// Size of the XTS-AES-256 master key (512 bits / 64 bytes).
pub const MASTER_KEY_BYTES: usize = sex_crypt::KEY; // 64 bytes

/// Formats and enrolls a new BDSM encrypted volume header on `dev`.
/// Writes the 4096-byte `sex-crypt` header at LBA 0 of the partition.
pub fn format_encrypted_volume<D: BlockIo + ?Sized>(
    dev: &mut D,
    passphrase: &[u8],
    salt: &[u8; 32],
    master_key: Option<&[u8; MASTER_KEY_BYTES]>,
    cost: Option<Cost>,
) -> Result<[u8; MASTER_KEY_BYTES], CorsetError> {
    if passphrase.is_empty() {
        return Err(CorsetError::InvalidParameter("passphrase cannot be empty"));
    }
    let sector_size = dev.sector_size();
    let header_sectors = (CRYPT_HEADER_BYTES as u64).div_ceil(sector_size as u64);
    if dev.sector_count() <= header_sectors {
        return Err(CorsetError::NoSpace("device too small to hold encrypted volume header and data"));
    }

    let default_cost = Cost::SMALL;
    let cost = cost.unwrap_or(default_cost);

    let master = match master_key {
        Some(k) => *k,
        None => {
            // Generate deterministic key from passphrase and salt
            let mut key = [0u8; MASTER_KEY_BYTES];
            for (i, b) in key.iter_mut().enumerate() {
                let p = passphrase.get(i % passphrase.len()).copied().unwrap_or(0);
                let s = salt[i % 32];
                *b = (i as u8).wrapping_mul(37) ^ p ^ s;
            }
            key
        }
    };

    let mut header_bytes = sex_crypt::header(passphrase, salt, &master, cost)?;

    // Write 4096-byte header to LBA 0
    dev.write_bytes_at(0, &header_bytes)?;
    dev.flush()?;
    header_bytes.fill(0);

    Ok(master)
}

/// Unlocks an existing BDSM encrypted volume using `passphrase`.
/// Returns a sector-translating `EncryptedBlockIo` wrapper over the volume.
pub fn unlock_encrypted_volume<'a, D: BlockIo + ?Sized>(
    dev: &'a mut D,
    passphrase: &[u8],
) -> Result<EncryptedBlockIo<'a, D>, CorsetError> {
    let mut header_buf = [0u8; CRYPT_HEADER_BYTES];
    dev.read_bytes_at(0, &mut header_buf)?;

    let mut master_key = sex_crypt::unlock(&header_buf, passphrase)?;
    header_buf.fill(0);
    let res = EncryptedBlockIo::new(dev, &master_key);
    master_key.fill(0);
    res
}

/// Changes the passphrase on an encrypted volume by rewrapping the master key in place.
pub fn rewrap_passphrase<D: BlockIo + ?Sized>(
    dev: &mut D,
    old_passphrase: &[u8],
    new_passphrase: &[u8],
    new_salt: &[u8; 32],
) -> Result<(), CorsetError> {
    let mut header_buf = [0u8; CRYPT_HEADER_BYTES];
    dev.read_bytes_at(0, &mut header_buf)?;

    let mut new_header = sex_crypt::rewrap(&header_buf, old_passphrase, new_passphrase, new_salt)?;
    header_buf.fill(0);
    dev.write_bytes_at(0, &new_header)?;
    dev.flush()?;
    new_header.fill(0);
    Ok(())
}

/// Cryptographically erases the encrypted volume header by overwriting it with zeros.
/// Guarantees that subsequent unlock attempts will fail with `CryptNotEncrypted`.
pub fn erase_encrypted_header<D: BlockIo + ?Sized>(dev: &mut D) -> Result<(), CorsetError> {
    let mut wipe_buf = [0u8; CRYPT_HEADER_BYTES];
    wipe_buf.fill(0);
    dev.write_bytes_at(0, &wipe_buf)?;
    dev.flush()?;
    Ok(())
}

/// A sector-translating, transparently encrypting and decrypting `BlockIo` view
/// of a BDSM encrypted partition.
pub struct EncryptedBlockIo<'a, D: BlockIo + ?Sized> {
    dev: &'a mut D,
    xts: sex_crypt::Xts,
    header_sectors: u64,
    sector_count: u64,
    sector_size: u32,
}

impl<'a, D: BlockIo + ?Sized> EncryptedBlockIo<'a, D> {
    pub fn new(dev: &'a mut D, master_key: &[u8; MASTER_KEY_BYTES]) -> Result<Self, CorsetError> {
        let sector_size = dev.sector_size();
        let header_sectors = (CRYPT_HEADER_BYTES as u64).div_ceil(sector_size as u64);
        let total = dev.sector_count();
        if total <= header_sectors {
            return Err(CorsetError::NoSpace("device too small for encrypted payload"));
        }
        let sector_count = total - header_sectors;
        let xts = sex_crypt::Xts::new(master_key);

        Ok(Self {
            dev,
            xts,
            header_sectors,
            sector_count,
            sector_size,
        })
    }

    pub fn usable_sectors(&self) -> u64 {
        self.sector_count
    }
}

impl<'a, D: BlockIo + ?Sized> core::fmt::Debug for EncryptedBlockIo<'a, D> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("EncryptedBlockIo")
            .field("header_sectors", &self.header_sectors)
            .field("sector_count", &self.sector_count)
            .field("sector_size", &self.sector_size)
            .finish()
        }
}

impl<'a, D: BlockIo + ?Sized> BlockIo for EncryptedBlockIo<'a, D> {
    fn sector_size(&self) -> u32 {
        self.sector_size
    }

    fn sector_count(&self) -> u64 {
        self.sector_count
    }

    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), CorsetError> {
        let ss = self.sector_size as usize;
        if ss == 0 || buf.is_empty() || buf.len() % ss != 0 {
            return Err(CorsetError::Io(IoError {
                kind: IoErrorKind::MisalignedBuffer,
                lba,
                sectors: if ss > 0 { (buf.len() / ss) as u64 } else { 0 },
            }));
        }
        let sectors = (buf.len() / ss) as u64;
        let end = lba.checked_add(sectors).ok_or(CorsetError::Io(IoError {
            kind: IoErrorKind::OutOfBounds,
            lba,
            sectors,
        }))?;
        if end > self.sector_count {
            return Err(CorsetError::Io(IoError {
                kind: IoErrorKind::OutOfBounds,
                lba,
                sectors,
            }));
        }

        let physical_lba = self.header_sectors.checked_add(lba).ok_or(CorsetError::Io(IoError {
            kind: IoErrorKind::OutOfBounds,
            lba,
            sectors,
        }))?;
        let phys_end = physical_lba.checked_add(sectors).ok_or(CorsetError::Io(IoError {
            kind: IoErrorKind::OutOfBounds,
            lba,
            sectors,
        }))?;
        if phys_end > self.dev.sector_count() {
            return Err(CorsetError::Io(IoError {
                kind: IoErrorKind::OutOfBounds,
                lba,
                sectors,
            }));
        }

        self.dev.read_sectors(physical_lba, buf)?;

        // Decrypt each sector in-place using its relative logical sector as the XTS data unit
        for (i, chunk) in buf.chunks_exact_mut(ss).enumerate() {
            let unit = lba.checked_add(i as u64).ok_or(CorsetError::Io(IoError {
                kind: IoErrorKind::OutOfBounds,
                lba,
                sectors,
            }))?;
            self.xts.decrypt(unit, chunk);
        }

        Ok(())
    }

    fn write_sectors(&mut self, lba: u64, buf: &[u8]) -> Result<(), CorsetError> {
        let ss = self.sector_size as usize;
        if ss == 0 || buf.is_empty() || buf.len() % ss != 0 {
            return Err(CorsetError::Io(IoError {
                kind: IoErrorKind::MisalignedBuffer,
                lba,
                sectors: if ss > 0 { (buf.len() / ss) as u64 } else { 0 },
            }));
        }
        let sectors = (buf.len() / ss) as u64;
        let end = lba.checked_add(sectors).ok_or(CorsetError::Io(IoError {
            kind: IoErrorKind::OutOfBounds,
            lba,
            sectors,
        }))?;
        if end > self.sector_count {
            return Err(CorsetError::Io(IoError {
                kind: IoErrorKind::OutOfBounds,
                lba,
                sectors,
            }));
        }

        let physical_lba = self.header_sectors.checked_add(lba).ok_or(CorsetError::Io(IoError {
            kind: IoErrorKind::OutOfBounds,
            lba,
            sectors,
        }))?;
        let phys_end = physical_lba.checked_add(sectors).ok_or(CorsetError::Io(IoError {
            kind: IoErrorKind::OutOfBounds,
            lba,
            sectors,
        }))?;
        if phys_end > self.dev.sector_count() {
            return Err(CorsetError::Io(IoError {
                kind: IoErrorKind::OutOfBounds,
                lba,
                sectors,
            }));
        }

        // Buffer and encrypt each sector with XTS-AES-256
        let mut encrypted = vec![0u8; buf.len()];
        encrypted.copy_from_slice(buf);
        for (i, chunk) in encrypted.chunks_exact_mut(ss).enumerate() {
            let unit = lba.checked_add(i as u64).ok_or(CorsetError::Io(IoError {
                kind: IoErrorKind::OutOfBounds,
                lba,
                sectors,
            }))?;
            self.xts.encrypt(unit, chunk);
        }

        self.dev.write_sectors(physical_lba, &encrypted)?;
        encrypted.fill(0);

        Ok(())
    }

    fn flush(&mut self) -> Result<(), CorsetError> {
        self.dev.flush()
    }
}
