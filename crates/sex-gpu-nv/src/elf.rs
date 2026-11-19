// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Just enough ELF64 to find named sections in `gsp-<version>.bin`.
//! GSP-RM ships as an ELF whose `.fwimage` section is the RISC-V image and
//! whose `.fwsignature_<chip>` sections hold Booter's signatures for it.

use crate::{rd16, rd32, rd64, Error};

/// The bytes of section `name`, or an error naming what is wrong.
pub fn section<'a>(img: &'a [u8], name: &str) -> Result<&'a [u8], Error> {
    let bad = Error::BadFirmware;
    if img.get(0..4) != Some(b"\x7fELF".as_slice()) { return Err(bad("gsp: not an ELF file")); }
    if rd8c(img, 4) != 2 || rd8c(img, 5) != 1 { return Err(bad("gsp: not ELF64 little-endian")); }
    let shoff = rd64(img, 0x28).ok_or(bad("gsp: short header"))? as usize;
    let shentsize = rd16(img, 0x3a).ok_or(bad("gsp: short header"))? as usize;
    let shnum = rd16(img, 0x3c).ok_or(bad("gsp: short header"))? as usize;
    let shstrndx = rd16(img, 0x3e).ok_or(bad("gsp: short header"))? as usize;
    if shentsize < 0x40 || shstrndx >= shnum { return Err(bad("gsp: bad section table")); }
    let sh = |i: usize| shoff.checked_add(i.checked_mul(shentsize)?);
    let strtab = sh(shstrndx).and_then(|o| rd64(img, o + 0x18)).ok_or(bad("gsp: bad shstrtab"))? as usize;
    for i in 0..shnum {
        let o = sh(i).ok_or(bad("gsp: bad section table"))?;
        let name_off = rd32(img, o).ok_or(bad("gsp: bad section table"))? as usize;
        let off = rd64(img, o + 0x18).ok_or(bad("gsp: bad section table"))? as usize;
        let size = rd64(img, o + 0x20).ok_or(bad("gsp: bad section table"))? as usize;
        let start = strtab.checked_add(name_off).ok_or(bad("gsp: bad name"))?;
        let n = name.as_bytes();
        if img.get(start..start + n.len()) == Some(n) && img.get(start + n.len()) == Some(&0) {
            return img.get(off..off.checked_add(size).ok_or(bad("gsp: bad section"))?).ok_or(bad("gsp: section outside file"));
        }
    }
    Err(bad("gsp: section not found"))
}

fn rd8c(b: &[u8], o: usize) -> u8 { b.get(o).copied().unwrap_or(0) }
