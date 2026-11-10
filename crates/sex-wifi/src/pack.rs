// SPDX-License-Identifier: GPL-3.0-only
//! The firmware pack: the Intel firmware files the boot image carries, as one
//! read-only blob the kernel hands to the network service without looking inside.
//! `scripts/make_wifi_firmware_pack.py` writes it.
//!
//! Layout (little endian): magic `SEXWIFW1`, u32 count, u32 reserved, then `count`
//! entries of 88 bytes {name[48] NUL padded, u32 offset, u32 length, sha256[32]},
//! then the files, each starting on a 4 KiB boundary. A file is only returned when
//! its SHA-256 matches, so a damaged pack fails by name rather than at the chip.
use sha2::{Digest, Sha256};

pub const MAGIC: &[u8; 8] = b"SEXWIFW1";
pub const HEADER: usize = 16;
pub const ENTRY: usize = 88;
pub const NAME: usize = 48;
pub const MAX_FILES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error { Magic, Truncated, TooMany, Missing, Damaged }

#[derive(Clone, Copy)]
pub struct Pack<'a> { bytes: &'a [u8], count: usize }

pub struct Entry<'a> { pub name: &'a [u8], pub data: &'a [u8], pub sha256: &'a [u8] }

impl<'a> Pack<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, Error> {
        if bytes.len() < HEADER || &bytes[..8] != MAGIC { return Err(Error::Magic); }
        let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        if count > MAX_FILES { return Err(Error::TooMany); }
        if bytes.len() < HEADER + count * ENTRY { return Err(Error::Truncated); }
        let pack = Pack { bytes, count };
        for i in 0..count { pack.entry(i).ok_or(Error::Truncated)?; }
        Ok(pack)
    }
    pub fn len(&self) -> usize { self.count }
    pub fn is_empty(&self) -> bool { self.count == 0 }

    pub fn entry(&self, i: usize) -> Option<Entry<'a>> {
        if i >= self.count { return None; }
        let e = &self.bytes[HEADER + i * ENTRY..HEADER + (i + 1) * ENTRY];
        let name_len = e[..NAME].iter().position(|b| *b == 0).unwrap_or(NAME);
        let offset = u32::from_le_bytes(e[48..52].try_into().unwrap()) as usize;
        let length = u32::from_le_bytes(e[52..56].try_into().unwrap()) as usize;
        let data = self.bytes.get(offset..offset.checked_add(length)?)?;
        Some(Entry { name: &e[..name_len], data, sha256: &e[56..88] })
    }

    /// The file called `name`, checked against its SHA-256.
    pub fn file(&self, name: &str) -> Result<&'a [u8], Error> {
        let e = (0..self.count).filter_map(|i| self.entry(i)).find(|e| e.name == name.as_bytes()).ok_or(Error::Missing)?;
        if Sha256::digest(e.data).as_slice() != e.sha256 { return Err(Error::Damaged); }
        Ok(e.data)
    }
}
