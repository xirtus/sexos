// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The GSP firmware pack: NVIDIA's files as one boot module (`nvidia-gsp.pack`),
//! built by `scripts/make_nvidia_firmware_pack.py` from the pinned linux-firmware files.
//!
//! Layout (little endian): `SEXNVFW1`, file count (u32), 0 (u32), then one 88-byte
//! entry per file: name (44 bytes, NUL padded), 0 (u32), offset (u32), length (u32),
//! SHA-256 (32 bytes). Files are stored raw, each starting on a 4 KiB boundary.
//!
//! The pack's own hashes are checked against [`PINS`] here; the service hashes the
//! bytes too, so a pack that lies about its content is refused as well.

use crate::{rd32, Error};

pub const MAGIC: &[u8; 8] = b"SEXNVFW1";
pub const ENTRY: usize = 88;
const NAME: usize = 44;
pub const MAX_FILES: usize = 16;

/// The files GSP-RM 535.113.01 boots from, with their SHA-256 (the same pins as
/// `scripts/fetch_nvidia_gsp_firmware.sh`, linux-firmware commit 69e51bb).
pub const PINS: &[(&str, &str)] = &[
    (BOOTER_LOAD, "3ab46bf4c70d72e8fc89d98e6bdc9b83a61954a586629d26b8b5653a620237e0"),
    (BOOTER_UNLOAD, "09240c83595856a0fe70d2ab3ec7325455cd8adfc48ec7b47f6fced1e5431beb"),
    (BOOTLOADER, "076d2a0675131ad8a8de215c611ce4271289adcc03e2cf39995f9b29079e2136"),
    (GSP_RM, "e30231ec0d317021770e16c81ae2c0553f728d0344248827582f78132c9447d4"),
];
pub const BOOTER_LOAD: &str = "booter_load-535.113.01.bin";
pub const BOOTER_UNLOAD: &str = "booter_unload-535.113.01.bin";
pub const BOOTLOADER: &str = "bootloader-535.113.01.bin";
pub const GSP_RM: &str = "gsp-535.113.01.bin";

/// One file of the pack.
#[derive(Clone, Copy, Debug)]
pub struct File<'a> { pub name: &'a str, pub data: &'a [u8], pub sha256: [u8; 32] }

#[derive(Clone, Copy, Debug)]
pub struct Pack<'a> { bytes: &'a [u8], count: usize }

impl<'a> Pack<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Pack<'a>, Error> {
        if bytes.len() < 16 || &bytes[..8] != MAGIC { return Err(Error::BadFirmware("pack: no SEXNVFW1 header")); }
        let count = rd32(bytes, 8).unwrap_or(0) as usize;
        if count > MAX_FILES || 16 + count * ENTRY > bytes.len() { return Err(Error::BadFirmware("pack: bad file count")); }
        let p = Pack { bytes, count };
        for i in 0..count { p.entry(i)?; }
        Ok(p)
    }
    pub fn len(&self) -> usize { self.count }
    pub fn is_empty(&self) -> bool { self.count == 0 }

    pub fn entry(&self, i: usize) -> Result<File<'a>, Error> {
        let e = &self.bytes[16 + i * ENTRY..16 + (i + 1) * ENTRY];
        let end = e[..NAME].iter().position(|b| *b == 0).unwrap_or(NAME);
        let name = core::str::from_utf8(&e[..end]).map_err(|_| Error::BadFirmware("pack: name not UTF-8"))?;
        if rd32(e, NAME) != Some(0) { return Err(Error::BadFirmware("pack: compressed entry")); }
        let (offset, length) = (rd32(e, NAME + 4).unwrap_or(0) as usize, rd32(e, NAME + 8).unwrap_or(0) as usize);
        let data = offset.checked_add(length).and_then(|end| self.bytes.get(offset..end))
            .ok_or(Error::BadFirmware("pack: entry outside the pack"))?;
        let mut sha256 = [0u8; 32];
        sha256.copy_from_slice(&e[NAME + 12..NAME + 44]);
        Ok(File { name, data, sha256 })
    }

    pub fn file(&self, name: &str) -> Option<File<'a>> {
        (0..self.count).filter_map(|i| self.entry(i).ok()).find(|f| f.name == name)
    }

    /// A pinned file whose recorded hash is the pin. The caller still hashes `data`
    /// and compares it with `sha256`.
    pub fn pinned(&self, name: &str) -> Result<File<'a>, Error> {
        let pin = PINS.iter().find(|(n, _)| *n == name).map(|(_, h)| *h).ok_or(Error::BadFirmware("pack: file has no pin"))?;
        let f = self.file(name).ok_or(Error::BadFirmware("pack: pinned file missing"))?;
        if !hex_eq(pin, &f.sha256) { return Err(Error::BadFirmware("pack: file is not the pinned version")); }
        Ok(f)
    }
}

/// Is `hex` (64 lowercase hex digits) the same hash as `bytes`?
pub fn hex_eq(hex: &str, bytes: &[u8; 32]) -> bool {
    let h = hex.as_bytes();
    h.len() == 64 && bytes.iter().enumerate().all(|(i, b)| {
        let d = |c: u8| match c { b'0'..=b'9' => Some(c - b'0'), b'a'..=b'f' => Some(c - b'a' + 10), _ => None };
        matches!((d(h[2 * i]), d(h[2 * i + 1])), (Some(hi), Some(lo)) if hi << 4 | lo == *b)
    })
}
