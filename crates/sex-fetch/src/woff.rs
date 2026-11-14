// SPDX-License-Identifier: GPL-3.0-only
//! Web font files: WOFF 1.0 (each table zlib-compressed) back to the TrueType or
//! OpenType file it wraps. WOFF2 (Brotli and transformed tables) is not read; pages
//! that list another format as well get that one.

use alloc::vec::Vec;

/// Largest font accepted, unpacked.
pub const MAX_FONT: usize = 32 << 20;

/// What kind of font file `data` is, by its first bytes.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Kind { Sfnt, Woff, Woff2, Unknown }

pub fn kind(data: &[u8]) -> Kind {
    match data.get(..4) {
        Some(b"wOFF") => Kind::Woff,
        Some(b"wOF2") => Kind::Woff2,
        Some([0, 1, 0, 0]) | Some(b"OTTO") | Some(b"true") => Kind::Sfnt,
        _ => Kind::Unknown,
    }
}

fn be16(d: &[u8], at: usize) -> Option<u16> { Some(u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?)) }
fn be32(d: &[u8], at: usize) -> Option<u32> { Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?)) }

/// The font file a web font holds: TrueType/OpenType as is, WOFF unpacked.
pub fn to_sfnt(data: &[u8]) -> Option<Vec<u8>> {
    match kind(data) {
        Kind::Sfnt => Some(Vec::from(data)),
        Kind::Woff => unwoff(data),
        _ => None,
    }
}

fn unwoff(d: &[u8]) -> Option<Vec<u8>> {
    let flavor = be32(d, 4)?;
    let n = be16(d, 12)? as usize;
    let total = be32(d, 16)? as usize;
    if n == 0 || n > 512 || total > MAX_FONT { return None; }
    // The sfnt header: search fields from the table count.
    let mut pow = 1usize; let mut log = 0u16;
    while pow * 2 <= n { pow *= 2; log += 1; }
    let mut out = Vec::with_capacity(total.max(12 + 16 * n));
    out.extend_from_slice(&flavor.to_be_bytes());
    out.extend_from_slice(&(n as u16).to_be_bytes());
    out.extend_from_slice(&((pow * 16) as u16).to_be_bytes());
    out.extend_from_slice(&log.to_be_bytes());
    out.extend_from_slice(&((n * 16 - pow * 16) as u16).to_be_bytes());
    let dir = out.len();
    out.resize(dir + 16 * n, 0);
    for i in 0..n {
        let e = 44 + 20 * i;
        let (tag, off, comp, orig, sum) = (be32(d, e)?, be32(d, e + 4)? as usize, be32(d, e + 8)? as usize, be32(d, e + 12)? as usize, be32(d, e + 16)?);
        if orig > MAX_FONT || out.len() + orig > MAX_FONT { return None; }
        let src = d.get(off..off.checked_add(comp)?)?;
        let at = out.len();
        if comp < orig {
            let table = zune_inflate::DeflateDecoder::new_with_options(src, zune_inflate::DeflateOptions::default().set_limit(orig).set_size_hint(orig)).decode_zlib().ok()?;
            if table.len() != orig { return None; }
            out.extend_from_slice(&table);
        } else if comp == orig {
            out.extend_from_slice(src);
        } else { return None; }
        while out.len() % 4 != 0 { out.push(0); }
        let r = dir + 16 * i;
        out[r..r + 4].copy_from_slice(&tag.to_be_bytes());
        out[r + 4..r + 8].copy_from_slice(&sum.to_be_bytes());
        out[r + 8..r + 12].copy_from_slice(&(at as u32).to_be_bytes());
        out[r + 12..r + 16].copy_from_slice(&(orig as u32).to_be_bytes());
    }
    Some(out)
}
