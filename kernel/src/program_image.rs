// SPDX-License-Identifier: GPL-3.0-only
//! Pure check of an untrusted program file, before any of it touches a page.
//! Every offset is bounded here, so the loader that follows cannot go out of range.
pub const PAGE: u64 = 4096;
pub const MAX_SEGMENTS: usize = 8;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
/// The linker's `-z stack-size=N` lands in this header's memory size.
const PT_GNU_STACK: u32 = 0x6474_e551;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const DT_NULL: u64 = 0;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_RELAENT: u64 = 9;
const R_X86_64_RELATIVE: u64 = 8;
const RELA_BYTES: u64 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment { pub offset: u64, pub file: u64, pub window: u64, pub memory: u64, pub writable: bool, pub executable: bool }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Image { pub entry: u64, pub segments: [Segment; MAX_SEGMENTS], pub count: usize, pub bias: u64, pub rela: u64, pub rela_count: u64, pub stack: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject { Header, Layout, Permissions, Span, Entry, Relocation }

fn u16_at(b: &[u8], at: u64) -> Option<u16> { let i = usize::try_from(at).ok()?; Some(u16::from_le_bytes(b.get(i..i.checked_add(2)?)?.try_into().ok()?)) }
fn u32_at(b: &[u8], at: u64) -> Option<u32> { let i = usize::try_from(at).ok()?; Some(u32::from_le_bytes(b.get(i..i.checked_add(4)?)?.try_into().ok()?)) }
fn u64_at(b: &[u8], at: u64) -> Option<u64> { let i = usize::try_from(at).ok()?; Some(u64::from_le_bytes(b.get(i..i.checked_add(8)?)?.try_into().ok()?)) }

/// Accepts only a little-endian x86-64 position-independent ELF whose segments are
/// page-separate, never writable and executable, and fit `window` bytes.
pub fn parse(elf: &[u8], window: u64) -> Result<Image, Reject> {
    use Reject::*;
    if elf.len() < 64 || elf[..4] != [0x7f, b'E', b'L', b'F'] || elf[4] != 2 || elf[5] != 1 { return Err(Header); }
    if u16_at(elf, 16) != Some(3) || u16_at(elf, 18) != Some(62) { return Err(Header); }
    let entry = u64_at(elf, 24).ok_or(Header)?;
    let (phoff, phentsize, phnum) = (u64_at(elf, 32).ok_or(Header)?, u16_at(elf, 54).ok_or(Header)? as u64, u16_at(elf, 56).ok_or(Header)? as u64);
    if phentsize < 56 || phnum == 0 || phnum > 64 { return Err(Header); }
    let mut image = Image { entry: 0, segments: [Segment { offset: 0, file: 0, window: 0, memory: 0, writable: false, executable: false }; MAX_SEGMENTS], count: 0, bias: u64::MAX, rela: 0, rela_count: 0, stack: 0 };
    let mut dynamic = None;
    let header = |i: u64| phoff.checked_add(i.checked_mul(phentsize)?);
    for i in 0..phnum {
        let at = header(i).ok_or(Header)?;
        let kind = u32_at(elf, at).ok_or(Header)?;
        if kind == PT_GNU_STACK { image.stack = u64_at(elf, at + 40).ok_or(Header)?; }
        if kind == PT_DYNAMIC {
            dynamic = Some((u64_at(elf, at + 8).ok_or(Header)?, u64_at(elf, at + 32).ok_or(Header)?));
        }
        if kind == PT_LOAD && u64_at(elf, at + 40).ok_or(Header)? != 0 {
            image.bias = image.bias.min(u64_at(elf, at + 16).ok_or(Header)?);
        }
    }
    if image.bias == u64::MAX || image.bias % PAGE != 0 { return Err(Layout); }
    let mut end = 0u64;
    for i in 0..phnum {
        let at = header(i).ok_or(Header)?;
        if u32_at(elf, at) != Some(PT_LOAD) { continue; }
        let flags = u32_at(elf, at + 4).ok_or(Header)?;
        let (offset, vaddr, file, memory) = (u64_at(elf, at + 8).ok_or(Header)?, u64_at(elf, at + 16).ok_or(Header)?, u64_at(elf, at + 32).ok_or(Header)?, u64_at(elf, at + 40).ok_or(Header)?);
        if memory == 0 { continue; }
        if flags & PF_W != 0 && flags & PF_X != 0 { return Err(Permissions); }
        if file > memory || offset.checked_add(file).ok_or(Layout)? > elf.len() as u64 { return Err(Layout); }
        let start = vaddr - image.bias;
        let stop = start.checked_add(memory).ok_or(Span)?;
        if stop > window { return Err(Span); }
        // Two segments in one page would need one set of permissions for both.
        if start % PAGE != 0 && start / PAGE < end.div_ceil(PAGE) { return Err(Permissions); }
        if start < end { return Err(Layout); }
        if image.count == MAX_SEGMENTS { return Err(Layout); }
        image.segments[image.count] = Segment { offset, file, window: start, memory, writable: flags & PF_W != 0, executable: flags & PF_X != 0 };
        image.count += 1;
        end = stop;
    }
    if image.count == 0 { return Err(Layout); }
    image.entry = entry.checked_sub(image.bias).ok_or(Entry)?;
    if !image.segments[..image.count].iter().any(|s| s.executable && image.entry >= s.window && image.entry < s.window + s.memory) { return Err(Entry); }
    if let Some((offset, size)) = dynamic {
        if offset.checked_add(size).ok_or(Relocation)? > elf.len() as u64 { return Err(Relocation); }
        let (mut rela, mut rela_size, mut rela_entry, mut seen_null) = (0, 0, RELA_BYTES, false);
        let mut at = offset;
        while at + 16 <= offset + size {
            let (tag, value) = (u64_at(elf, at).ok_or(Relocation)?, u64_at(elf, at + 8).ok_or(Relocation)?);
            match tag { DT_NULL => { seen_null = true; break } DT_RELA => rela = value, DT_RELASZ => rela_size = value, DT_RELAENT => rela_entry = value, _ => {} }
            at += 16;
        }
        if !seen_null || rela_entry != RELA_BYTES || rela_size % RELA_BYTES != 0 { return Err(Relocation); }
        if rela_size != 0 {
            let offset = file_offset(&image, rela.checked_sub(image.bias).ok_or(Relocation)?, rela_size).ok_or(Relocation)?;
            for i in 0..rela_size / RELA_BYTES {
                let entry = offset + i * RELA_BYTES;
                let (target, info) = (u64_at(elf, entry).ok_or(Relocation)?, u64_at(elf, entry + 8).ok_or(Relocation)?);
                if info & 0xffff_ffff != R_X86_64_RELATIVE { return Err(Relocation); }
                let target = target.checked_sub(image.bias).ok_or(Relocation)?;
                if !image.segments[..image.count].iter().any(|s| target >= s.window && target.checked_add(8).is_some_and(|e| e <= s.window + s.memory)) { return Err(Relocation); }
                let addend = u64_at(elf, entry + 16).ok_or(Relocation)?;
                if addend.checked_sub(image.bias).ok_or(Relocation)? > window { return Err(Relocation); }
            }
            image.rela = offset;
            image.rela_count = rela_size / RELA_BYTES;
        }
    }
    Ok(image)
}
/// File position of `size` bytes at window offset `at`, if the file holds all of them.
fn file_offset(image: &Image, at: u64, size: u64) -> Option<u64> {
    image.segments[..image.count].iter().find(|s| at >= s.window && at.checked_add(size).is_some_and(|e| e <= s.window + s.file)).map(|s| s.offset + (at - s.window))
}
/// Window offset of the 8-byte word to patch, and its new value, for relocation `index`.
pub fn relocation(elf: &[u8], image: &Image, index: u64, base: u64) -> Option<(u64, u64)> {
    let at = image.rela + index * RELA_BYTES;
    Some((u64_at(elf, at)? - image.bias, base + (u64_at(elf, at + 16)? - image.bias)))
}
