//! tar reading (v7, ustar, GNU long names, pax) and header writing.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::io::{read_exact_at, ByteSource, Input};
use crate::{Counted, Entry, EntryKind, Error, Loc, Result};

const BLOCK: u64 = 512;
/// Largest GNU long name / pax header we accept.
const MAX_META: u64 = 1 << 20;

fn checksum_ok(h: &[u8]) -> bool {
    let stored = match parse_octal(&h[148..156]) {
        Some(v) => v,
        None => return false,
    };
    let mut unsigned: u64 = 0;
    let mut signed: i64 = 0;
    for (i, &b) in h[..512].iter().enumerate() {
        let b = if (148..156).contains(&i) { b' ' } else { b };
        unsigned += b as u64;
        signed += b as i8 as i64;
    }
    stored == unsigned || stored as i64 == signed
}

/// A plausible tar header block.
pub(crate) fn is_header(h: &[u8]) -> bool {
    h.len() >= 512 && h[..512].iter().any(|&b| b != 0) && checksum_ok(h)
}

/// An archive consisting only of end-of-archive zero blocks.
pub(crate) fn is_empty_tar<S: ByteSource + ?Sized>(src: &S) -> Result<bool> {
    let len = src.len();
    if len < 1024 || len % 512 != 0 || len > 64 * 1024 {
        return Ok(false);
    }
    let mut b = vec![0u8; len as usize];
    read_exact_at(src, 0, &mut b)?;
    Ok(b.iter().all(|&x| x == 0))
}

fn parse_octal(f: &[u8]) -> Option<u64> {
    let mut v: u64 = 0;
    let mut seen = false;
    for &b in f {
        match b {
            b'0'..=b'7' => {
                v = v.checked_mul(8)?.checked_add((b - b'0') as u64)?;
                seen = true;
            }
            b' ' if !seen => {}
            b' ' | 0 => break,
            _ => return None,
        }
    }
    Some(v)
}

/// Octal, or GNU base-256 when the high bit of the first byte is set.
fn parse_num(f: &[u8]) -> Option<u64> {
    if f[0] & 0x80 != 0 {
        if f[0] == 0xff {
            return None; // negative
        }
        let mut v: u64 = (f[0] & 0x7f) as u64;
        for &b in &f[1..] {
            v = v.checked_mul(256)?.checked_add(b as u64)?;
        }
        return Some(v);
    }
    parse_octal(f)
}

fn cstr(f: &[u8]) -> &[u8] {
    let end = f.iter().position(|&b| b == 0).unwrap_or(f.len());
    &f[..end]
}

fn text(b: &[u8]) -> String {
    match core::str::from_utf8(b) {
        Ok(s) => String::from(s),
        // Legacy archives: treat as Latin-1 so every byte stays visible.
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}

#[derive(Default)]
struct Pax {
    path: Option<String>,
    linkpath: Option<String>,
    size: Option<u64>,
    mtime: Option<i64>,
}

fn parse_pax(data: &[u8], pax: &mut Pax) -> Result<()> {
    let mut rest = data;
    while !rest.is_empty() {
        if rest.iter().all(|&b| b == 0) {
            break;
        }
        let sp = rest.iter().position(|&b| b == b' ').ok_or(Error::Corrupt("bad pax record"))?;
        let len: usize = core::str::from_utf8(&rest[..sp])
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or(Error::Corrupt("bad pax record length"))?;
        if len <= sp + 1 || len > rest.len() {
            return Err(Error::Corrupt("bad pax record length"));
        }
        let rec = &rest[sp + 1..len];
        let rec = rec.strip_suffix(b"\n").unwrap_or(rec);
        if let Some(eq) = rec.iter().position(|&b| b == b'=') {
            let (k, v) = (&rec[..eq], &rec[eq + 1..]);
            match k {
                b"path" => pax.path = Some(text(v)),
                b"linkpath" => pax.linkpath = Some(text(v)),
                b"size" => {
                    pax.size = core::str::from_utf8(v).ok().and_then(|s| s.parse().ok());
                    if pax.size.is_none() {
                        return Err(Error::Corrupt("bad pax size"));
                    }
                }
                b"mtime" => {
                    pax.mtime = core::str::from_utf8(v)
                        .ok()
                        .and_then(|s| s.split('.').next())
                        .and_then(|s| s.parse().ok());
                }
                _ => {}
            }
        }
        rest = &rest[len..];
    }
    Ok(())
}

fn read_meta(input: &mut dyn Input, size: u64) -> Result<Vec<u8>> {
    if size > MAX_META {
        return Err(Error::TooLarge);
    }
    let mut v = vec![0u8; size as usize];
    input.read_exact(&mut v)?;
    input.skip(pad(size))?;
    Ok(v)
}

fn pad(size: u64) -> u64 {
    (BLOCK - size % BLOCK) % BLOCK
}

/// Read all headers. Data offsets are positions in `input`'s stream.
pub(crate) fn scan(input: &mut dyn Input) -> Result<Vec<Entry>> {
    let mut r = Counted { inner: input, pos: 0 };
    let mut entries = Vec::new();
    let mut long_name: Option<String> = None;
    let mut long_link: Option<String> = None;
    let mut pax = Pax::default();
    let mut h = [0u8; 512];
    loop {
        let n = r.read_full(&mut h)?;
        if n == 0 {
            break; // missing end-of-archive blocks: tolerate
        }
        if n < 512 {
            return Err(Error::Corrupt("tar header is truncated"));
        }
        if h.iter().all(|&b| b == 0) {
            break;
        }
        if !checksum_ok(&h) {
            return Err(Error::Corrupt("tar header checksum"));
        }
        let typeflag = h[156];
        let hsize = parse_num(&h[124..136]).ok_or(Error::Corrupt("tar size field"))?;
        match typeflag {
            b'L' => {
                long_name = Some(text(cstr(&read_meta(&mut r, hsize)?)));
                continue;
            }
            b'K' => {
                long_link = Some(text(cstr(&read_meta(&mut r, hsize)?)));
                continue;
            }
            b'x' | b'X' => {
                let data = read_meta(&mut r, hsize)?;
                parse_pax(&data, &mut pax)?;
                continue;
            }
            b'g' => {
                // Global pax header: nothing in it changes our listing.
                r.skip(hsize + pad(hsize))?;
                continue;
            }
            _ => {}
        }
        let ustar = &h[257..262] == b"ustar";
        let name = if let Some(p) = pax.path.take() {
            p
        } else if let Some(p) = long_name.take() {
            p
        } else {
            let base = cstr(&h[0..100]);
            let prefix = if ustar && &h[257..263] == b"ustar\0" { cstr(&h[345..500]) } else { &[][..] };
            if prefix.is_empty() {
                text(base)
            } else {
                format!("{}/{}", text(prefix), text(base))
            }
        };
        let link = pax.linkpath.take().or(long_link.take()).unwrap_or_else(|| text(cstr(&h[157..257])));
        let size = pax.size.take().unwrap_or(hsize);
        let mtime = pax.mtime.take().or_else(|| parse_num(&h[136..148]).map(|v| v as i64));
        let mode = parse_num(&h[100..108]).map(|m| (m & 0o7777) as u32);
        let mut kind = match typeflag {
            b'0' | 0 | b'7' => EntryKind::File,
            b'5' => EntryKind::Dir,
            b'2' => EntryKind::Symlink,
            b'1' => EntryKind::Hardlink,
            _ => EntryKind::Other,
        };
        if kind == EntryKind::File && name.ends_with('/') {
            kind = EntryKind::Dir; // old tars mark folders by a slash
        }
        // Links and folders carry no data even if size is set (except
        // hardlinks in some pax writers; skip whatever is declared).
        let data_size = match kind {
            EntryKind::Dir | EntryKind::Symlink | EntryKind::Hardlink => {
                if typeflag == b'1' || typeflag == b'2' {
                    0
                } else {
                    size
                }
            }
            _ => size,
        };
        let offset = r.pos;
        let mut e = Entry::new(name, kind, if matches!(kind, EntryKind::File) { size } else { 0 }, Loc::Tar { offset });
        e.mtime = mtime;
        e.mode = mode;
        e.compressed_size = None;
        if matches!(kind, EntryKind::Symlink | EntryKind::Hardlink) {
            e.link_target = Some(link);
        }
        if !(e.path.is_empty() && kind == EntryKind::Dir) {
            entries.push(e);
        }
        r.skip(data_size + pad(data_size))?;
    }
    Ok(entries)
}

// ---------------------------------------------------------------- writing

fn put_octal(f: &mut [u8], v: u64) -> bool {
    // width-1 digits then NUL.
    let digits = f.len() - 1;
    if digits < 22 && v >= 1u64 << (3 * digits) {
        return false;
    }
    let mut x = v;
    for i in (0..digits).rev() {
        f[i] = b'0' + (x & 7) as u8;
        x >>= 3;
    }
    f[digits] = 0;
    true
}

fn put_str(f: &mut [u8], s: &[u8]) {
    f[..s.len()].copy_from_slice(s);
}

/// Build a header block. `name` must fit (<=100 bytes, or split by caller).
fn header(name: &[u8], prefix: &[u8], typeflag: u8, size: u64, mtime: u64, mode: u32) -> [u8; 512] {
    let mut h = [0u8; 512];
    put_str(&mut h[0..100], name);
    put_octal(&mut h[100..108], mode as u64);
    put_octal(&mut h[108..116], 0);
    put_octal(&mut h[116..124], 0);
    if !put_octal(&mut h[124..136], size) {
        // Real size goes in a pax record; keep a placeholder.
        put_octal(&mut h[124..136], 0);
    }
    put_octal(&mut h[136..148], mtime.min(0o77777777777));
    h[156] = typeflag;
    put_str(&mut h[257..263], b"ustar\0");
    put_str(&mut h[263..265], b"00");
    put_str(&mut h[345..500], prefix);
    h[148..156].copy_from_slice(b"        ");
    let sum: u64 = h.iter().map(|&b| b as u64).sum();
    let mut c = [0u8; 8];
    put_octal(&mut c[..7], sum);
    c[7] = b' ';
    h[148..156].copy_from_slice(&c);
    h
}

fn pax_record(out: &mut Vec<u8>, key: &str, value: &[u8]) {
    let base = key.len() + value.len() + 3; // ' ', '=', '\n'
    let mut len = base + 1;
    loop {
        let digits = format!("{len}").len();
        if base + digits == len {
            break;
        }
        len = base + digits;
    }
    out.extend_from_slice(format!("{len} {key}=").as_bytes());
    out.extend_from_slice(value);
    out.push(b'\n');
}

/// Split a path into ustar (prefix, name) if possible.
fn split_ustar(p: &[u8]) -> Option<(&[u8], &[u8])> {
    if p.len() <= 100 {
        return Some((&[], p));
    }
    for (i, &b) in p.iter().enumerate() {
        if b == b'/' && i <= 155 && p.len() - i - 1 <= 100 && p.len() - i > 1 {
            return Some((&p[..i], &p[i + 1..]));
        }
    }
    None
}

/// Header block(s) for one member (pax extension when needed).
pub(crate) fn member_headers(path: &str, kind: EntryKind, size: u64, mtime: i64, mode: u32) -> Vec<u8> {
    let mut stored = String::from(path);
    if kind == EntryKind::Dir {
        stored.push('/');
    }
    let p = stored.as_bytes();
    let mtime = mtime.max(0) as u64;
    let typeflag = if kind == EntryKind::Dir { b'5' } else { b'0' };
    let mut out = Vec::new();
    let mut pax = Vec::new();
    let split = split_ustar(p);
    if split.is_none() {
        pax_record(&mut pax, "path", p);
    }
    if size >= 0o77777777777 {
        pax_record(&mut pax, "size", format!("{size}").as_bytes());
    }
    if !pax.is_empty() {
        let n = (pax.len() as u64).to_le(); // length only
        out.extend_from_slice(&header(b"././@PaxHeader", &[], b'x', n, mtime, 0o644));
        out.extend_from_slice(&pax);
        out.resize(out.len() + pad(pax.len() as u64) as usize, 0);
    }
    let (prefix, name) = split.unwrap_or((&[], &p[p.len().saturating_sub(100)..]));
    out.extend_from_slice(&header(name, prefix, typeflag, size, mtime, mode));
    out
}

pub(crate) fn padding(size: u64) -> usize {
    pad(size) as usize
}
