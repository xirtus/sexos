// SPDX-License-Identifier: GPL-3.0-only
//! The firmware pack: the Intel firmware files the boot image carries, as one
//! read-only blob the kernel hands to the network service without looking inside.
//! `scripts/make_wifi_firmware_pack.py` writes it.
//!
//! Layout (little endian): magic `SEXWIFW1`, u32 count, u32 reserved, then `count`
//! entries of 88 bytes {name[48] NUL padded, u32 offset, u32 length, sha256[32]},
//! then the files, each starting on a 4 KiB boundary. A file is only returned when
//! its SHA-256 matches, so a damaged pack fails by name rather than at the chip.
//!
//! `SEXWIFW2` packs may store a file DEFLATE-compressed (raw, no zlib header): the
//! name shrinks to 44 bytes and bytes 44..48 hold its inflated length (0 = stored).
//! The SHA-256 is always of the inflated file. Compressing the firmware takes the pack
//! from 13.8 MB to about 5.5 MB, which keeps it inside the installed disk's 32 MiB
//! boot partition (sex_pdx::fs::BOOT_SECTORS) and out of boot RAM.
use sha2::{Digest, Sha256};

pub const MAGIC: &[u8; 8] = b"SEXWIFW2";
/// The first pack format: every file stored, names up to 48 bytes.
pub const MAGIC_V1: &[u8; 8] = b"SEXWIFW1";
/// Name bytes in a `SEXWIFW2` entry; bytes 44..48 are the inflated length.
pub const NAME_V2: usize = 44;
/// The DEFLATE state `Pack::open` inflates a compressed file with.
pub type Inflater = miniz_oxide::inflate::core::DecompressorOxide;
pub const HEADER: usize = 16;
pub const ENTRY: usize = 88;
pub const NAME: usize = 48;
pub const MAX_FILES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Magic, Truncated, TooMany, Missing, Damaged,
    /// Compressed: read it with `Pack::open` and room for its inflated bytes.
    Compressed,
    /// The scratch given to `Pack::open` is smaller than the inflated file.
    TooBig }

#[derive(Clone, Copy)]
pub struct Pack<'a> { bytes: &'a [u8], count: usize, v2: bool }

/// `inflated` is the file's length once inflated, 0 when `data` is the file itself.
pub struct Entry<'a> { pub name: &'a [u8], pub data: &'a [u8], pub sha256: &'a [u8], pub inflated: usize }

impl<'a> Pack<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER { return Err(Error::Magic); }
        let v2 = match &bytes[..8] { m if m == MAGIC => true, m if m == MAGIC_V1 => false, _ => return Err(Error::Magic) };
        let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        if count > MAX_FILES { return Err(Error::TooMany); }
        if bytes.len() < HEADER + count * ENTRY { return Err(Error::Truncated); }
        let pack = Pack { bytes, count, v2 };
        for i in 0..count { pack.entry(i).ok_or(Error::Truncated)?; }
        Ok(pack)
    }
    pub fn len(&self) -> usize { self.count }
    pub fn is_empty(&self) -> bool { self.count == 0 }

    pub fn entry(&self, i: usize) -> Option<Entry<'a>> {
        if i >= self.count { return None; }
        let e = &self.bytes[HEADER + i * ENTRY..HEADER + (i + 1) * ENTRY];
        let names = if self.v2 { NAME_V2 } else { NAME };
        let name_len = e[..names].iter().position(|b| *b == 0).unwrap_or(names);
        let inflated = if self.v2 { u32::from_le_bytes(e[44..48].try_into().unwrap()) as usize } else { 0 };
        let offset = u32::from_le_bytes(e[48..52].try_into().unwrap()) as usize;
        let length = u32::from_le_bytes(e[52..56].try_into().unwrap()) as usize;
        let data = self.bytes.get(offset..offset.checked_add(length)?)?;
        Some(Entry { name: &e[..name_len], data, sha256: &e[56..88], inflated })
    }

    fn find(&self, name: &str) -> Result<Entry<'a>, Error> {
        (0..self.count).filter_map(|i| self.entry(i)).find(|e| e.name == name.as_bytes()).ok_or(Error::Missing)
    }

    /// The stored file called `name`, checked against its SHA-256.
    pub fn file(&self, name: &str) -> Result<&'a [u8], Error> {
        let e = self.find(name)?;
        if e.inflated != 0 { return Err(Error::Compressed); }
        if Sha256::digest(e.data).as_slice() != e.sha256 { return Err(Error::Damaged); }
        Ok(e.data)
    }

    /// The file called `name`, stored or compressed: a compressed one is inflated into
    /// the front of `scratch` with the caller's `inflater` (10.5 KB, so this crate
    /// still never allocates). Either way it is checked against its SHA-256.
    pub fn open<'s>(&self, name: &str, scratch: &'s mut [u8], inflater: &mut Inflater) -> Result<&'s [u8], Error> where 'a: 's {
        let e = self.find(name)?;
        if e.inflated == 0 { return self.file(name); }
        let out = scratch.get_mut(..e.inflated).ok_or(Error::TooBig)?;
        inflater.init();
        let (status, _, written) = miniz_oxide::inflate::core::decompress(inflater, e.data, out, 0,
            miniz_oxide::inflate::core::inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF);
        if status != miniz_oxide::inflate::TINFLStatus::Done || written != e.inflated { return Err(Error::Damaged); }
        let out = &scratch[..e.inflated];
        if Sha256::digest(out).as_slice() != e.sha256 { return Err(Error::Damaged); }
        Ok(out)
    }
}
