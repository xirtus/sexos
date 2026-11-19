// SPDX-License-Identifier: GPL-3.0-only
//! Block device abstraction, in-memory test devices, and storage adapters.

use crate::error::{CorsetError, IoError, IoErrorKind};
use alloc::vec::Vec;

/// Standard sector size for legacy storage and default GPT layout (512 bytes).
pub const SECTOR_512: u32 = 512;
/// Advanced Format 4Kn sector size (4096 bytes).
pub const SECTOR_4096: u32 = 4096;

/// Common block I/O interface for disk partitions, images, and raw devices.
pub trait BlockIo {
    /// Logical sector size in bytes (typically 512 or 4096).
    fn sector_size(&self) -> u32;

    /// Total number of logical sectors on this device or partition.
    fn sector_count(&self) -> u64;

    /// Read whole sectors starting at `lba` into `buf`.
    /// `buf.len()` must be a non-zero multiple of `sector_size()`.
    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), CorsetError>;

    /// Write whole sectors starting at `lba` from `buf`.
    /// `buf.len()` must be a non-zero multiple of `sector_size()`.
    fn write_sectors(&mut self, lba: u64, buf: &[u8]) -> Result<(), CorsetError>;

    /// Flush any dirty volatile write caches to non-volatile storage.
    fn flush(&mut self) -> Result<(), CorsetError> {
        Ok(())
    }

    /// Total capacity in bytes.
    fn total_bytes(&self) -> u64 {
        self.sector_count().saturating_mul(self.sector_size() as u64)
    }

    /// Convenience helper to read bytes at byte offset `offset` into `buf`.
    fn read_bytes_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), CorsetError> {
        if buf.is_empty() {
            return Ok(());
        }
        let ss = self.sector_size() as u64;
        let mut sector_buf = alloc::vec![0u8; ss as usize];
        let mut done = 0;
        while done < buf.len() {
            let at = offset.checked_add(done as u64).ok_or(CorsetError::InvalidParameter("offset overflow"))?;
            let lba = at / ss;
            let within = (at % ss) as usize;
            let left = buf.len() - done;

            if within == 0 && left >= ss as usize {
                let whole_sectors = (left / ss as usize) as u64;
                let whole_bytes = (whole_sectors * ss) as usize;
                self.read_sectors(lba, &mut buf[done..done + whole_bytes])?;
                done += whole_bytes;
            } else {
                self.read_sectors(lba, &mut sector_buf)?;
                let take = (ss as usize - within).min(left);
                buf[done..done + take].copy_from_slice(&sector_buf[within..within + take]);
                done += take;
            }
        }
        Ok(())
    }

    /// Convenience helper to write bytes at byte offset `offset` from `buf`.
    fn write_bytes_at(&mut self, offset: u64, buf: &[u8]) -> Result<(), CorsetError> {
        if buf.is_empty() {
            return Ok(());
        }
        let ss = self.sector_size() as u64;
        let mut sector_buf = alloc::vec![0u8; ss as usize];
        let mut done = 0;
        while done < buf.len() {
            let at = offset.checked_add(done as u64).ok_or(CorsetError::InvalidParameter("offset overflow"))?;
            let lba = at / ss;
            let within = (at % ss) as usize;
            let left = buf.len() - done;

            if within == 0 && left >= ss as usize {
                let whole_sectors = (left / ss as usize) as u64;
                let whole_bytes = (whole_sectors * ss) as usize;
                self.write_sectors(lba, &buf[done..done + whole_bytes])?;
                done += whole_bytes;
            } else {
                self.read_sectors(lba, &mut sector_buf)?;
                let take = (ss as usize - within).min(left);
                sector_buf[within..within + take].copy_from_slice(&buf[done..done + take]);
                self.write_sectors(lba, &sector_buf)?;
                done += take;
            }
        }
        Ok(())
    }
}

/// In-memory block device for deterministic testing and formatting.
#[derive(Clone, Debug)]
pub struct MemBlockDevice {
    data: Vec<u8>,
    sector_size: u32,
}

impl MemBlockDevice {
    /// Create a zero-initialized memory block device.
    pub fn new(sector_count: u64, sector_size: u32) -> Result<Self, CorsetError> {
        if !matches!(sector_size, SECTOR_512 | SECTOR_4096) {
            return Err(CorsetError::UnsupportedSectorSize(sector_size));
        }
        let total_bytes = sector_count
            .checked_mul(sector_size as u64)
            .ok_or(CorsetError::InvalidParameter("capacity overflow"))?;
        let byte_len = usize::try_from(total_bytes)
            .map_err(|_| CorsetError::InvalidParameter("device too large for host memory"))?;

        let mut data = Vec::new();
        data.try_reserve_exact(byte_len)
            .map_err(|_| CorsetError::NoSpace("allocation failed"))?;
        data.resize(byte_len, 0);

        Ok(Self { data, sector_size })
    }

    /// Create a 512-byte sector memory block device.
    pub fn new_512(sector_count: u64) -> Result<Self, CorsetError> {
        Self::new(sector_count, SECTOR_512)
    }

    /// Create from existing byte buffer.
    pub fn from_bytes(bytes: Vec<u8>, sector_size: u32) -> Result<Self, CorsetError> {
        if !matches!(sector_size, SECTOR_512 | SECTOR_4096) {
            return Err(CorsetError::UnsupportedSectorSize(sector_size));
        }
        if bytes.len() % sector_size as usize != 0 {
            return Err(CorsetError::InvalidParameter("buffer size not a multiple of sector size"));
        }
        Ok(Self {
            data: bytes,
            sector_size,
        })
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.data
    }

    /// Utility for test verification: corrupt specific byte offsets.
    pub fn corrupt_bytes(&mut self, offset: usize, bytes: &[u8]) {
        if offset + bytes.len() <= self.data.len() {
            self.data[offset..offset + bytes.len()].copy_from_slice(bytes);
        }
    }
}

impl BlockIo for MemBlockDevice {
    fn sector_size(&self) -> u32 {
        self.sector_size
    }

    fn sector_count(&self) -> u64 {
        (self.data.len() / self.sector_size as usize) as u64
    }

    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), CorsetError> {
        let ss = self.sector_size as usize;
        if buf.is_empty() || buf.len() % ss != 0 {
            return Err(CorsetError::Io(IoError {
                kind: IoErrorKind::MisalignedBuffer,
                lba,
                sectors: (buf.len() / ss) as u64,
            }));
        }
        let sectors = (buf.len() / ss) as u64;
        let end_lba = lba.checked_add(sectors).ok_or(CorsetError::Io(IoError {
            kind: IoErrorKind::OutOfBounds,
            lba,
            sectors,
        }))?;
        if end_lba > self.sector_count() {
            return Err(CorsetError::Io(IoError {
                kind: IoErrorKind::OutOfBounds,
                lba,
                sectors,
            }));
        }
        let start_byte = (lba as usize) * ss;
        let end_byte = start_byte + buf.len();
        buf.copy_from_slice(&self.data[start_byte..end_byte]);
        Ok(())
    }

    fn write_sectors(&mut self, lba: u64, buf: &[u8]) -> Result<(), CorsetError> {
        let ss = self.sector_size as usize;
        if buf.is_empty() || buf.len() % ss != 0 {
            return Err(CorsetError::Io(IoError {
                kind: IoErrorKind::MisalignedBuffer,
                lba,
                sectors: (buf.len() / ss) as u64,
            }));
        }
        let sectors = (buf.len() / ss) as u64;
        let end_lba = lba.checked_add(sectors).ok_or(CorsetError::Io(IoError {
            kind: IoErrorKind::OutOfBounds,
            lba,
            sectors,
        }))?;
        if end_lba > self.sector_count() {
            return Err(CorsetError::Io(IoError {
                kind: IoErrorKind::OutOfBounds,
                lba,
                sectors,
            }));
        }
        let start_byte = (lba as usize) * ss;
        let end_byte = start_byte + buf.len();
        self.data[start_byte..end_byte].copy_from_slice(buf);
        Ok(())
    }

    fn flush(&mut self) -> Result<(), CorsetError> {
        Ok(())
    }
}

/// A slice or window of another block device, representing a partition.
pub struct PartitionIo<'a, D: BlockIo + ?Sized> {
    dev: &'a mut D,
    base_lba: u64,
    sector_count: u64,
}

impl<'a, D: BlockIo + ?Sized> PartitionIo<'a, D> {
    pub fn new(dev: &'a mut D, base_lba: u64, sector_count: u64) -> Result<Self, CorsetError> {
        let total = dev.sector_count();
        let end = base_lba.checked_add(sector_count).ok_or(CorsetError::InvalidParameter("partition overflow"))?;
        if end > total {
            return Err(CorsetError::InvalidParameter("partition exceeds device bounds"));
        }
        Ok(Self {
            dev,
            base_lba,
            sector_count,
        })
    }

    pub fn base_lba(&self) -> u64 {
        self.base_lba
    }
}

impl<'a, D: BlockIo + ?Sized> BlockIo for PartitionIo<'a, D> {
    fn sector_size(&self) -> u32 {
        self.dev.sector_size()
    }

    fn sector_count(&self) -> u64 {
        self.sector_count
    }

    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), CorsetError> {
        let ss = self.sector_size() as usize;
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
        let phys_lba = self.base_lba.checked_add(lba).ok_or(CorsetError::Io(IoError {
            kind: IoErrorKind::OutOfBounds,
            lba,
            sectors,
        }))?;
        let phys_end = phys_lba.checked_add(sectors).ok_or(CorsetError::Io(IoError {
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
        self.dev.read_sectors(phys_lba, buf)
    }

    fn write_sectors(&mut self, lba: u64, buf: &[u8]) -> Result<(), CorsetError> {
        let ss = self.sector_size() as usize;
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
        let phys_lba = self.base_lba.checked_add(lba).ok_or(CorsetError::Io(IoError {
            kind: IoErrorKind::OutOfBounds,
            lba,
            sectors,
        }))?;
        let phys_end = phys_lba.checked_add(sectors).ok_or(CorsetError::Io(IoError {
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
        self.dev.write_sectors(phys_lba, buf)
    }

    fn flush(&mut self) -> Result<(), CorsetError> {
        self.dev.flush()
    }
}

/// Adapter converting a `BlockIo` into a `sex_disk::BlockDev`.
pub struct SexDiskAdapter<'a, D: BlockIo + ?Sized>(pub &'a mut D);

impl<'a, D: BlockIo + ?Sized> sex_disk::BlockDev for SexDiskAdapter<'a, D> {
    fn sectors(&self) -> u64 {
        self.0.sector_count()
    }

    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), sex_disk::Error> {
        self.0.read_sectors(lba, buf).map_err(|e| match e {
            CorsetError::Io(_) => sex_disk::Error::Io,
            _ => sex_disk::Error::Invalid,
        })
    }

    fn write(&mut self, lba: u64, buf: &[u8]) -> Result<(), sex_disk::Error> {
        self.0.write_sectors(lba, buf).map_err(|e| match e {
            CorsetError::Io(_) => sex_disk::Error::Io,
            _ => sex_disk::Error::Invalid,
        })
    }
}

/// Adapter converting a `BlockIo` into a `sex_volumes::Disk`.
pub struct SexVolumesAdapter<'a, D: BlockIo + ?Sized>(pub &'a mut D);

impl<'a, D: BlockIo + ?Sized> sex_volumes::Disk for SexVolumesAdapter<'a, D> {
    fn sectors(&self) -> u64 {
        self.0.sector_count()
    }

    fn read(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), sex_volumes::Error> {
        self.0.read_sectors(lba, buf).map_err(|_| sex_volumes::Error::Io)
    }

    fn write(&mut self, lba: u64, buf: &[u8]) -> Result<(), sex_volumes::Error> {
        self.0.write_sectors(lba, buf).map_err(|_| sex_volumes::Error::Io)
    }

    fn flush(&mut self) -> Result<(), sex_volumes::Error> {
        self.0.flush().map_err(|_| sex_volumes::Error::Io)
    }
}

/// Adapter converting a `BlockIo` into a `sexfs::Device` (which runs on 4096-byte blocks).
pub struct SexFsAdapter<'a, D: BlockIo + ?Sized>(pub &'a mut D);

impl<'a, D: BlockIo + ?Sized> sexfs::Device for SexFsAdapter<'a, D> {
    fn blocks(&self) -> u64 {
        let total_bytes = self.0.sector_count() * self.0.sector_size() as u64;
        total_bytes / (sexfs::BLOCK as u64)
    }

    fn read(&mut self, block: u64, out: &mut [u8; sexfs::BLOCK]) -> Result<(), ()> {
        let sectors_per_block = (sexfs::BLOCK as u64) / (self.0.sector_size() as u64);
        let start_lba = block.checked_mul(sectors_per_block).ok_or(())?;
        self.0.read_sectors(start_lba, out).map_err(|_| ())
    }

    fn write(&mut self, block: u64, data: &[u8; sexfs::BLOCK]) -> Result<(), ()> {
        let sectors_per_block = (sexfs::BLOCK as u64) / (self.0.sector_size() as u64);
        let start_lba = block.checked_mul(sectors_per_block).ok_or(())?;
        self.0.write_sectors(start_lba, data).map_err(|_| ())
    }

    fn flush(&mut self) -> Result<(), ()> {
        self.0.flush().map_err(|_| ())
    }

    fn run_blocks(&self) -> usize {
        16
    }

    fn read_run(&mut self, first: u64, out: &mut [u8]) -> Result<(), ()> {
        let sectors_per_block = (sexfs::BLOCK as u64) / (self.0.sector_size() as u64);
        let start_lba = first.checked_mul(sectors_per_block).ok_or(())?;
        self.0.read_sectors(start_lba, out).map_err(|_| ())
    }

    fn write_run(&mut self, first: u64, data: &[u8]) -> Result<(), ()> {
        let sectors_per_block = (sexfs::BLOCK as u64) / (self.0.sector_size() as u64);
        let start_lba = first.checked_mul(sectors_per_block).ok_or(())?;
        self.0.write_sectors(start_lba, data).map_err(|_| ())
    }
}
