//! Mapping lives entirely in the helper's heap/domain. Failure cannot cause a
//! relocation write into the parent: every target was admitted against PT_LOAD.
use crate::elf::{self, Image, Reloc, PAGE};
use alloc::{
    alloc::{alloc_zeroed, dealloc},
    vec::Vec,
};
use core::alloc::Layout;
#[derive(Debug)]
pub enum Error {
    Elf(elf::Error),
    Memory,
    Import,
    Seal(u64),
}
impl From<elf::Error> for Error {
    fn from(e: elf::Error) -> Self {
        Self::Elf(e)
    }
}
pub struct Mapped {
    base: *mut u8,
    layout: Layout,
    sealed: Vec<(usize, usize)>,
}
impl Mapped {
    pub fn load(bytes: &[u8]) -> Result<Self, Error> {
        let image = Image::parse(bytes)?;
        // Resolve every import before allocation or executing any plugin instruction.
        image.relocations(|_, _, r| {
            if let Reloc::Symbol {
                name, value: None, ..
            } = r
            {
                if import(name).is_none() {
                    return Err(elf::Error::Symbol);
                }
            }
            Ok(())
        })?;
        let layout = Layout::from_size_align(image.size, PAGE).map_err(|_| Error::Memory)?;
        let base = unsafe { alloc_zeroed(layout) };
        if base.is_null() {
            return Err(Error::Memory);
        }
        let mut loaded = Self {
            base,
            layout,
            sealed: Vec::new(),
        };
        image.copy(unsafe { core::slice::from_raw_parts_mut(base, image.size) })?;
        image.relocations(|at, _, r| {
            let value = match r {
                Reloc::Relative(v) => base as usize + v,
                Reloc::Symbol {
                    name,
                    value,
                    addend,
                } => {
                    let target = match value {
                        Some(v) => base as usize + v,
                        None => import(name).ok_or(elf::Error::Symbol)?,
                    };
                    target
                        .checked_add_signed(addend as isize)
                        .ok_or(elf::Error::Relocation)?
                }
            };
            unsafe { core::ptr::write_unaligned(base.add(at).cast::<usize>(), value) };
            Ok(())
        })?;
        for s in &image.segments {
            if !s.executable {
                continue;
            }
            let at = s.memory.start / PAGE * PAGE;
            let pages = s.memory.end.div_ceil(PAGE) - at / PAGE;
            // Track successful partial chunks too, so failed sealing never frees RX pages.
            for offset in (0..pages).step_by(256) {
                let n = (pages - offset).min(256);
                let address = base as usize + at + offset * PAGE;
                sex_pdx::program_code::seal(address as u64, n as u64)
                    .map_err(|(e, _)| Error::Seal(e))?;
                loaded.sealed.push((address, n));
            }
        }
        Ok(loaded)
    }
    /// # Safety
    /// Returned addresses point to arbitrary native plugin code/data and may only
    /// be called with the real format ABI, inside an isolated disposable helper.
    pub unsafe fn export(&self, bytes: &[u8], name: &[u8]) -> Result<*const u8, Error> {
        let offset = Image::parse(bytes)?.export(name)?;
        if offset >= self.layout.size() {
            return Err(Error::Import);
        }
        Ok(self.base.add(offset))
    }
}
fn import(name: &[u8]) -> Option<usize> {
    Some(match name {
        b"moan_alloc" => plugin_alloc as *const () as usize,
        b"moan_free" => plugin_free as *const () as usize,
        b"calloc" => sex_libc::mem::calloc as *const () as usize,
        b"free" => sex_libc::mem::free as *const () as usize,
        b"strcmp" => sex_libc::string::strcmp as *const () as usize,
        b"snprintf" => sex_libc::stdio::snprintf as *const () as usize,
        _ => return None,
    })
}
impl Drop for Mapped {
    fn drop(&mut self) {
        for &(address, pages) in &self.sealed {
            if sex_pdx::program_code::open(address as u64, pages as u64).is_err() {
                return;
            }
        }
        unsafe { dealloc(self.base, self.layout) }
    }
}

unsafe extern "C" fn plugin_alloc(size: usize, align: usize) -> *mut u8 {
    if size == 0 || size > elf::MAX_IMAGE || align > 65536 {
        return core::ptr::null_mut();
    }
    match Layout::from_size_align(size, align) {
        Ok(layout) => alloc::alloc::alloc(layout),
        Err(_) => core::ptr::null_mut(),
    }
}
unsafe extern "C" fn plugin_free(ptr: *mut u8, size: usize, align: usize) {
    if ptr.is_null() {
        return;
    }
    if let Ok(layout) = Layout::from_size_align(size, align) {
        dealloc(ptr, layout)
    }
}
