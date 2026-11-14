//! zip reading (central directory, zip64, UTF-8/CP437 names) and record
//! encoding for the writer.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::codec::{Inflate, LzmaDecoder, ZstdDecoder};
use crate::xz::XzDecoder;
use crate::io::{read_exact_at, ByteSource, Input, IoError, SrcReader, Take};
use crate::path::normalize;
use crate::{pump, Entry, EntryKind, Error, Loc, ProgressFn, Result};

fn u16le(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}
fn u64le(b: &[u8], o: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[o..o + 8]);
    u64::from_le_bytes(a)
}

const EOCD_SIG: u32 = 0x0605_4b50;
const Z64_LOC_SIG: u32 = 0x0706_4b50;
const Z64_EOCD_SIG: u32 = 0x0606_4b50;
const CDH_SIG: u32 = 0x0201_4b50;
const LFH_SIG: u32 = 0x0403_4b50;

/// CP437 code points for bytes 0x80..=0xFF.
const CP437_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', 'É', 'æ', 'Æ', 'ô', 'ö', 'ò',
    'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', 'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½',
    '¼', '¡', '«', '»', '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐', '└', '┴',
    '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧', '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫',
    '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀', 'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ',
    'ε', '∩', '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{a0}',
];

fn decode_name(b: &[u8], utf8: bool) -> String {
    if utf8 || b.is_ascii() {
        return String::from_utf8_lossy(b).into_owned();
    }
    // Many tools store UTF-8 without setting the flag; prefer it when valid.
    if let Ok(s) = core::str::from_utf8(b) {
        return String::from(s);
    }
    b.iter().map(|&c| if c < 0x80 { c as char } else { CP437_HIGH[(c - 0x80) as usize] }).collect()
}

/// DOS date/time (local time, treated as UTC) to Unix seconds.
fn dos_to_unix(time: u16, date: u16) -> Option<i64> {
    let year = 1980 + (date >> 9) as i64;
    let month = ((date >> 5) & 15) as i64;
    let day = (date & 31) as i64;
    if !(1..=12).contains(&month) || day == 0 {
        return None;
    }
    let (h, m, s) = ((time >> 11) as i64, ((time >> 5) & 63) as i64, ((time & 31) * 2) as i64);
    Some(days_from_civil(year, month, day) * 86400 + h * 3600 + m * 60 + s)
}

pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Unix seconds to DOS (time, date); clamps to 1980..=2107.
pub(crate) fn unix_to_dos(t: i64) -> (u16, u16) {
    let t = t.clamp(315_532_800, 4_354_819_199);
    let days = t.div_euclid(86400);
    let secs = t.rem_euclid(86400);
    // civil_from_days
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    let time = ((secs / 3600) << 11) | (((secs / 60) % 60) << 5) | ((secs % 60) / 2);
    let date = ((y - 1980) << 9) | (m << 5) | d;
    (time as u16, date as u16)
}

struct Eocd {
    count: u64,
    cd_size: u64,
    cd_offset: u64,
    /// Where the central directory really starts (prefix-adjusted).
    cd_start: u64,
}

fn find_eocd<S: ByteSource + ?Sized>(src: &S) -> Result<Eocd> {
    let len = src.len();
    if len < 22 {
        return Err(Error::UnknownFormat);
    }
    let tail_len = len.min(22 + 65535 + 20);
    let tail_start = len - tail_len;
    let mut tail = vec![0u8; tail_len as usize];
    read_exact_at(src, tail_start, &mut tail)?;
    let mut pos = None;
    for i in (0..=tail.len() - 22).rev() {
        if u32le(&tail, i) == EOCD_SIG {
            let clen = u16le(&tail, i + 20) as usize;
            if i + 22 + clen <= tail.len() {
                pos = Some(i);
                break;
            }
        }
    }
    let i = pos.ok_or(Error::UnknownFormat)?;
    let eocd_abs = tail_start + i as u64;
    let e = &tail[i..];
    let mut count = u16le(e, 10) as u64;
    let mut cd_size = u32le(e, 12) as u64;
    let mut cd_offset = u32le(e, 16) as u64;
    let mut cd_end = eocd_abs;
    // zip64 locator immediately precedes the EOCD.
    if eocd_abs >= 20 {
        let mut loc = [0u8; 20];
        read_exact_at(src, eocd_abs - 20, &mut loc)?;
        if u32le(&loc, 0) == Z64_LOC_SIG {
            let z64_off = u64le(&loc, 8);
            let mut rec = [0u8; 56];
            // The record may also be displaced by a prefix; try both.
            let guess = eocd_abs - 20 - 56;
            let at = if z64_off + 56 <= len && {
                read_exact_at(src, z64_off, &mut rec)?;
                u32le(&rec, 0) == Z64_EOCD_SIG
            } {
                z64_off
            } else {
                read_exact_at(src, guess, &mut rec)?;
                if u32le(&rec, 0) != Z64_EOCD_SIG {
                    return Err(Error::Corrupt("zip64 end record missing"));
                }
                guess
            };
            count = u64le(&rec, 32);
            cd_size = u64le(&rec, 40);
            cd_offset = u64le(&rec, 48);
            cd_end = at;
        }
    }
    let cd_start = cd_end.checked_sub(cd_size).ok_or(Error::Corrupt("central directory size"))?;
    Ok(Eocd { count, cd_size, cd_offset, cd_start })
}

pub(crate) fn open<S: ByteSource + ?Sized>(src: &S) -> Result<Vec<Entry>> {
    let eocd = find_eocd(src)?;
    // Bytes prepended to the archive (self-extractors) shift all offsets.
    let shift = eocd.cd_start as i128 - eocd.cd_offset as i128;
    let mut r = SrcReader::new(src, eocd.cd_start, eocd.cd_start + eocd.cd_size);
    let mut entries = Vec::with_capacity(eocd.count.min(1 << 16) as usize);
    let mut h = [0u8; 46];
    for _ in 0..eocd.count {
        r.read_exact(&mut h)?;
        if u32le(&h, 0) != CDH_SIG {
            return Err(Error::Corrupt("central directory entry"));
        }
        let made_by_os = h[5];
        let flags = u16le(&h, 8);
        let method = u16le(&h, 10);
        let (time, date) = (u16le(&h, 12), u16le(&h, 14));
        let crc = u32le(&h, 16);
        let mut csize = u32le(&h, 20) as u64;
        let mut usize_ = u32le(&h, 24) as u64;
        let (nlen, xlen, clen) = (u16le(&h, 28) as usize, u16le(&h, 30) as usize, u16le(&h, 32) as usize);
        let ext_attr = u32le(&h, 38);
        let mut local = u32le(&h, 42) as u64;
        let mut name = vec![0u8; nlen];
        r.read_exact(&mut name)?;
        let mut extra = vec![0u8; xlen];
        r.read_exact(&mut extra)?;
        r.skip(clen as u64)?;

        let mut mtime = dos_to_unix(time, date);
        let mut unicode_name: Option<String> = None;
        let mut x = 0;
        while x + 4 <= extra.len() {
            let id = u16le(&extra, x);
            let sz = u16le(&extra, x + 2) as usize;
            let d = &extra[x + 4..(x + 4 + sz).min(extra.len())];
            match id {
                0x0001 => {
                    let mut o = 0;
                    let mut take = |v: &mut u64| {
                        if *v == 0xFFFF_FFFF && o + 8 <= d.len() {
                            *v = u64le(d, o);
                            o += 8;
                        }
                    };
                    take(&mut usize_);
                    take(&mut csize);
                    take(&mut local);
                }
                0x5455 if !d.is_empty() && d[0] & 1 != 0 && d.len() >= 5 => {
                    mtime = Some(u32le(d, 1) as i32 as i64);
                }
                0x7075 if d.len() > 5 && d[0] == 1 => {
                    let stored_crc = u32le(d, 1);
                    if crc32fast::hash(&name) == stored_crc {
                        if let Ok(s) = core::str::from_utf8(&d[5..]) {
                            unicode_name = Some(String::from(s));
                        }
                    }
                }
                _ => {}
            }
            x += 4 + sz;
        }
        let raw = unicode_name.unwrap_or_else(|| decode_name(&name, flags & 0x800 != 0));
        let unix_mode = if made_by_os == 3 { Some(ext_attr >> 16) } else { None };
        let is_dir = raw.ends_with('/')
            || raw.ends_with('\\')
            || unix_mode.map(|m| m & 0o170000 == 0o040000).unwrap_or(false)
            || (made_by_os == 0 && ext_attr & 0x10 != 0);
        let is_link = unix_mode.map(|m| m & 0o170000 == 0o120000).unwrap_or(false);
        let kind = if is_dir {
            EntryKind::Dir
        } else if is_link {
            EntryKind::Symlink
        } else {
            EntryKind::File
        };
        let local = (local as i128 + shift).max(0) as u64;
        let mut e = Entry::new(raw, kind, if is_dir { 0 } else { usize_ }, Loc::Zip { local_offset: local, method, csize, crc });
        // Windows tools sometimes use backslashes.
        let (p, safe) = normalize(&e.raw_path.replace('\\', "/"));
        e.path = p;
        e.safe = safe;
        e.compressed_size = Some(csize);
        e.mtime = mtime;
        e.mode = unix_mode.map(|m| m & 0o7777).filter(|&m| m != 0);
        e.encrypted = flags & 1 != 0;
        if !(e.path.is_empty() && kind == EntryKind::Dir) {
            entries.push(e);
        }
    }
    Ok(entries)
}

pub(crate) fn extract<S: ByteSource + ?Sized>(
    src: &S,
    e: &Entry,
    out: &mut dyn FnMut(&[u8]) -> core::result::Result<(), IoError>,
    done: &mut u64,
    total: u64,
    progress: ProgressFn<'_>,
) -> Result<()> {
    let Loc::Zip { local_offset, method, csize, crc } = e.loc else { return Err(Error::NotAFile) };
    let mut lh = [0u8; 30];
    read_exact_at(src, local_offset, &mut lh)?;
    if u32le(&lh, 0) != LFH_SIG {
        return Err(Error::Corrupt("local file header"));
    }
    let start = local_offset + 30 + u16le(&lh, 26) as u64 + u16le(&lh, 28) as u64;
    let end = start.checked_add(csize).filter(|&x| x <= src.len()).ok_or(Error::Corrupt("member data out of range"))?;
    let raw = SrcReader::new(src, start, end);
    let mut hasher = crc32fast::Hasher::new();
    let mut input: alloc::boxed::Box<dyn Input + '_> = match method {
        0 => {
            if csize != e.size {
                return Err(Error::Corrupt("stored size mismatch"));
            }
            alloc::boxed::Box::new(raw)
        }
        8 => alloc::boxed::Box::new(Inflate::new(raw)),
        93 => alloc::boxed::Box::new(ZstdDecoder::new(raw)),
        95 => alloc::boxed::Box::new(XzDecoder::new(raw)),
        14 => {
            let mut raw = raw;
            let mut p = [0u8; 9];
            raw.read_exact(&mut p)?;
            if u16le(&p, 2) != 5 {
                return Err(Error::Unsupported("zip lzma properties"));
            }
            let dict = u32le(&p, 5);
            let flags = u16le(&lh, 6);
            let unpacked = if flags & 2 != 0 { u64::MAX } else { e.size };
            alloc::boxed::Box::new(LzmaDecoder::new(raw, p[4], dict, unpacked)?)
        }
        9 => return Err(Error::Unsupported("zip deflate64")),
        12 => return Err(Error::Unsupported("zip bzip2")),
        99 => return Err(Error::Encrypted),
        _ => return Err(Error::Unsupported("zip compression method")),
    };
    pump(&mut Take { inner: &mut input, left: e.size }, e.size, out, done, total, progress, Some(&mut hasher))?;
    if hasher.finalize() != crc {
        return Err(Error::Checksum);
    }
    Ok(())
}

// ---------------------------------------------------------------- writing

pub(crate) struct CdRecord {
    pub name: Vec<u8>,
    pub is_dir: bool,
    pub mtime: i64,
    pub mode: u32,
    pub method: u16,
    pub crc: u32,
    pub csize: u64,
    pub usize: u64,
    pub offset: u64,
    pub zip64_local: bool,
}

const MAX32: u64 = 0xFFFF_FFFF;

fn ext_time(v: &mut Vec<u8>, mtime: i64) {
    v.extend_from_slice(&0x5455u16.to_le_bytes());
    v.extend_from_slice(&5u16.to_le_bytes());
    v.push(1);
    v.extend_from_slice(&(mtime.clamp(i32::MIN as i64, i32::MAX as i64) as i32).to_le_bytes());
}

/// Local header for a streamed member (sizes in the data descriptor).
pub(crate) fn local_header(name: &[u8], mtime: i64, method: u16, zip64: bool, is_dir: bool) -> Vec<u8> {
    let (t, d) = unix_to_dos(mtime);
    let mut v = Vec::with_capacity(30 + name.len() + 29);
    v.extend_from_slice(&LFH_SIG.to_le_bytes());
    v.extend_from_slice(&(if zip64 { 45u16 } else { 20 }).to_le_bytes());
    let flags: u16 = 0x800 | if is_dir { 0 } else { 0x8 };
    v.extend_from_slice(&flags.to_le_bytes());
    v.extend_from_slice(&method.to_le_bytes());
    v.extend_from_slice(&t.to_le_bytes());
    v.extend_from_slice(&d.to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes()); // crc
    let sz: u32 = if zip64 { 0xFFFF_FFFF } else { 0 };
    v.extend_from_slice(&sz.to_le_bytes());
    v.extend_from_slice(&sz.to_le_bytes());
    v.extend_from_slice(&(name.len() as u16).to_le_bytes());
    let mut extra = Vec::new();
    if zip64 {
        extra.extend_from_slice(&1u16.to_le_bytes());
        extra.extend_from_slice(&16u16.to_le_bytes());
        extra.extend_from_slice(&0u64.to_le_bytes());
        extra.extend_from_slice(&0u64.to_le_bytes());
    }
    ext_time(&mut extra, mtime);
    v.extend_from_slice(&(extra.len() as u16).to_le_bytes());
    v.extend_from_slice(name);
    v.extend_from_slice(&extra);
    v
}

pub(crate) fn data_descriptor(crc: u32, csize: u64, usize: u64, zip64: bool) -> Vec<u8> {
    let mut v = Vec::with_capacity(24);
    v.extend_from_slice(&0x0807_4b50u32.to_le_bytes());
    v.extend_from_slice(&crc.to_le_bytes());
    if zip64 {
        v.extend_from_slice(&csize.to_le_bytes());
        v.extend_from_slice(&usize.to_le_bytes());
    } else {
        v.extend_from_slice(&(csize as u32).to_le_bytes());
        v.extend_from_slice(&(usize as u32).to_le_bytes());
    }
    v
}

pub(crate) fn central_record(r: &CdRecord, force64: bool) -> Vec<u8> {
    let (t, d) = unix_to_dos(r.mtime);
    let big_u = r.usize >= MAX32 || (force64 && r.zip64_local);
    let big_c = r.csize >= MAX32 || (force64 && r.zip64_local);
    let big_o = r.offset >= MAX32 || force64;
    let mut z64 = Vec::new();
    if big_u {
        z64.extend_from_slice(&r.usize.to_le_bytes());
    }
    if big_c {
        z64.extend_from_slice(&r.csize.to_le_bytes());
    }
    if big_o {
        z64.extend_from_slice(&r.offset.to_le_bytes());
    }
    let mut extra = Vec::new();
    if !z64.is_empty() {
        extra.extend_from_slice(&1u16.to_le_bytes());
        extra.extend_from_slice(&(z64.len() as u16).to_le_bytes());
        extra.extend_from_slice(&z64);
    }
    ext_time(&mut extra, r.mtime);
    let needed: u16 = if !z64.is_empty() || r.zip64_local { 45 } else { 20 };
    let mut v = Vec::with_capacity(46 + r.name.len() + extra.len());
    v.extend_from_slice(&CDH_SIG.to_le_bytes());
    v.extend_from_slice(&(0x0300u16 | 63).to_le_bytes()); // Unix, spec 6.3
    v.extend_from_slice(&needed.to_le_bytes());
    let flags: u16 = 0x800 | if r.is_dir { 0 } else { 0x8 };
    v.extend_from_slice(&flags.to_le_bytes());
    v.extend_from_slice(&r.method.to_le_bytes());
    v.extend_from_slice(&t.to_le_bytes());
    v.extend_from_slice(&d.to_le_bytes());
    v.extend_from_slice(&r.crc.to_le_bytes());
    v.extend_from_slice(&(if big_c { MAX32 } else { r.csize } as u32).to_le_bytes());
    v.extend_from_slice(&(if big_u { MAX32 } else { r.usize } as u32).to_le_bytes());
    v.extend_from_slice(&(r.name.len() as u16).to_le_bytes());
    v.extend_from_slice(&(extra.len() as u16).to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes()); // comment
    v.extend_from_slice(&0u16.to_le_bytes()); // disk
    v.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
    let unix: u32 = if r.is_dir { 0o040000 | r.mode } else { 0o100000 | r.mode };
    let ext = (unix << 16) | if r.is_dir { 0x10 } else { 0 };
    v.extend_from_slice(&ext.to_le_bytes());
    v.extend_from_slice(&(if big_o { MAX32 } else { r.offset } as u32).to_le_bytes());
    v.extend_from_slice(&r.name);
    v.extend_from_slice(&extra);
    v
}

pub(crate) fn end_records(count: u64, cd_offset: u64, cd_size: u64, force64: bool) -> Vec<u8> {
    let mut v = Vec::new();
    let z64 = force64 || count >= 0xFFFF || cd_offset >= MAX32 || cd_size >= MAX32;
    if z64 {
        let z64_at = cd_offset + cd_size;
        v.extend_from_slice(&Z64_EOCD_SIG.to_le_bytes());
        v.extend_from_slice(&44u64.to_le_bytes());
        v.extend_from_slice(&(0x0300u16 | 63).to_le_bytes());
        v.extend_from_slice(&45u16.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&count.to_le_bytes());
        v.extend_from_slice(&count.to_le_bytes());
        v.extend_from_slice(&cd_size.to_le_bytes());
        v.extend_from_slice(&cd_offset.to_le_bytes());
        v.extend_from_slice(&Z64_LOC_SIG.to_le_bytes());
        v.extend_from_slice(&0u32.to_le_bytes());
        v.extend_from_slice(&z64_at.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
    }
    v.extend_from_slice(&EOCD_SIG.to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes());
    let c16 = if z64 { 0xFFFF } else { count as u16 };
    v.extend_from_slice(&c16.to_le_bytes());
    v.extend_from_slice(&c16.to_le_bytes());
    v.extend_from_slice(&(if z64 { MAX32 } else { cd_size } as u32).to_le_bytes());
    v.extend_from_slice(&(if z64 { MAX32 } else { cd_offset } as u32).to_le_bytes());
    v.extend_from_slice(&0u16.to_le_bytes());
    v
}
