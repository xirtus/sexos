// SPDX-License-Identifier: GPL-3.0-only
//! Guest RAM: one page-aligned block of the VMM's own heap that the kernel maps into
//! the VM at guest-physical 0 (`SYS_VM` LEND_MEMORY). Every access by guest-physical
//! address is bounds-checked here; the guest picks those addresses, so nothing here
//! may panic or reach past the block.
use alloc::alloc::{alloc_zeroed, dealloc, Layout};
use core::ptr::NonNull;

pub const PAGE: u64 = 4096;

/// A guest-physical access outside RAM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault { pub gpa: u64, pub len: u64 }

pub struct Ram { base: NonNull<u8>, len: usize }

// The block is owned exclusively (the VM sees it too, but only while a vCPU of this
// VMM runs, and that happens on the thread that owns this).
unsafe impl Send for Ram {}

impl Ram {
    /// `bytes` of zeroed RAM, page-aligned, a whole number of pages. None when the heap
    /// has no room or `bytes` is 0 or not page-sized.
    pub fn new(bytes: usize) -> Option<Ram> {
        if bytes == 0 || bytes % PAGE as usize != 0 { return None; }
        let layout = Layout::from_size_align(bytes, PAGE as usize).ok()?;
        let base = NonNull::new(unsafe { alloc_zeroed(layout) })?;
        Some(Ram { base, len: bytes })
    }
    pub fn len(&self) -> u64 { self.len as u64 }
    pub fn is_empty(&self) -> bool { self.len == 0 }
    /// The block's address in the VMM, for lending it to the VM.
    pub fn host_address(&self) -> u64 { self.base.as_ptr() as u64 }
    pub fn as_slice(&self) -> &[u8] { unsafe { core::slice::from_raw_parts(self.base.as_ptr(), self.len) } }
    pub fn as_mut_slice(&mut self) -> &mut [u8] { unsafe { core::slice::from_raw_parts_mut(self.base.as_ptr(), self.len) } }

    fn range(&self, gpa: u64, len: usize) -> Result<core::ops::Range<usize>, Fault> {
        let fault = Fault { gpa, len: len as u64 };
        let end = gpa.checked_add(len as u64).ok_or(fault)?;
        if end > self.len as u64 { return Err(fault); }
        Ok(gpa as usize..end as usize)
    }
    pub fn slice(&self, gpa: u64, len: usize) -> Result<&[u8], Fault> {
        let r = self.range(gpa, len)?;
        Ok(&self.as_slice()[r])
    }
    pub fn slice_mut(&mut self, gpa: u64, len: usize) -> Result<&mut [u8], Fault> {
        let r = self.range(gpa, len)?;
        Ok(&mut self.as_mut_slice()[r])
    }
    pub fn read(&self, gpa: u64, out: &mut [u8]) -> Result<(), Fault> {
        out.copy_from_slice(self.slice(gpa, out.len())?);
        Ok(())
    }
    pub fn write(&mut self, gpa: u64, data: &[u8]) -> Result<(), Fault> {
        self.slice_mut(gpa, data.len())?.copy_from_slice(data);
        Ok(())
    }
    pub fn fill(&mut self, gpa: u64, len: usize, byte: u8) -> Result<(), Fault> {
        self.slice_mut(gpa, len)?.fill(byte);
        Ok(())
    }
    pub fn read_u8(&self, gpa: u64) -> Result<u8, Fault> { Ok(self.slice(gpa, 1)?[0]) }
    pub fn read_u16(&self, gpa: u64) -> Result<u16, Fault> { let mut b = [0; 2]; self.read(gpa, &mut b)?; Ok(u16::from_le_bytes(b)) }
    pub fn read_u32(&self, gpa: u64) -> Result<u32, Fault> { let mut b = [0; 4]; self.read(gpa, &mut b)?; Ok(u32::from_le_bytes(b)) }
    pub fn read_u64(&self, gpa: u64) -> Result<u64, Fault> { let mut b = [0; 8]; self.read(gpa, &mut b)?; Ok(u64::from_le_bytes(b)) }
    pub fn write_u8(&mut self, gpa: u64, v: u8) -> Result<(), Fault> { self.write(gpa, &[v]) }
    pub fn write_u16(&mut self, gpa: u64, v: u16) -> Result<(), Fault> { self.write(gpa, &v.to_le_bytes()) }
    pub fn write_u32(&mut self, gpa: u64, v: u32) -> Result<(), Fault> { self.write(gpa, &v.to_le_bytes()) }
    pub fn write_u64(&mut self, gpa: u64, v: u64) -> Result<(), Fault> { self.write(gpa, &v.to_le_bytes()) }
}

impl Drop for Ram {
    fn drop(&mut self) {
        // The layout was valid when the block was made.
        if let Ok(layout) = Layout::from_size_align(self.len, PAGE as usize) {
            unsafe { dealloc(self.base.as_ptr(), layout) };
        }
    }
}

/// Splits `pages` into loans of at most `chunk` pages: (first page, pages). The kernel
/// maps a loan under the global program-table lock, so a big guest is lent in pieces.
pub fn loans(pages: u64, chunk: u64) -> impl Iterator<Item = (u64, u64)> {
    let chunk = chunk.max(1);
    (0..pages.div_ceil(chunk)).map(move |i| (i * chunk, chunk.min(pages - i * chunk)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_are_checked_without_panics() {
        let mut ram = Ram::new(2 * PAGE as usize).unwrap();
        assert_eq!(ram.host_address() % PAGE, 0);
        ram.write_u32(0x1FFC, 0xDEAD_BEEF).unwrap();
        assert_eq!(ram.read_u32(0x1FFC), Ok(0xDEAD_BEEF));
        assert!(ram.read_u32(0x1FFD).is_err(), "straddles the end");
        assert!(ram.read_u64(u64::MAX - 3).is_err(), "wraps");
        assert!(ram.slice(0x2000, 0).is_ok(), "empty at the end is fine");
        assert!(ram.slice(0x2001, 0).is_err());
        assert!(Ram::new(100).is_none() && Ram::new(0).is_none());
    }

    #[test]
    fn loans_split_in_chunks() {
        let v: alloc::vec::Vec<_> = loans(10000, 4096).collect();
        assert_eq!(v, [(0, 4096), (4096, 4096), (8192, 1808)]);
        assert_eq!(loans(0, 4096).count(), 0);
        assert_eq!(loans(4096, 4096).collect::<alloc::vec::Vec<_>>(), [(0, 4096)]);
    }
}
