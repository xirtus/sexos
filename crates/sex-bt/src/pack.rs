// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Firmware files handed to Cufflinks as one read-only blob, in the same layout as the
//! Wi-Fi pack (`crates/sex-wifi/src/pack.rs`, `scripts/make_wifi_firmware_pack.py`):
//! magic, u32 count, u32 reserved, then entries of {name[48], u32 offset, u32 length,
//! sha256[32]}, then the files. Either magic is accepted, so the Bluetooth files can
//! ride in the Wi-Fi pack or in a pack of their own (`SEXBTFW1`).
//! The SHA-256 is not checked here (no hash crate in this driver crate yet); the
//! controller's own signature check refuses a damaged .sfi.
//! In a `SEXWIFW2` pack names are 44 bytes and bytes 44..48 give an inflated length:
//! only stored files (length 0) are found here; the Bluetooth files are small and
//! the packer stores them as they are.

pub const MAGICS: [&[u8; 8]; 3] = [b"SEXBTFW1", b"SEXWIFW1", b"SEXWIFW2"];
const HEADER: usize = 16;
const ENTRY: usize = 88;
const NAME: usize = 48;

/// The file called `name`, if the pack holds it whole.
pub fn find<'a>(pack: &'a [u8], name: &str) -> Option<&'a [u8]> {
    if pack.len() < HEADER || !MAGICS.iter().any(|m| &pack[..8] == *m) { return None; }
    let count = u32::from_le_bytes(pack[8..12].try_into().ok()?) as usize;
    for i in 0..count.min(256) {
        let e = pack.get(HEADER + i * ENTRY..HEADER + (i + 1) * ENTRY)?;
        let v2 = &pack[..8] == b"SEXWIFW2";
        let names = if v2 { 44 } else { NAME };
        let n = &e[..names];
        let n = &n[..n.iter().position(|&c| c == 0).unwrap_or(names)];
        if n != name.as_bytes() { continue; }
        if v2 && e[44..48] != [0; 4] { return None; }
        let off = u32::from_le_bytes(e[48..52].try_into().ok()?) as usize;
        let len = u32::from_le_bytes(e[52..56].try_into().ok()?) as usize;
        return pack.get(off..off.checked_add(len)?);
    }
    None
}
