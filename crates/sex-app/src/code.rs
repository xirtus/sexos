// SPDX-License-Identifier: GPL-3.0-only
//! Machine code a program makes while it runs: the memory a JIT compiler writes to.
//!
//! A `CodeBuf` is whole pages of the program's own heap. It is either writable data
//! (`Open`) or executable code (`Sealed`), never both: `seal` asks the kernel to turn
//! the pages into code, and the kernel first checks that they hold no instruction able
//! to change the program's protection keys (`sex_pdx::program_code`). `open` turns them
//! back into data for patching. The pages go back to the heap, as data, on drop.
//!
//! The buffer is page-aligned and a whole number of pages, so the heap never keeps its
//! own bookkeeping inside a sealed page.
use alloc::alloc::{alloc_zeroed, dealloc};
use core::alloc::Layout;
use sex_pdx::program_code as code;

const PAGE: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The heap had no room.
    NoMemory,
    /// The kernel refused; the status is one of `sex_pdx::program_code`'s.
    Refused(u64),
    /// The bytes hold an instruction code may not contain, at this offset.
    Blocked(usize),
    /// More bytes than the buffer holds.
    TooBig,
}

pub struct CodeBuf { base: *mut u8, pages: usize, sealed: bool }

impl CodeBuf {
    /// A writable buffer of at least `bytes` bytes, zeroed.
    pub fn new(bytes: usize) -> Result<Self, Error> {
        let pages = bytes.max(1).div_ceil(PAGE);
        let layout = Layout::from_size_align(pages * PAGE, PAGE).map_err(|_| Error::TooBig)?;
        let base = unsafe { alloc_zeroed(layout) };
        if base.is_null() { return Err(Error::NoMemory); }
        Ok(Self { base, pages, sealed: false })
    }
    pub fn len(&self) -> usize { self.pages * PAGE }
    pub fn is_empty(&self) -> bool { false }
    pub fn is_sealed(&self) -> bool { self.sealed }
    pub fn as_ptr(&self) -> *const u8 { self.base }

    /// The bytes, for writing code. Only while open.
    pub fn bytes_mut(&mut self) -> Option<&mut [u8]> {
        if self.sealed { return None; }
        Some(unsafe { core::slice::from_raw_parts_mut(self.base, self.len()) })
    }
    /// Copies `code` to the start and seals the buffer.
    pub fn load(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if bytes.len() > self.len() { return Err(Error::TooBig); }
        if self.sealed { self.open()?; }
        let out = self.bytes_mut().ok_or(Error::Refused(code::DENIED))?;
        out[..bytes.len()].copy_from_slice(bytes);
        // Fill the rest with int3, so a stray jump stops instead of running old bytes.
        out[bytes.len()..].fill(0xCC);
        self.seal()
    }
    /// Makes the buffer executable and read-only.
    pub fn seal(&mut self) -> Result<(), Error> {
        if self.sealed { return Ok(()); }
        match code::seal(self.base as u64, self.pages as u64) {
            Ok(_) => { self.sealed = true; Ok(()) }
            Err((code::BLOCKED, at)) => Err(Error::Blocked(at as usize)),
            Err((e, _)) => Err(Error::Refused(e)),
        }
    }
    /// Makes the buffer writable data again.
    pub fn open(&mut self) -> Result<(), Error> {
        if !self.sealed { return Ok(()); }
        code::open(self.base as u64, self.pages as u64).map_err(|(e, _)| Error::Refused(e))?;
        self.sealed = false;
        Ok(())
    }
    /// The code at `offset` as a function. Calling it is as unsafe as the code is.
    ///
    /// # Safety
    /// The buffer must be sealed and hold a function of type `F` at `offset`, and
    /// `F` must be a function pointer type.
    pub unsafe fn entry<F: Copy>(&self, offset: usize) -> F {
        assert!(self.sealed && offset < self.len());
        assert_eq!(core::mem::size_of::<F>(), core::mem::size_of::<usize>());
        let at = self.base.add(offset);
        core::mem::transmute_copy(&at)
    }
}

impl Drop for CodeBuf {
    fn drop(&mut self) {
        // The heap writes to freed memory, so it must be data first. If the kernel
        // refused, leak the pages rather than fault.
        if self.open().is_err() { return; }
        unsafe { dealloc(self.base, Layout::from_size_align_unchecked(self.pages * PAGE, PAGE)) };
    }
}
