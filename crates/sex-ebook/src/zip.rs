// SPDX-License-Identifier: GPL-3.0-only
//! Reading ZIP archives held in memory: the central directory, then each member stored
//! or deflated. Enough for EPUB and CBZ; no ZIP64, encryption or multi-disk archives.
//! Sizes come from the central directory and every read is bounded by them.
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No end-of-central-directory record: not a ZIP archive.
    NotZip,
    /// The directory or a member header points outside the archive.
    Damaged,
    /// A compression method other than stored or deflate, or an encrypted member.
    Unsupported,
    /// The member's data did not inflate to its stated size.
    Corrupt,
    /// The member is larger than the caller allows.
    TooLarge,
    /// No member by that name.
    Missing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    method: u16,
    flags: u16,
    compressed: usize,
    pub size: usize,
    local: usize,
}

pub struct Zip<'a> {
    data: &'a [u8],
    pub entries: Vec<Entry>,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> { b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]])) }
fn u32_at(b: &[u8], i: usize) -> Option<u32> { b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])) }

/// True when `data` starts like a ZIP archive.
pub fn is_zip(data: &[u8]) -> bool { data.starts_with(b"PK\x03\x04") || data.starts_with(b"PK\x05\x06") }

impl<'a> Zip<'a> {
    pub fn open(data: &'a [u8]) -> Result<Zip<'a>, Error> {
        // The end record is in the last 64 KiB + 22 bytes (the comment is at most 64 KiB).
        let lowest = data.len().saturating_sub(22 + 0xFFFF);
        let end = (lowest..=data.len().saturating_sub(22)).rev()
            .find(|&i| data[i..].starts_with(b"PK\x05\x06"))
            .ok_or(Error::NotZip)?;
        let count = u16_at(data, end + 10).ok_or(Error::Damaged)? as usize;
        let mut at = u32_at(data, end + 16).ok_or(Error::Damaged)? as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if u32_at(data, at) != Some(0x0201_4b50) { return Err(Error::Damaged); }
            let get16 = |o| u16_at(data, at + o).ok_or(Error::Damaged);
            let get32 = |o| u32_at(data, at + o).ok_or(Error::Damaged);
            let (flags, method) = (get16(8)?, get16(10)?);
            let (compressed, size) = (get32(20)? as usize, get32(24)? as usize);
            let (name_len, extra_len, comment_len) = (get16(28)? as usize, get16(30)? as usize, get16(32)? as usize);
            let local = get32(42)? as usize;
            let name = data.get(at + 46..at + 46 + name_len).ok_or(Error::Damaged)?;
            entries.push(Entry { name: String::from_utf8_lossy(name).into_owned(), method, flags, compressed, size, local });
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Zip { data, entries })
    }
    pub fn entry(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
            .or_else(|| self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name)))
    }
    /// The member called `name`, unpacked, if it is at most `max` bytes.
    pub fn read(&self, name: &str, max: usize) -> Result<Vec<u8>, Error> {
        let e = self.entry(name).ok_or(Error::Missing)?;
        self.read_entry(e, max)
    }
    pub fn read_entry(&self, e: &Entry, max: usize) -> Result<Vec<u8>, Error> {
        if e.size > max { return Err(Error::TooLarge); }
        if e.flags & 1 != 0 { return Err(Error::Unsupported); }
        let d = self.data;
        if u32_at(d, e.local) != Some(0x0403_4b50) { return Err(Error::Damaged); }
        let name_len = u16_at(d, e.local + 26).ok_or(Error::Damaged)? as usize;
        let extra_len = u16_at(d, e.local + 28).ok_or(Error::Damaged)? as usize;
        let start = e.local + 30 + name_len + extra_len;
        let raw = d.get(start..start.checked_add(e.compressed).ok_or(Error::Damaged)?).ok_or(Error::Damaged)?;
        match e.method {
            0 => Ok(raw[..e.size.min(raw.len())].to_vec()),
            8 => {
                let options = zune_inflate::DeflateOptions::default().set_limit(e.size.max(1)).set_size_hint(e.size);
                let out = zune_inflate::DeflateDecoder::new_with_options(raw, options).decode_deflate().map_err(|_| Error::Corrupt)?;
                if out.len() != e.size { return Err(Error::Corrupt); }
                Ok(out)
            }
            _ => Err(Error::Unsupported),
        }
    }
}
