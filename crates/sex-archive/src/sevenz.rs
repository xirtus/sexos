//! 7z reading: listing (plain and LZMA-compressed headers) and extraction of
//! folders that use a single copy, LZMA, LZMA2 or deflate coder. Filter
//! chains (BCJ, delta), bzip2, PPMd and AES are listed but refused on
//! extract with `Unsupported` / `Encrypted`. Also hosts the `.xz` index
//! reader used to size single-file xz streams.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::codec::{Inflate, Lzma2Decoder, LzmaDecoder};
use crate::io::{read_exact_at, ByteSource, Input, IoError, SrcReader, Take};
use crate::{pump, Archive, Entry, EntryKind, Error, ExtractVisitor, Loc, ProgressFn, Result};

pub(crate) const MAGIC: [u8; 6] = [b'7', b'z', 0xbc, 0xaf, 0x27, 0x1c];
const MAX_HEADER: u64 = 64 << 20;

struct Cur<'a> {
    b: &'a [u8],
    p: usize,
}

const TRUNC: Error = Error::Corrupt("7z header is truncated");

impl<'a> Cur<'a> {
    fn u8(&mut self) -> Result<u8> {
        let v = *self.b.get(self.p).ok_or(TRUNC)?;
        self.p += 1;
        Ok(v)
    }
    fn bytes(&mut self, n: u64) -> Result<&'a [u8]> {
        let n = usize::try_from(n).map_err(|_| TRUNC)?;
        let end = self.p.checked_add(n).filter(|&e| e <= self.b.len()).ok_or(TRUNC)?;
        let s = &self.b[self.p..end];
        self.p = end;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64> {
        let b = self.bytes(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }
    fn num(&mut self) -> Result<u64> {
        let first = self.u8()?;
        let mut mask = 0x80u8;
        let mut value: u64 = 0;
        for i in 0..8 {
            if first & mask == 0 {
                let high = (first & mask.wrapping_sub(1)) as u64;
                value |= high << (8 * i);
                return Ok(value);
            }
            value |= (self.u8()? as u64) << (8 * i);
            mask >>= 1;
        }
        Ok(value)
    }
    /// A count that must be plausible for the header size.
    fn count(&mut self) -> Result<usize> {
        let n = self.num()?;
        if n > self.b.len() as u64 * 8 + 64 {
            return Err(Error::Corrupt("7z count out of range"));
        }
        Ok(n as usize)
    }
    fn bits(&mut self, n: usize) -> Result<Vec<bool>> {
        let mut v = Vec::with_capacity(n);
        let mut byte = 0u8;
        for i in 0..n {
            if i % 8 == 0 {
                byte = self.u8()?;
            }
            v.push(byte & (0x80 >> (i % 8)) != 0);
        }
        Ok(v)
    }
    fn defined(&mut self, n: usize) -> Result<Vec<bool>> {
        if self.u8()? != 0 {
            Ok(vec![true; n])
        } else {
            self.bits(n)
        }
    }
    fn digests(&mut self, n: usize) -> Result<Vec<Option<u32>>> {
        let d = self.defined(n)?;
        d.into_iter().map(|x| if x { self.u32().map(Some) } else { Ok(None) }).collect()
    }
}

#[derive(Debug, Clone)]
struct Coder {
    id: Vec<u8>,
    num_in: u64,
    num_out: u64,
    props: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Folder {
    coders: Vec<Coder>,
    bind_out: Vec<u64>,
    num_packed: usize,
    unpack_sizes: Vec<u64>,
    crc: Option<u32>,
    /// Absolute offsets/sizes of this folder's packed streams.
    packs: Vec<(u64, u64)>,
    /// Number of files stored in the folder.
    num_streams: usize,
}

impl Folder {
    fn unpack_size(&self) -> u64 {
        let total_out: u64 = self.coders.iter().map(|c| c.num_out).sum();
        for i in 0..total_out {
            if !self.bind_out.contains(&i) {
                return self.unpack_sizes.get(i as usize).copied().unwrap_or(0);
            }
        }
        0
    }
}

#[derive(Default)]
pub(crate) struct Info {
    folders: Vec<Folder>,
    /// (size, crc) of each substream, in folder order.
    streams: Vec<(usize, u64, Option<u32>)>,
}

fn parse_folder(c: &mut Cur<'_>) -> Result<Folder> {
    let nc = c.count()?;
    if nc == 0 || nc > 64 {
        return Err(Error::Corrupt("7z coder count"));
    }
    let mut f = Folder::default();
    for _ in 0..nc {
        let flag = c.u8()?;
        if flag & 0x80 != 0 {
            return Err(Error::Unsupported("7z alternative coder methods"));
        }
        let id = c.bytes((flag & 0x0f) as u64)?.to_vec();
        let (num_in, num_out) = if flag & 0x10 != 0 { (c.num()?, c.num()?) } else { (1, 1) };
        if num_in > 64 || num_out > 64 {
            return Err(Error::Corrupt("7z coder streams"));
        }
        let props = if flag & 0x20 != 0 {
            let n = c.num()?;
            c.bytes(n)?.to_vec()
        } else {
            Vec::new()
        };
        f.coders.push(Coder { id, num_in, num_out, props });
    }
    let total_out: u64 = f.coders.iter().map(|c| c.num_out).sum();
    let total_in: u64 = f.coders.iter().map(|c| c.num_in).sum();
    let mut bind_in = Vec::new();
    for _ in 0..total_out.saturating_sub(1) {
        bind_in.push(c.num()?);
        f.bind_out.push(c.num()?);
    }
    let packed = total_in.checked_sub(total_out.saturating_sub(1)).ok_or(Error::Corrupt("7z bind pairs"))?;
    f.num_packed = packed as usize;
    if packed > 1 {
        for _ in 0..packed {
            c.num()?;
        }
    }
    Ok(f)
}

/// Parse StreamsInfo. Returns folders (with pack positions) and substreams.
fn parse_streams(c: &mut Cur<'_>) -> Result<Info> {
    let mut pack_pos = 0u64;
    let mut pack_sizes: Vec<u64> = Vec::new();
    let mut info = Info::default();
    let mut sub_counts: Option<Vec<usize>> = None;
    let mut sub_sizes: Option<Vec<u64>> = None;
    let mut sub_crcs: Option<Vec<Option<u32>>> = None;
    loop {
        match c.u8()? {
            0x00 => break,
            0x06 => {
                pack_pos = c.num()?;
                let n = c.count()?;
                loop {
                    match c.u8()? {
                        0x00 => break,
                        0x09 => {
                            for _ in 0..n {
                                pack_sizes.push(c.num()?);
                            }
                        }
                        0x0A => {
                            c.digests(n)?;
                        }
                        _ => {
                            let sz = c.num()?;
                            c.bytes(sz)?;
                        }
                    }
                }
            }
            0x07 => {
                if c.u8()? != 0x0B {
                    return Err(Error::Corrupt("7z unpack info"));
                }
                let nf = c.count()?;
                if c.u8()? != 0 {
                    return Err(Error::Unsupported("7z external folders"));
                }
                for _ in 0..nf {
                    info.folders.push(parse_folder(c)?);
                }
                if c.u8()? != 0x0C {
                    return Err(Error::Corrupt("7z unpack sizes"));
                }
                for f in info.folders.iter_mut() {
                    let outs: u64 = f.coders.iter().map(|c| c.num_out).sum();
                    for _ in 0..outs {
                        f.unpack_sizes.push(c.num()?);
                    }
                }
                loop {
                    match c.u8()? {
                        0x00 => break,
                        0x0A => {
                            let d = c.digests(nf)?;
                            for (f, crc) in info.folders.iter_mut().zip(d) {
                                f.crc = crc;
                            }
                        }
                        _ => {
                            let sz = c.num()?;
                            c.bytes(sz)?;
                        }
                    }
                }
            }
            0x08 => {
                let nf = info.folders.len();
                let mut counts = vec![1usize; nf];
                let mut t = c.u8()?;
                if t == 0x0D {
                    for x in counts.iter_mut() {
                        *x = c.count()?;
                    }
                    t = c.u8()?;
                }
                let mut sizes = Vec::new();
                let has_sizes = t == 0x09;
                for (i, &n) in counts.iter().enumerate() {
                    if n == 0 {
                        continue;
                    }
                    let total = info.folders[i].unpack_size();
                    let mut sum = 0u64;
                    for _ in 0..n - 1 {
                        let s = if has_sizes { c.num()? } else { 0 };
                        sum = sum.checked_add(s).ok_or(Error::Corrupt("7z sizes"))?;
                        sizes.push(s);
                    }
                    sizes.push(total.checked_sub(sum).ok_or(Error::Corrupt("7z sizes"))?);
                }
                if has_sizes {
                    t = c.u8()?;
                }
                let mut crcs: Vec<Option<u32>> = Vec::new();
                let unknown: usize = counts
                    .iter()
                    .enumerate()
                    .map(|(i, &n)| if n == 1 && info.folders[i].crc.is_some() { 0 } else { n })
                    .sum();
                let mut digests: Vec<Option<u32>> = Vec::new();
                loop {
                    match t {
                        0x00 => break,
                        0x0A => digests = c.digests(unknown)?,
                        _ => {
                            let sz = c.num()?;
                            c.bytes(sz)?;
                        }
                    }
                    t = c.u8()?;
                }
                let mut di = digests.into_iter();
                for (i, &n) in counts.iter().enumerate() {
                    if n == 1 && info.folders[i].crc.is_some() {
                        crcs.push(info.folders[i].crc);
                    } else {
                        for _ in 0..n {
                            crcs.push(di.next().flatten());
                        }
                    }
                }
                sub_counts = Some(counts);
                sub_sizes = Some(sizes);
                sub_crcs = Some(crcs);
            }
            _ => return Err(Error::Corrupt("7z streams info")),
        }
    }
    // Assign packed streams to folders.
    let mut pos = 32 + pack_pos;
    let mut ps = pack_sizes.into_iter();
    for f in info.folders.iter_mut() {
        for _ in 0..f.num_packed {
            let s = ps.next().ok_or(Error::Corrupt("7z pack streams"))?;
            f.packs.push((pos, s));
            pos = pos.checked_add(s).ok_or(Error::Corrupt("7z pack size"))?;
        }
    }
    let counts = sub_counts.unwrap_or_else(|| vec![1; info.folders.len()]);
    let sizes = sub_sizes.unwrap_or_else(|| info.folders.iter().map(|f| f.unpack_size()).collect());
    let crcs = sub_crcs.unwrap_or_else(|| info.folders.iter().map(|f| f.crc).collect());
    let mut k = 0;
    for (i, &n) in counts.iter().enumerate() {
        info.folders[i].num_streams = n;
        for _ in 0..n {
            let s = *sizes.get(k).ok_or(Error::Corrupt("7z substreams"))?;
            info.streams.push((i, s, crcs.get(k).copied().flatten()));
            k += 1;
        }
    }
    Ok(info)
}

fn lzma2_dict(p: u8) -> Result<u32> {
    if p > 40 {
        return Err(Error::Corrupt("7z lzma2 dictionary"));
    }
    Ok(if p == 40 { u32::MAX } else { (2 | (p as u32 & 1)) << (p / 2 + 11) })
}

/// Decoder for a whole folder's output.
fn folder_reader<'a, S: ByteSource + ?Sized>(src: &'a S, f: &Folder) -> Result<Box<dyn Input + 'a>> {
    if f.coders.iter().any(|c| c.id == [6, 0xf1, 7, 1]) {
        return Err(Error::Encrypted);
    }
    if f.coders.len() != 1 || f.packs.len() != 1 {
        return Err(Error::Unsupported("7z coder chain (BCJ/delta filters)"));
    }
    let (start, size) = f.packs[0];
    let end = start.checked_add(size).filter(|&e| e <= src.len()).ok_or(Error::Corrupt("7z pack out of range"))?;
    let raw = SrcReader::new(src, start, end);
    let c = &f.coders[0];
    let unpack = f.unpack_size();
    let r: Box<dyn Input + 'a> = match c.id.as_slice() {
        [0] => Box::new(raw),
        [0x21] => Box::new(Lzma2Decoder::new(raw, lzma2_dict(*c.props.first().ok_or(TRUNC)?)?)),
        [3, 1, 1] => {
            if c.props.len() < 5 {
                return Err(Error::Corrupt("7z lzma properties"));
            }
            let dict = u32::from_le_bytes([c.props[1], c.props[2], c.props[3], c.props[4]]);
            Box::new(LzmaDecoder::new(raw, c.props[0], dict, unpack)?)
        }
        [4, 1, 8] => Box::new(Inflate::new(raw)),
        [4, 2, 2] => return Err(Error::Unsupported("7z bzip2")),
        [3, 4, 1] => return Err(Error::Unsupported("7z PPMd")),
        _ => return Err(Error::Unsupported("7z coder")),
    };
    Ok(Box::new(Take { inner: r, left: unpack }))
}

fn decode_folder_to_vec<S: ByteSource + ?Sized>(src: &S, f: &Folder) -> Result<Vec<u8>> {
    let size = f.unpack_size();
    if size > MAX_HEADER {
        return Err(Error::TooLarge);
    }
    let mut v = vec![0u8; size as usize];
    folder_reader(src, f)?.read_exact(&mut v)?;
    if let Some(crc) = f.crc {
        if crc32fast::hash(&v) != crc {
            return Err(Error::Checksum);
        }
    }
    Ok(v)
}

fn filetime_to_unix(ft: u64) -> i64 {
    (ft / 10_000_000) as i64 - 11_644_473_600
}

pub(crate) fn open<S: ByteSource + ?Sized>(src: &S) -> Result<(Vec<Entry>, Info)> {
    let mut sh = [0u8; 32];
    read_exact_at(src, 0, &mut sh)?;
    if sh[6] != 0 {
        return Err(Error::Unsupported("7z format version"));
    }
    let mut c = Cur { b: &sh[12..32], p: 0 };
    let (off, size, crc) = (c.u64()?, c.u64()?, c.u32()?);
    if crc32fast::hash(&sh[12..32]) != u32::from_le_bytes([sh[8], sh[9], sh[10], sh[11]]) {
        return Err(Error::Corrupt("7z start header checksum"));
    }
    if size == 0 {
        return Ok((Vec::new(), Info::default()));
    }
    if size > MAX_HEADER || 32u64.checked_add(off).and_then(|x| x.checked_add(size)).map_or(true, |e| e > src.len()) {
        return Err(Error::Corrupt("7z header out of range"));
    }
    let mut header = vec![0u8; size as usize];
    read_exact_at(src, 32 + off, &mut header)?;
    if crc32fast::hash(&header) != crc {
        return Err(Error::Corrupt("7z header checksum"));
    }
    for _ in 0..4 {
        let mut c = Cur { b: &header, p: 0 };
        match c.u8()? {
            0x01 => return parse_header(&mut c),
            0x17 => {
                let info = parse_streams(&mut c)?;
                let f = info.folders.first().ok_or(Error::Corrupt("7z encoded header"))?;
                header = decode_folder_to_vec(src, f)?;
            }
            _ => return Err(Error::Corrupt("7z header")),
        }
    }
    Err(Error::Corrupt("7z header nesting"))
}

fn parse_header(c: &mut Cur<'_>) -> Result<(Vec<Entry>, Info)> {
    let mut id = c.u8()?;
    if id == 0x02 {
        loop {
            if c.u8()? == 0 {
                break;
            }
            let sz = c.num()?;
            c.bytes(sz)?;
        }
        id = c.u8()?;
    }
    if id == 0x03 {
        parse_streams(c)?;
        id = c.u8()?;
    }
    let mut info = Info::default();
    if id == 0x04 {
        info = parse_streams(c)?;
        id = c.u8()?;
    }
    let mut entries = Vec::new();
    if id == 0x05 {
        entries = parse_files(c, &info)?;
        id = c.u8()?;
    }
    if id != 0 {
        return Err(Error::Corrupt("7z header end"));
    }
    Ok((entries, info))
}

fn parse_files(c: &mut Cur<'_>, info: &Info) -> Result<Vec<Entry>> {
    let n = c.count()?;
    let mut empty_stream = vec![false; n];
    let mut empty_file: Vec<bool> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    let mut mtimes: Vec<Option<i64>> = vec![None; n];
    let mut attrs: Vec<Option<u32>> = vec![None; n];
    loop {
        let t = c.num()?;
        if t == 0 {
            break;
        }
        let size = c.num()?;
        let data = c.bytes(size)?;
        let mut d = Cur { b: data, p: 0 };
        match t {
            0x0E => empty_stream = d.bits(n)?,
            0x0F => empty_file = d.bits(empty_stream.iter().filter(|&&x| x).count())?,
            0x11 => {
                if d.u8()? != 0 {
                    return Err(Error::Unsupported("7z external names"));
                }
                let units: Vec<u16> = d.b[1..].chunks_exact(2).map(|p| u16::from_le_bytes([p[0], p[1]])).collect();
                for name in units.split(|&u| u == 0).take(n) {
                    names.push(char::decode_utf16(name.iter().copied()).map(|r| r.unwrap_or('\u{fffd}')).collect());
                }
            }
            0x14 => {
                let def = d.defined(n)?;
                if d.u8()? != 0 {
                    return Err(Error::Unsupported("7z external times"));
                }
                for (i, x) in def.into_iter().enumerate() {
                    if x {
                        mtimes[i] = Some(filetime_to_unix(d.u64()?));
                    }
                }
            }
            0x15 => {
                let def = d.defined(n)?;
                if d.u8()? != 0 {
                    return Err(Error::Unsupported("7z external attributes"));
                }
                for (i, x) in def.into_iter().enumerate() {
                    if x {
                        attrs[i] = Some(d.u32()?);
                    }
                }
            }
            _ => {}
        }
    }
    let mut entries = Vec::with_capacity(n);
    let mut stream = 0usize;
    let mut folder_off: Vec<u64> = vec![0; info.folders.len()];
    let mut empty_idx = 0usize;
    for i in 0..n {
        let name = names.get(i).cloned().unwrap_or_default();
        let attr = attrs[i];
        let unix = attr.filter(|a| a & 0x8000 != 0).map(|a| a >> 16);
        let e = if !empty_stream[i] {
            let &(folder, size, crc) = info.streams.get(stream).ok_or(Error::Corrupt("7z file without stream"))?;
            stream += 1;
            let offset = folder_off[folder];
            folder_off[folder] += size;
            let is_link = unix.map(|m| m & 0o170000 == 0o120000).unwrap_or(false);
            let kind = if is_link { EntryKind::Symlink } else { EntryKind::File };
            let mut e = Entry::new(name, kind, size, Loc::SevenZ { folder, offset, crc });
            e.encrypted = info.folders[folder].coders.iter().any(|c| c.id == [6, 0xf1, 7, 1]);
            e
        } else {
            let is_file = empty_file.get(empty_idx).copied().unwrap_or(false);
            empty_idx += 1;
            let is_dir = !is_file;
            Entry::new(name, if is_dir { EntryKind::Dir } else { EntryKind::File }, 0, Loc::None)
        };
        let mut e = e;
        e.mtime = mtimes[i];
        e.mode = unix.map(|m| m & 0o7777);
        if !(e.path.is_empty() && e.kind == EntryKind::Dir) {
            entries.push(e);
        }
    }
    Ok(entries)
}

pub(crate) fn extract_one<S: ByteSource>(
    a: &Archive<S>,
    info: &Info,
    index: usize,
    out: &mut dyn FnMut(&[u8]) -> core::result::Result<(), IoError>,
    done: &mut u64,
    total: u64,
    progress: ProgressFn<'_>,
) -> Result<()> {
    let e = &a.entries[index];
    match e.loc {
        Loc::None => pump(&mut &[][..], 0, out, done, total, progress, None),
        Loc::SevenZ { folder, offset, crc } => {
            let mut r = folder_reader(&a.src, &info.folders[folder])?;
            r.skip(offset)?;
            let mut h = crc32fast::Hasher::new();
            pump(&mut Take { inner: r, left: e.size }, e.size, out, done, total, progress, Some(&mut h))?;
            if crc.is_some_and(|c| c != h.finalize()) {
                return Err(Error::Checksum);
            }
            Ok(())
        }
        _ => Err(Error::NotAFile),
    }
}

pub(crate) fn extract_all<S: ByteSource>(
    a: &Archive<S>,
    info: &Info,
    visitor: &mut dyn ExtractVisitor,
    done: &mut u64,
    total: u64,
    progress: ProgressFn<'_>,
) -> Result<()> {
    // Empty files first, then each folder decoded once.
    for e in a.entries.iter().filter(|e| e.loc == Loc::None) {
        if a.visit_begin(visitor, e)?.is_some() {
            visitor.end(e).map_err(Error::Sink)?;
        }
    }
    for (fi, f) in info.folders.iter().enumerate() {
        let mut members: Vec<&Entry> =
            a.entries.iter().filter(|e| matches!(e.loc, Loc::SevenZ { folder, .. } if folder == fi)).collect();
        members.sort_by_key(|e| match e.loc {
            Loc::SevenZ { offset, .. } => offset,
            _ => 0,
        });
        let mut wanted = Vec::new();
        for e in &members {
            wanted.push(a.visit_begin_peek(e));
        }
        if !wanted.iter().any(|&w| w) {
            continue;
        }
        let mut r = crate::Counted { inner: folder_reader(&a.src, f)?, pos: 0 };
        for (e, want) in members.into_iter().zip(wanted) {
            if !want || a.visit_begin(visitor, e)?.is_none() {
                continue;
            }
            let Loc::SevenZ { offset, crc, .. } = e.loc else { continue };
            r.skip(offset - r.pos)?;
            let mut h = crc32fast::Hasher::new();
            let mut out = |b: &[u8]| visitor.data(b);
            pump(&mut Take { inner: &mut r, left: e.size }, e.size, &mut out, done, total, progress, Some(&mut h))?;
            if crc.is_some_and(|c| c != h.finalize()) {
                return Err(Error::Checksum);
            }
            visitor.end(e).map_err(Error::Sink)?;
        }
    }
    Ok(())
}

fn read_varint(c: &mut Cur<'_>) -> Result<u64> {
    let mut v = 0u64;
    for i in 0..9 {
        let b = c.u8()?;
        v |= ((b & 0x7f) as u64) << (7 * i);
        if b & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(Error::Corrupt("xz varint"))
}

/// Uncompressed size of a single-stream `.xz` file, from its index.
pub(crate) fn xz_size<S: ByteSource + ?Sized>(src: &S) -> Result<u64> {
    let len = src.len();
    if len < 24 {
        return Err(Error::Corrupt("xz too short"));
    }
    let mut foot = [0u8; 12];
    read_exact_at(src, len - 12, &mut foot)?;
    if &foot[10..12] != b"YZ" {
        return Err(Error::Unsupported("xz stream padding"));
    }
    let back = (u32::from_le_bytes([foot[4], foot[5], foot[6], foot[7]]) as u64 + 1) * 4;
    let idx_start = (len - 12).checked_sub(back).ok_or(Error::Corrupt("xz index"))?;
    if back > 1 << 20 {
        return Err(Error::TooLarge);
    }
    let mut idx = vec![0u8; back as usize];
    read_exact_at(src, idx_start, &mut idx)?;
    let mut c = Cur { b: &idx, p: 0 };
    if c.u8()? != 0 {
        return Err(Error::Corrupt("xz index"));
    }
    let n = read_varint(&mut c)?;
    let mut total = 0u64;
    let mut blocks = 0u64;
    for _ in 0..n {
        let unpadded = read_varint(&mut c)?;
        total = total.checked_add(read_varint(&mut c)?).ok_or(Error::TooLarge)?;
        blocks += (unpadded + 3) & !3;
    }
    if 12 + blocks != idx_start {
        // More than one stream: the index only covers the last one.
        return Err(Error::Unsupported("multi-stream xz size"));
    }
    Ok(total)
}
