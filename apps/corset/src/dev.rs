// SPDX-License-Identifier: GPL-3.0-only
//! The disk the person handed Corset, as corset-core's block device: sectors through
//! Files (`sex_app::files::disks`), at most `IO_SECTORS` a request.
use alloc::boxed::Box;
use core::cell::{Cell, RefCell};
use corset_core::{BlockIo, CorsetError, IoError, IoErrorKind};
use sex_app::files::disks;
use sex_app::Window;
use sha2::{Digest, Sha256};

pub struct Raw<'a, 'b> {
    pub window: &'a RefCell<&'b mut Window>,
    pub id: u16,
    pub sectors: u64,
    /// Files' status for the last failure, to say why.
    pub status: &'a Cell<u8>,
}
const CHUNK: usize = disks::IO_SECTORS as usize * disks::SECTOR;
impl Raw<'_, '_> {
    fn failed(&self, status: u8, kind: IoErrorKind, lba: u64, len: usize) -> CorsetError {
        self.status.set(status);
        CorsetError::Io(IoError { kind, lba, sectors: (len / disks::SECTOR) as u64 })
    }
    fn check(&self, lba: u64, len: usize) -> Result<(), CorsetError> {
        if len == 0 || len % disks::SECTOR != 0 { return Err(self.failed(0, IoErrorKind::MisalignedBuffer, lba, len)); }
        match lba.checked_add((len / disks::SECTOR) as u64) {
            Some(end) if end <= self.sectors => Ok(()),
            _ => Err(self.failed(0, IoErrorKind::OutOfBounds, lba, len)),
        }
    }
}
impl BlockIo for Raw<'_, '_> {
    fn sector_size(&self) -> u32 { disks::SECTOR as u32 }
    fn sector_count(&self) -> u64 { self.sectors }
    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<(), CorsetError> {
        self.check(lba, buf.len())?;
        for (k, chunk) in buf.chunks_mut(CHUNK).enumerate() {
            let at = lba + (k * CHUNK / disks::SECTOR) as u64;
            let len = chunk.len();
            disks::read(&mut self.window.borrow_mut(), self.id, at, chunk).map_err(|e| self.failed(e, IoErrorKind::DeviceFailure, at, len))?;
        }
        Ok(())
    }
    fn write_sectors(&mut self, lba: u64, buf: &[u8]) -> Result<(), CorsetError> {
        self.check(lba, buf.len())?;
        for (k, chunk) in buf.chunks(CHUNK).enumerate() {
            let at = lba + (k * CHUNK / disks::SECTOR) as u64;
            disks::write(&mut self.window.borrow_mut(), self.id, at, chunk).map_err(|e| self.failed(e, IoErrorKind::DeviceFailure, at, chunk.len()))?;
        }
        Ok(())
    }
    fn flush(&mut self) -> Result<(), CorsetError> {
        disks::flush(&mut self.window.borrow_mut(), self.id).map_err(|e| { self.status.set(e); CorsetError::Io(IoError { kind: IoErrorKind::FlushFailed, lba: 0, sectors: 0 }) })
    }
}

/// Random bytes for GUIDs, serials, salts and keys: the CPU's generator (RDRAND), hashed
/// with a counter. None on a CPU without one.
pub fn random() -> Option<Box<dyn FnMut(&mut [u8])>> {
    if core::arch::x86_64::__cpuid(1).ecx & (1 << 30) == 0 { return None; }
    fn rdrand(out: &mut [u8]) {
        for chunk in out.chunks_mut(8) {
            let mut v = 0u64;
            for _ in 0..10 { if unsafe { core::arch::x86_64::_rdrand64_step(&mut v) } == 1 { break; } }
            chunk.copy_from_slice(&v.to_le_bytes()[..chunk.len()]);
        }
    }
    let mut pool = [0u8; 32];
    rdrand(&mut pool);
    let mut n = 0u64;
    Some(Box::new(move |out: &mut [u8]| {
        for chunk in out.chunks_mut(32) {
            let mut fresh = [0u8; 32];
            rdrand(&mut fresh);
            n += 1;
            let block: [u8; 32] = Sha256::new().chain_update(pool).chain_update(fresh).chain_update(n.to_le_bytes()).finalize().into();
            chunk.copy_from_slice(&block[..chunk.len()]);
            pool = Sha256::digest(block).into();
        }
    }))
}
