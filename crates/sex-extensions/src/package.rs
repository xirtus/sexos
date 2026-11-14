// SPDX-License-Identifier: GPL-3.0-only
//! Extension packages: Chrome's `.crx` (versions 2 and 3, a header before a ZIP) and
//! plain `.zip`, read into (path, bytes). Signatures are not checked yet: the person
//! chooses what to install, and each extension runs confined to what it asks for.

use super::{clean_path, Error};
use alloc::string::String;
use alloc::vec::Vec;

/// Largest unpacked extension.
pub const MAX_TOTAL: usize = 64 << 20;
/// Most files in one.
pub const MAX_FILES: usize = 8192;

fn u16le(b: &[u8], at: usize) -> Option<usize> { Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?) as usize) }
fn u32le(b: &[u8], at: usize) -> Option<usize> { Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?) as usize) }

/// The ZIP inside a package.
pub fn zip_of(bytes: &[u8]) -> Result<&[u8], Error> {
    if !bytes.starts_with(b"Cr24") { return Ok(bytes); }
    let bad = Error::Package("damaged .crx header");
    let start = match u32le(bytes, 4).ok_or(bad.clone())? {
        3 => 12 + u32le(bytes, 8).ok_or(bad.clone())?,
        2 => 16 + u32le(bytes, 8).ok_or(bad.clone())? + u32le(bytes, 12).ok_or(bad.clone())?,
        _ => return Err(Error::Package("unknown .crx version")),
    };
    bytes.get(start..).ok_or(bad)
}

/// Every file of a package, folders left out. A folder that wraps everything (as
/// GitHub release ZIPs often have) is taken off when it holds `manifest.json`.
pub fn unpack(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>, Error> {
    let zip = zip_of(bytes)?;
    let bad = Error::Package("damaged ZIP");
    // End of central directory: the last `PK\5\6` within the final 64 KiB + 22.
    let from = zip.len().saturating_sub(65_557);
    let eocd = (from..zip.len().saturating_sub(21)).rev().find(|&i| zip[i..].starts_with(b"PK\x05\x06")).ok_or(bad.clone())?;
    let count = u16le(zip, eocd + 10).ok_or(bad.clone())?;
    let mut at = u32le(zip, eocd + 16).ok_or(bad.clone())?;
    if count > MAX_FILES { return Err(Error::Package("too many files")); }
    let mut out = Vec::new();
    let mut total = 0usize;
    for _ in 0..count {
        if !zip.get(at..).is_some_and(|z| z.starts_with(b"PK\x01\x02")) { return Err(bad); }
        let method = u16le(zip, at + 10).ok_or(bad.clone())?;
        let packed = u32le(zip, at + 20).ok_or(bad.clone())?;
        let size = u32le(zip, at + 24).ok_or(bad.clone())?;
        let (n, e, c) = (u16le(zip, at + 28).ok_or(bad.clone())?, u16le(zip, at + 30).ok_or(bad.clone())?, u16le(zip, at + 32).ok_or(bad.clone())?);
        let local = u32le(zip, at + 42).ok_or(bad.clone())?;
        let name = core::str::from_utf8(zip.get(at + 46..at + 46 + n).ok_or(bad.clone())?).map_err(|_| bad.clone())?;
        at += 46 + n + e + c;
        if name.ends_with('/') { continue; }
        let Some(path) = clean_path(name) else { return Err(Error::Package("a file outside the package")) };
        total += size;
        if total > MAX_TOTAL { return Err(Error::Package("too big")); }
        if !zip.get(local..).is_some_and(|z| z.starts_with(b"PK\x03\x04")) { return Err(bad); }
        let data_at = local + 30 + u16le(zip, local + 26).ok_or(bad.clone())? + u16le(zip, local + 28).ok_or(bad.clone())?;
        let raw = zip.get(data_at..data_at + packed).ok_or(bad.clone())?;
        let data = match method {
            0 => Vec::from(raw),
            8 => {
                let opts = zune_inflate::DeflateOptions::default().set_limit(size.max(1)).set_size_hint(size);
                zune_inflate::DeflateDecoder::new_with_options(raw, opts).decode_deflate().map_err(|_| Error::Package("a file could not be unpacked"))?
            }
            _ => return Err(Error::Package("a file is packed in an unknown way")),
        };
        if data.len() != size { return Err(bad); }
        out.push((path, data));
    }
    // Take off a single top folder around manifest.json.
    if !out.iter().any(|(p, _)| p == "manifest.json") {
        if let Some(top) = out.iter().find(|(p, _)| p.ends_with("/manifest.json") && p.matches('/').count() == 1).map(|(p, _)| String::from(&p[..p.len() - "manifest.json".len()])) {
            out.retain(|(p, _)| p.starts_with(&top));
            for (p, _) in out.iter_mut() { *p = String::from(&p[top.len()..]); }
        }
    }
    if !out.iter().any(|(p, _)| p == "manifest.json") { return Err(Error::Package("no manifest.json")); }
    Ok(out)
}
