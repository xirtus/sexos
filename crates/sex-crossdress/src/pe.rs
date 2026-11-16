// SPDX-License-Identifier: GPL-3.0-only
//! PE32+ (64-bit Windows) images: parse the headers, lay the sections out at a base,
//! apply base relocations, and list the imports. Written from the Microsoft PE/COFF
//! specification. The image is untrusted input: every offset and size is checked, and
//! nothing here panics or reads past the file.
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No `MZ`/`PE\0\0`, or not a PE32+ x86-64 image.
    NotPe,
    /// A header, section, directory or name points outside the file or the image.
    Bounds,
    /// The image needs relocating and has none (`/FIXED`), or a relocation is unknown.
    Relocation,
    /// The image is bigger than this loader accepts.
    TooBig,
}

pub const MACHINE_AMD64: u16 = 0x8664;
const MAGIC_PE32_PLUS: u16 = 0x20B;
/// IMAGE_FILE_RELOCS_STRIPPED.
const RELOCS_STRIPPED: u16 = 0x0001;
/// IMAGE_FILE_DLL.
const FILE_DLL: u16 = 0x2000;
/// Largest image this loader lays out (SizeOfImage).
pub const MAX_IMAGE: u32 = 512 << 20;

pub const SCN_EXECUTE: u32 = 0x2000_0000;
pub const SCN_READ: u32 = 0x4000_0000;
pub const SCN_WRITE: u32 = 0x8000_0000;

const DIR_EXPORT: usize = 0;
const DIR_IMPORT: usize = 1;
const DIR_EXCEPTION: usize = 3;
const DIR_BASERELOC: usize = 5;
const DIR_TLS: usize = 9;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub name: [u8; 8],
    /// Offset in the image (RVA) and size in memory.
    pub rva: u32,
    pub virtual_size: u32,
    /// Where its bytes are in the file.
    pub raw_offset: u32,
    pub raw_size: u32,
    pub characteristics: u32,
}

impl Section {
    pub fn writable(&self) -> bool { self.characteristics & SCN_WRITE != 0 }
    pub fn executable(&self) -> bool { self.characteristics & SCN_EXECUTE != 0 }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Directory { pub rva: u32, pub size: u32 }

/// One imported function: which DLL, by name or ordinal, and the RVA of its slot in
/// the import address table (the loader writes the function's address there).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import { pub dll: String, pub name: Option<String>, pub ordinal: Option<u16>, pub slot_rva: u32 }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub machine: u16,
    pub characteristics: u16,
    pub dll_characteristics: u16,
    pub subsystem: u16,
    pub preferred_base: u64,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    pub section_alignment: u32,
    pub entry_rva: u32,
    pub stack_reserve: u64,
    pub stack_commit: u64,
    pub sections: Vec<Section>,
    pub directories: [Directory; 16],
}

fn u16_at(d: &[u8], o: usize) -> Result<u16, Error> {
    d.get(o..o.checked_add(2).ok_or(Error::Bounds)?).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or(Error::Bounds)
}
fn u32_at(d: &[u8], o: usize) -> Result<u32, Error> {
    d.get(o..o.checked_add(4).ok_or(Error::Bounds)?).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok_or(Error::Bounds)
}
fn u64_at(d: &[u8], o: usize) -> Result<u64, Error> {
    let b = d.get(o..o.checked_add(8).ok_or(Error::Bounds)?).ok_or(Error::Bounds)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(b);
    Ok(u64::from_le_bytes(a))
}
/// A NUL-terminated ASCII name at `o`, at most 255 bytes.
fn name_at(d: &[u8], o: usize) -> Result<String, Error> {
    let rest = d.get(o..).ok_or(Error::Bounds)?;
    let end = rest.iter().take(256).position(|&c| c == 0).ok_or(Error::Bounds)?;
    let s = &rest[..end];
    if !s.iter().all(|c| c.is_ascii_graphic()) { return Err(Error::Bounds); }
    Ok(s.iter().map(|&c| c as char).collect())
}

impl Image {
    pub fn parse(file: &[u8]) -> Result<Image, Error> {
        if file.get(0..2) != Some(b"MZ") { return Err(Error::NotPe); }
        let pe = u32_at(file, 0x3C)? as usize;
        if file.get(pe..pe + 4) != Some(b"PE\0\0") { return Err(Error::NotPe); }
        let coff = pe + 4;
        let machine = u16_at(file, coff)?;
        let sections = u16_at(file, coff + 2)? as usize;
        let opt_size = u16_at(file, coff + 16)? as usize;
        let characteristics = u16_at(file, coff + 18)?;
        let opt = coff + 20;
        if machine != MACHINE_AMD64 || u16_at(file, opt)? != MAGIC_PE32_PLUS || opt_size < 112 { return Err(Error::NotPe); }
        let entry_rva = u32_at(file, opt + 16)?;
        let preferred_base = u64_at(file, opt + 24)?;
        let section_alignment = u32_at(file, opt + 32)?;
        let size_of_image = u32_at(file, opt + 56)?;
        let size_of_headers = u32_at(file, opt + 60)?;
        let subsystem = u16_at(file, opt + 68)?;
        let dll_characteristics = u16_at(file, opt + 70)?;
        let stack_reserve = u64_at(file, opt + 72)?;
        let stack_commit = u64_at(file, opt + 80)?;
        let dir_count = (u32_at(file, opt + 108)? as usize).min(16);
        if opt_size < 112 + 8 * dir_count { return Err(Error::Bounds); }
        let mut directories = [Directory::default(); 16];
        for (i, d) in directories.iter_mut().enumerate().take(dir_count) {
            *d = Directory { rva: u32_at(file, opt + 112 + 8 * i)?, size: u32_at(file, opt + 116 + 8 * i)? };
        }
        if size_of_image == 0 || size_of_image > MAX_IMAGE { return Err(Error::TooBig); }
        if preferred_base % 0x1_0000 != 0 || section_alignment < 0x1000 || section_alignment % 0x1000 != 0 { return Err(Error::NotPe); }
        if size_of_headers > size_of_image || entry_rva >= size_of_image { return Err(Error::Bounds); }
        let table = opt + opt_size;
        let mut list = Vec::new();
        for i in 0..sections.min(96) {
            let o = table + 40 * i;
            let mut name = [0u8; 8];
            name.copy_from_slice(file.get(o..o + 8).ok_or(Error::Bounds)?);
            let s = Section {
                name,
                virtual_size: u32_at(file, o + 8)?,
                rva: u32_at(file, o + 12)?,
                raw_size: u32_at(file, o + 16)?,
                raw_offset: u32_at(file, o + 20)?,
                characteristics: u32_at(file, o + 36)?,
            };
            let mem = s.virtual_size.max(s.raw_size) as u64;
            if s.rva as u64 + mem > size_of_image as u64 || s.rva < size_of_headers { return Err(Error::Bounds); }
            if s.raw_size != 0 && s.raw_offset as u64 + s.raw_size as u64 > file.len() as u64 { return Err(Error::Bounds); }
            list.push(s);
        }
        Ok(Image {
            machine, characteristics, dll_characteristics, subsystem, preferred_base, size_of_image, size_of_headers,
            section_alignment, entry_rva, stack_reserve, stack_commit, sections: list, directories,
        })
    }

    pub fn is_dll(&self) -> bool { self.characteristics & FILE_DLL != 0 }
    /// The image can only run at its preferred base.
    pub fn fixed(&self) -> bool { self.characteristics & RELOCS_STRIPPED != 0 || self.directories[DIR_BASERELOC].size == 0 }
    pub fn has_tls(&self) -> bool { self.directories[DIR_TLS].size != 0 }
    pub fn exception_directory(&self) -> Directory { self.directories[DIR_EXCEPTION] }

    /// The image as it is in memory at `base`: headers and sections copied to their RVAs,
    /// the rest zero, base relocations applied when `base` is not the preferred base.
    pub fn lay_out(&self, file: &[u8], base: u64) -> Result<Vec<u8>, Error> {
        let mut mem = Vec::new();
        mem.try_reserve_exact(self.size_of_image as usize).map_err(|_| Error::TooBig)?;
        mem.resize(self.size_of_image as usize, 0);
        let h = (self.size_of_headers as usize).min(file.len());
        mem[..h].copy_from_slice(&file[..h]);
        for s in &self.sections {
            let n = s.raw_size.min(s.virtual_size.max(s.raw_size)) as usize;
            if n == 0 { continue; }
            let src = file.get(s.raw_offset as usize..s.raw_offset as usize + n).ok_or(Error::Bounds)?;
            mem[s.rva as usize..s.rva as usize + n].copy_from_slice(src);
        }
        if base != self.preferred_base {
            if self.fixed() { return Err(Error::Relocation); }
            self.relocate(&mut mem, base.wrapping_sub(self.preferred_base))?;
        }
        Ok(mem)
    }

    /// Applies the base relocation blocks to a laid-out image moved by `delta`.
    fn relocate(&self, mem: &mut [u8], delta: u64) -> Result<(), Error> {
        let dir = self.directories[DIR_BASERELOC];
        let (mut o, end) = (dir.rva as usize, dir.rva as usize + dir.size as usize);
        if end > mem.len() { return Err(Error::Bounds); }
        while o + 8 <= end {
            let page = u32_at(mem, o)? as usize;
            let size = u32_at(mem, o + 4)? as usize;
            if size < 8 || o + size > end { return Err(Error::Bounds); }
            for i in 0..(size - 8) / 2 {
                let e = u16_at(mem, o + 8 + 2 * i)?;
                let at = page + (e & 0xFFF) as usize;
                match e >> 12 {
                    0 => {} // IMAGE_REL_BASED_ABSOLUTE: padding
                    3 => { // HIGHLOW
                        let v = u32_at(mem, at)?.wrapping_add(delta as u32);
                        mem.get_mut(at..at + 4).ok_or(Error::Bounds)?.copy_from_slice(&v.to_le_bytes());
                    }
                    10 => { // DIR64
                        let v = u64_at(mem, at)?.wrapping_add(delta);
                        mem.get_mut(at..at + 8).ok_or(Error::Bounds)?.copy_from_slice(&v.to_le_bytes());
                    }
                    _ => return Err(Error::Relocation),
                }
            }
            o += size;
        }
        Ok(())
    }

    /// The imports of a laid-out image (`lay_out`), in import-table order.
    pub fn imports(&self, mem: &[u8]) -> Result<Vec<Import>, Error> {
        let dir = self.directories[DIR_IMPORT];
        let mut out = Vec::new();
        if dir.size == 0 { return Ok(out); }
        let mut d = dir.rva as usize;
        loop {
            let lookup = u32_at(mem, d)?;
            let name = u32_at(mem, d + 12)?;
            let iat = u32_at(mem, d + 16)?;
            if lookup == 0 && name == 0 && iat == 0 { break; }
            let dll = name_at(mem, name as usize)?;
            // The lookup table names the functions; old linkers leave it 0 and use the IAT.
            let mut t = if lookup != 0 { lookup } else { iat } as usize;
            let mut slot = iat;
            loop {
                let e = u64_at(mem, t)?;
                if e == 0 { break; }
                let (fname, ordinal) = if e >> 63 != 0 {
                    (None, Some(e as u16))
                } else {
                    // IMAGE_IMPORT_BY_NAME: a hint, then the name.
                    (Some(name_at(mem, (e as u32 as usize).checked_add(2).ok_or(Error::Bounds)?)?), None)
                };
                out.push(Import { dll: dll.clone(), name: fname, ordinal, slot_rva: slot });
                if out.len() > 65_536 { return Err(Error::TooBig); }
                t += 8;
                slot = slot.checked_add(8).ok_or(Error::Bounds)?;
            }
            d += 20;
        }
        Ok(out)
    }

    /// Exported names and their RVAs (forwarders included as they are).
    pub fn exports(&self, mem: &[u8]) -> Result<Vec<(String, u32)>, Error> {
        let dir = self.directories[DIR_EXPORT];
        let mut out = Vec::new();
        if dir.size == 0 { return Ok(out); }
        let d = dir.rva as usize;
        let functions = u32_at(mem, d + 28)? as usize;
        let names = u32_at(mem, d + 32)? as usize;
        let ordinals = u32_at(mem, d + 36)? as usize;
        let count = (u32_at(mem, d + 24)? as usize).min(65_536);
        for i in 0..count {
            let name = name_at(mem, u32_at(mem, names + 4 * i)? as usize)?;
            let index = u16_at(mem, ordinals + 2 * i)? as usize;
            out.push((name, u32_at(mem, functions + 4 * index)?));
        }
        Ok(out)
    }
}

/// Writes `address` into an import slot of a laid-out image.
pub fn bind(mem: &mut [u8], slot_rva: u32, address: u64) -> Result<(), Error> {
    let at = slot_rva as usize;
    mem.get_mut(at..at + 8).ok_or(Error::Bounds)?.copy_from_slice(&address.to_le_bytes());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_program_parses() {
        let img = Image::parse(crate::TEST_EXE).unwrap();
        assert_eq!(img.machine, MACHINE_AMD64);
        assert_eq!(img.preferred_base, 0x1_4000_0000);
        assert!(!img.is_dll() && !img.fixed() && !img.has_tls());
        assert!(img.sections.iter().any(|s| &s.name[..5] == b".text" && s.executable() && !s.writable()));
        assert!(img.sections.iter().any(|s| &s.name[..6] == b".rdata" && !s.writable()));
        let mem = img.lay_out(crate::TEST_EXE, img.preferred_base).unwrap();
        assert_eq!(&mem[..2], b"MZ");
        let imports = img.imports(&mem).unwrap();
        let names: Vec<&str> = imports.iter().filter_map(|i| i.name.as_deref()).collect();
        for want in ["GetStdHandle", "WriteFile", "ExitProcess", "AddVectoredExceptionHandler", "VirtualAlloc"] {
            assert!(names.contains(&want), "{want} is imported");
        }
        assert!(imports.iter().all(|i| i.dll.eq_ignore_ascii_case("kernel32.dll")));
    }

    #[test]
    fn relocation_moves_every_absolute_address() {
        let img = Image::parse(crate::TEST_EXE).unwrap();
        let at_home = img.lay_out(crate::TEST_EXE, img.preferred_base).unwrap();
        let delta = 0x7_0000_0000u64;
        let moved = img.lay_out(crate::TEST_EXE, img.preferred_base + delta).unwrap();
        let dir = img.directories[DIR_BASERELOC];
        assert!(dir.size > 0, "the test program keeps its relocations");
        let mut changed = 0;
        for i in (0..at_home.len() - 8).step_by(1) {
            let (a, b) = (u64_at(&at_home, i).unwrap(), u64_at(&moved, i).unwrap());
            if a != b && b.wrapping_sub(a) == delta { changed += 1; }
        }
        assert!(changed > 0, "some pointer moved by exactly delta");
    }

    #[test]
    fn hostile_headers_are_refused_without_panics() {
        assert_eq!(Image::parse(b""), Err(Error::NotPe));
        assert_eq!(Image::parse(b"MZ"), Err(Error::Bounds));
        let mut f = crate::TEST_EXE.to_vec();
        let pe = u32_at(&f, 0x3C).unwrap() as usize;
        // Machine: i386.
        let mut g = f.clone();
        g[pe + 4..pe + 6].copy_from_slice(&0x14Cu16.to_le_bytes());
        assert_eq!(Image::parse(&g), Err(Error::NotPe));
        // SizeOfImage huge.
        let opt = pe + 24;
        let mut g = f.clone();
        g[opt + 56..opt + 60].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(Image::parse(&g), Err(Error::TooBig));
        // Truncations at every length never panic.
        for n in 0..f.len().min(2048) { let _ = Image::parse(&f[..n]); }
        // A section whose raw data runs past the file.
        let sections_at = opt + u16_at(&f, pe + 20).unwrap() as usize;
        f[sections_at + 16..sections_at + 20].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
        assert_eq!(Image::parse(&f), Err(Error::Bounds));
    }

    #[test]
    fn a_fixed_image_cannot_move() {
        let mut img = Image::parse(crate::TEST_EXE).unwrap();
        img.characteristics |= RELOCS_STRIPPED;
        assert!(img.fixed());
        assert_eq!(img.lay_out(crate::TEST_EXE, img.preferred_base + 0x1_0000), Err(Error::Relocation));
        assert!(img.lay_out(crate::TEST_EXE, img.preferred_base).is_ok(), "but loads at its own base");
    }
}
