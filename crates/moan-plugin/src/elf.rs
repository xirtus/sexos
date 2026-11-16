//! Deliberately narrow ELF64 shared-object admission. This is not a Linux loader.
//! Only source-built, freestanding x86_64 SysV objects are supported. The loader
//! never searches dependencies, runs ELF constructors, or resolves ambient symbols.
use alloc::vec::Vec;
use core::ops::Range;
pub const PAGE: usize = 4096;
pub const MAX_FILE: usize = 16 * 1024 * 1024;
pub const MAX_IMAGE: usize = 32 * 1024 * 1024;
const MAX_HEADERS: usize = 128;
const MAX_RELOCS: usize = 65536;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Format,
    Bounds,
    Architecture,
    Permission,
    Dependency,
    Relocation,
    Symbol,
    Constructors,
}
#[derive(Clone, Debug)]
pub struct Segment {
    pub file: Range<usize>,
    pub memory: Range<usize>,
    pub executable: bool,
    pub writable: bool,
}
#[derive(Clone, Copy, Debug)]
struct Section {
    kind: u32,
    offset: usize,
    size: usize,
    link: usize,
    entsize: usize,
}
pub struct Image<'a> {
    bytes: &'a [u8],
    pub segments: Vec<Segment>,
    sections: Vec<Section>,
    pub size: usize,
}
fn u16b(b: &[u8], at: usize) -> Result<u16, Error> {
    Ok(u16::from_le_bytes(
        b.get(at..at + 2).ok_or(Error::Bounds)?.try_into().unwrap(),
    ))
}
fn u32b(b: &[u8], at: usize) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(
        b.get(at..at + 4).ok_or(Error::Bounds)?.try_into().unwrap(),
    ))
}
fn u64b(b: &[u8], at: usize) -> Result<u64, Error> {
    Ok(u64::from_le_bytes(
        b.get(at..at + 8).ok_or(Error::Bounds)?.try_into().unwrap(),
    ))
}
fn num(v: u64) -> Result<usize, Error> {
    usize::try_from(v).map_err(|_| Error::Bounds)
}
fn range(start: usize, len: usize, limit: usize) -> Result<Range<usize>, Error> {
    let end = start.checked_add(len).ok_or(Error::Bounds)?;
    if end > limit {
        return Err(Error::Bounds);
    }
    Ok(start..end)
}
impl<'a> Image<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < 64 || bytes.len() > MAX_FILE {
            return Err(Error::Bounds);
        }
        if &bytes[..7] != b"\x7fELF\x02\x01\x01" || u16b(bytes, 16)? != 3 || u32b(bytes, 20)? != 1 {
            return Err(Error::Format);
        }
        if u16b(bytes, 18)? != 62 {
            return Err(Error::Architecture);
        }
        let ph = num(u64b(bytes, 32)?)?;
        let sh = num(u64b(bytes, 40)?)?;
        let pn = u16b(bytes, 56)? as usize;
        let sn = u16b(bytes, 60)? as usize;
        if u16b(bytes, 52)? != 64
            || u16b(bytes, 54)? != 56
            || u16b(bytes, 58)? != 64
            || pn == 0
            || pn > MAX_HEADERS
            || sn == 0
            || sn > MAX_HEADERS
        {
            return Err(Error::Bounds);
        }
        range(ph, pn * 56, bytes.len())?;
        range(sh, sn * 64, bytes.len())?;
        let mut segments: Vec<Segment> = Vec::new();
        let mut size = 0;
        for i in 0..pn {
            let p = ph + i * 56;
            let kind = u32b(bytes, p)?;
            let flags = u32b(bytes, p + 4)?;
            if kind == 3 || kind == 7 {
                return Err(Error::Dependency);
            } // PT_INTERP/PT_TLS
            if kind == 0x6474e551 && flags & 1 != 0 {
                return Err(Error::Permission);
            }
            if kind == 2 {
                let off = num(u64b(bytes, p + 8)?)?;
                let len = num(u64b(bytes, p + 32)?)?;
                range(off, len, bytes.len())?;
                if len % 16 != 0 {
                    return Err(Error::Format);
                }
                for d in (off..off + len).step_by(16) {
                    match u64b(bytes, d)? {
                        1 | 15 | 29 => return Err(Error::Dependency),
                        12 | 13 | 25 | 26 | 32 => {
                            if u64b(bytes, d + 8)? != 0 {
                                return Err(Error::Constructors);
                            }
                        }
                        _ => (),
                    }
                }
            }
            if kind != 1 {
                continue;
            }
            let off = num(u64b(bytes, p + 8)?)?;
            let va = num(u64b(bytes, p + 16)?)?;
            let fs = num(u64b(bytes, p + 32)?)?;
            let ms = num(u64b(bytes, p + 40)?)?;
            let align = u64b(bytes, p + 48)?;
            if fs > ms
                || ms == 0
                || align != PAGE as u64
                || va % PAGE != off % PAGE
                || flags & !7 != 0
                || flags & 4 == 0
                || flags & 3 == 3
            {
                return Err(Error::Permission);
            }
            let file = range(off, fs, bytes.len())?;
            let memory = range(va, ms, MAX_IMAGE)?;
            let pages = va / PAGE..memory.end.div_ceil(PAGE);
            if segments.iter().any(|s| {
                let r = s.memory.start / PAGE..s.memory.end.div_ceil(PAGE);
                r.start < pages.end && pages.start < r.end
            }) {
                return Err(Error::Permission);
            }
            size = size.max(memory.end.div_ceil(PAGE) * PAGE);
            segments.push(Segment {
                file,
                memory,
                executable: flags & 1 != 0,
                writable: flags & 2 != 0,
            });
        }
        if segments.is_empty() || !segments.iter().any(|s| s.executable) {
            return Err(Error::Format);
        }
        let mut sections = Vec::with_capacity(sn);
        for i in 0..sn {
            let s = sh + i * 64;
            let kind = u32b(bytes, s + 4)?;
            let flags = u64b(bytes, s + 8)?;
            if kind == 9 || flags & 0x400 != 0 {
                return Err(Error::Relocation);
            }
            let section = Section {
                kind,
                offset: num(u64b(bytes, s + 24)?)?,
                size: num(u64b(bytes, s + 32)?)?,
                link: u32b(bytes, s + 40)? as usize,
                entsize: num(u64b(bytes, s + 56)?)?,
            };
            if kind != 8 {
                range(section.offset, section.size, bytes.len())?;
            }
            if section.link >= sn {
                return Err(Error::Bounds);
            }
            sections.push(section);
        }
        let image = Self {
            bytes,
            segments,
            sections,
            size,
        };
        // Preflight all relocation records and symbols BEFORE allocating executable memory.
        image.relocations(|_, _, _| Ok(()))?;
        Ok(image)
    }
    fn contains(&self, at: usize, len: usize) -> bool {
        self.segments
            .iter()
            .any(|s| at >= s.memory.start && at.checked_add(len).is_some_and(|e| e <= s.memory.end))
    }
    fn name(&self, table: usize, at: usize) -> Result<&'a [u8], Error> {
        let s = *self.sections.get(table).ok_or(Error::Symbol)?;
        if s.kind != 3 || at >= s.size {
            return Err(Error::Symbol);
        }
        let text = &self.bytes[s.offset + at..s.offset + s.size];
        let end = text
            .iter()
            .take(256)
            .position(|b| *b == 0)
            .ok_or(Error::Symbol)?;
        Ok(&text[..end])
    }
    fn symbol(&self, table: usize, index: usize) -> Result<(&'a [u8], Option<usize>), Error> {
        let s = *self.sections.get(table).ok_or(Error::Symbol)?;
        if s.kind != 11 || s.entsize != 24 || s.size % 24 != 0 || index >= s.size / 24 {
            return Err(Error::Symbol);
        }
        let p = s.offset + index * 24;
        let name = self.name(s.link, u32b(self.bytes, p)? as usize)?;
        let sh = u16b(self.bytes, p + 6)?;
        let value = num(u64b(self.bytes, p + 8)?)?;
        if sh == 0 {
            return Ok((name, None));
        }
        if sh as usize >= self.sections.len() || !self.contains(value, 1) {
            return Err(Error::Symbol);
        }
        Ok((name, Some(value)))
    }
    pub fn export(&self, name: &[u8]) -> Result<usize, Error> {
        let mut found = None;
        for (i, s) in self.sections.iter().enumerate() {
            if s.kind != 11 {
                continue;
            }
            if s.entsize != 24 || s.size % 24 != 0 {
                return Err(Error::Symbol);
            }
            for n in 1..s.size / 24 {
                let (key, value) = self.symbol(i, n)?;
                if key == name {
                    if found.is_some() {
                        return Err(Error::Symbol);
                    }
                    found = value;
                }
            }
        }
        found.ok_or(Error::Symbol)
    }
    /// Relocations are restricted to mapped eight-byte targets. Symbol resolution is
    /// supplied explicitly by the helper, never by ambient global process lookup.
    pub fn relocations(
        &self,
        mut visit: impl FnMut(usize, u32, Reloc<'a>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let mut count = 0;
        for s in &self.sections {
            if s.kind != 4 {
                continue;
            }
            if s.entsize != 24 || s.size % 24 != 0 {
                return Err(Error::Relocation);
            }
            count += s.size / 24;
            if count > MAX_RELOCS {
                return Err(Error::Bounds);
            }
            for p in (s.offset..s.offset + s.size).step_by(24) {
                let at = num(u64b(self.bytes, p)?)?;
                let info = u64b(self.bytes, p + 8)?;
                let kind = info as u32;
                let sym = (info >> 32) as usize;
                let addend = u64b(self.bytes, p + 16)? as i64;
                if !self.contains(at, 8) {
                    return Err(Error::Bounds);
                }
                let relocation = match kind {
                    8 if sym == 0 && addend >= 0 && self.contains(addend as usize, 1) => {
                        Reloc::Relative(addend as usize)
                    }
                    1 | 6 | 7 => {
                        let (name, value) = self.symbol(s.link, sym)?;
                        Reloc::Symbol {
                            name,
                            value,
                            addend,
                        }
                    }
                    _ => return Err(Error::Relocation),
                };
                visit(at, kind, relocation)?;
            }
        }
        Ok(())
    }
    pub fn copy(&self, out: &mut [u8]) -> Result<(), Error> {
        if out.len() != self.size {
            return Err(Error::Bounds);
        }
        out.fill(0);
        for s in &self.segments {
            out[s.memory.start..s.memory.start + s.file.len()]
                .copy_from_slice(&self.bytes[s.file.clone()]);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug)]
pub enum Reloc<'a> {
    Relative(usize),
    Symbol {
        name: &'a [u8],
        value: Option<usize>,
        addend: i64,
    },
}
#[cfg(test)]
mod tests {
    use super::*;
    fn put16(b: &mut [u8], p: usize, v: u16) {
        b[p..p + 2].copy_from_slice(&v.to_le_bytes())
    }
    fn put32(b: &mut [u8], p: usize, v: u32) {
        b[p..p + 4].copy_from_slice(&v.to_le_bytes())
    }
    fn put64(b: &mut [u8], p: usize, v: u64) {
        b[p..p + 8].copy_from_slice(&v.to_le_bytes())
    }
    fn fixture() -> Vec<u8> {
        let mut b = alloc::vec![0;8192];
        b[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        put16(&mut b, 16, 3);
        put16(&mut b, 18, 62);
        put32(&mut b, 20, 1);
        put64(&mut b, 32, 64);
        put64(&mut b, 40, 256);
        put16(&mut b, 52, 64);
        put16(&mut b, 54, 56);
        put16(&mut b, 56, 1);
        put16(&mut b, 58, 64);
        put16(&mut b, 60, 1);
        put32(&mut b, 64, 1);
        put32(&mut b, 68, 5);
        put64(&mut b, 72, 4096);
        put64(&mut b, 80, 4096);
        put64(&mut b, 96, 1);
        put64(&mut b, 104, 4096);
        put64(&mut b, 112, 4096);
        b[4096] = 0xc3;
        b
    }
    #[test]
    fn supported_image_copies_and_zeroes_bss() {
        let b = fixture();
        let image = Image::parse(&b).unwrap();
        let mut memory = alloc::vec![0xff;image.size];
        image.copy(&mut memory).unwrap();
        assert_eq!(memory[4096], 0xc3);
        assert!(memory[4097..].iter().all(|b| *b == 0));
        assert!(image.export(b"absent").is_err());
    }
    #[test]
    fn malformed_headers_never_panic() {
        let b = fixture();
        for n in 0..b.len() {
            let _ = Image::parse(&b[..n]);
        }
        for p in [32, 40, 72, 80, 96, 104, 112] {
            let mut v = b.clone();
            put64(&mut v, p, u64::MAX);
            assert!(Image::parse(&v).is_err(), "offset {p}");
        }
    }
    #[test]
    fn rejects_write_execute_tls_interpreter_and_overlap() {
        let b = fixture();
        let mut wx = b.clone();
        put32(&mut wx, 68, 7);
        assert!(matches!(Image::parse(&wx), Err(Error::Permission)));
        for kind in [3, 7] {
            let mut v = b.clone();
            put32(&mut v, 64, kind);
            assert!(matches!(Image::parse(&v), Err(Error::Dependency)));
        }
        let mut overlap = b;
        put16(&mut overlap, 56, 2);
        let p = overlap[64..120].to_vec();
        overlap[120..176].copy_from_slice(&p);
        assert!(matches!(Image::parse(&overlap), Err(Error::Permission)));
    }
    #[test]
    fn no_dynamic_dependency_or_constructor() {
        for tag in [1, 12, 25] {
            let mut b = fixture();
            put16(&mut b, 56, 2);
            put32(&mut b, 120, 2);
            put64(&mut b, 128, 512);
            put64(&mut b, 152, 16);
            put64(&mut b, 512, tag);
            put64(&mut b, 520, 1);
            assert!(Image::parse(&b).is_err());
        }
    }
}
