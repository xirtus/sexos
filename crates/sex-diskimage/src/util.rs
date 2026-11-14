// SPDX-License-Identifier: GPL-3.0-only
//! Byte helpers shared by the format readers.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::{ByteSource, Entry, EntryKind, Error, IoError, Node, Result};

/// Fill `buf` from `offset`, failing if the source ends first.
pub fn read_exact<S: ByteSource + ?Sized>(src: &S, offset: u64, buf: &mut [u8]) -> Result<()> {
    let end = offset.checked_add(buf.len() as u64).ok_or(Error::Corrupt("offset out of range"))?;
    if end > src.len() {
        return Err(Error::Corrupt("structure past the end of the image"));
    }
    let mut done = 0;
    while done < buf.len() {
        let n = src.read_at(offset + done as u64, &mut buf[done..])?;
        if n == 0 {
            return Err(Error::Corrupt("image ends early"));
        }
        done += n;
    }
    Ok(())
}

pub fn read_vec<S: ByteSource + ?Sized>(src: &S, offset: u64, len: usize) -> Result<Vec<u8>> {
    let mut v = vec![0u8; len];
    read_exact(src, offset, &mut v)?;
    Ok(v)
}

/// Read what is there at `offset`, up to `len` bytes, zero-filling past the end.
pub fn read_padded<S: ByteSource + ?Sized>(src: &S, offset: u64, len: usize) -> Result<Vec<u8>> {
    let mut v = vec![0u8; len];
    let avail = src.len().saturating_sub(offset).min(len as u64) as usize;
    if avail > 0 {
        read_exact(src, offset, &mut v[..avail])?;
    }
    Ok(v)
}

pub fn io_err(e: Error) -> IoError {
    match e {
        Error::Io(e) => e,
        other => IoError(other.message()),
    }
}

macro_rules! num {
    ($name:ident, $t:ty, $n:expr, $conv:ident) => {
        #[inline]
        pub fn $name(b: &[u8], at: usize) -> $t {
            match b.get(at..at + $n) {
                Some(s) => <$t>::$conv(s.try_into().unwrap()),
                None => 0,
            }
        }
    };
}
num!(le16, u16, 2, from_le_bytes);
num!(le32, u32, 4, from_le_bytes);
num!(le64, u64, 8, from_le_bytes);
num!(be32, u32, 4, from_be_bytes);
num!(be64, u64, 8, from_be_bytes);

/// Days from 1970-01-01 to the given civil date (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Seconds since the epoch for a local time with `tz_minutes` east of UTC.
pub fn unix_time(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32, tz_minutes: i64) -> Option<i64> {
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86400 + (h * 3600 + mi * 60 + s) as i64 - tz_minutes * 60)
}

/// UTF-16 big-endian bytes to a string (unpaired surrogates become U+FFFD).
pub fn utf16be(b: &[u8]) -> String {
    let units = b.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]]));
    char::decode_utf16(units).map(|r| r.unwrap_or('\u{FFFD}')).collect()
}

pub fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

pub fn lookup(path: &str, root: Node, mut list: impl FnMut(&Node) -> Result<Vec<Entry>>) -> Result<Entry> {
    let mut cur = Entry {
        name: String::new(),
        kind: EntryKind::Dir,
        size: 0,
        mtime: None,
        mode: None,
        hidden: false,
        synthetic: false,
        node: root,
    };
    for part in path.split('/').filter(|p| !p.is_empty() && *p != ".") {
        if !cur.is_dir() {
            return Err(Error::NotADirectory);
        }
        let items = list(&cur.node)?;
        let found = match items.iter().position(|e| e.name == part) {
            Some(i) => i,
            None => items.iter().position(|e| e.name.eq_ignore_ascii_case(part)).ok_or(Error::NotFound)?,
        };
        cur = items.into_iter().nth(found).unwrap();
    }
    Ok(cur)
}

pub const MAX_DEPTH: usize = 64;
pub const MAX_ENTRIES: usize = 1_000_000;

pub fn walk(root: Node, mut list: impl FnMut(&Node) -> Result<Vec<Entry>>) -> Result<Vec<(String, Entry)>> {
    let mut out: Vec<(String, Entry)> = Vec::new();
    // (path prefix, node, depth)
    let mut stack: Vec<(String, Node, usize)> = vec![(String::new(), root, 0)];
    while let Some((prefix, node, depth)) = stack.pop() {
        let items = list(&node)?;
        let mut subdirs = Vec::new();
        for e in items {
            if out.len() >= MAX_ENTRIES {
                return Err(Error::Unsupported("too many files in the image"));
            }
            let mut path = prefix.clone();
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(&e.name);
            if e.is_dir() && depth + 1 < MAX_DEPTH {
                subdirs.push((path.clone(), e.node.clone(), depth + 1));
            }
            out.push((path, e));
        }
        // Visit in name order: push reversed so the first pops first.
        stack.extend(subdirs.into_iter().rev());
    }
    Ok(out)
}

pub fn read_to_vec(len: u64, limit: u64, mut read: impl FnMut(u64, &mut [u8]) -> Result<usize>) -> Result<Vec<u8>> {
    if len > limit || len > usize::MAX as u64 {
        return Err(Error::Unsupported("file too large to load whole"));
    }
    let mut v = vec![0u8; len as usize];
    let mut done = 0;
    while done < v.len() {
        let n = read(done as u64, &mut v[done..])?;
        if n == 0 {
            return Err(Error::Corrupt("file ends early"));
        }
        done += n;
    }
    Ok(v)
}

/// Read from a file laid out as (byte offset, length) runs. A run whose offset
/// is `u64::MAX` reads as zeros (sparse).
pub fn read_runs<S: ByteSource + ?Sized>(src: &S, runs: &[(u64, u64)], offset: u64, buf: &mut [u8]) -> Result<usize> {
    let mut base = 0u64;
    for &(at, len) in runs {
        if offset < base + len {
            let within = offset - base;
            let n = ((len - within) as usize).min(buf.len());
            if at == u64::MAX {
                buf[..n].fill(0);
            } else {
                read_exact(src, at + within, &mut buf[..n])?;
            }
            return Ok(n);
        }
        base += len;
    }
    Ok(0)
}
