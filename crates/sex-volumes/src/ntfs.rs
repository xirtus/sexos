// SPDX-License-Identifier: GPL-3.0-only
//! NTFS, read only: Windows disks and sticks open in Files, nothing is ever written.
//!
//! Everything on NTFS is a file in the master file table ($MFT), itself a file whose
//! first record describes where the rest of it lives. A file is a record of attributes;
//! big ones live outside the record as runs of clusters, and when a record overflows its
//! attributes spill into extension records named by an $ATTRIBUTE_LIST. Directories are
//! B+trees of $FILE_NAME keys: the root node in the record, the others in INDX blocks.
//! Records are read one at a time on demand; only the $MFT's own run list, the volume's
//! upper-case table and its label are kept.
use crate::{le16, le32, le64, parts, read_bytes, Disk, Entry, Error, Format, Kind, Result, Stat, Volume};
use alloc::{string::String, vec, vec::Vec};
use core::cmp::Ordering;

const MFT: u64 = 0;
const VOLUME: u64 = 3;
const ROOT: u64 = 5;
const BITMAP: u64 = 6;
const UPCASE: u64 = 10;

const STANDARD_INFORMATION: u32 = 0x10;
const ATTRIBUTE_LIST: u32 = 0x20;
const VOLUME_NAME: u32 = 0x60;
const DATA: u32 = 0x80;
const INDEX_ROOT: u32 = 0x90;
const INDEX_ALLOCATION: u32 = 0xA0;
/// "$I30" in UTF-16LE: the file name index of a directory.
const I30: &[u8] = b"$\0I\x003\x000\0";

/// Attribute flags: any compression method, or encrypted ($EFS).
const COMPRESSED: u16 = 0x00FF;
const ENCRYPTED: u16 = 0x4000;
/// $STANDARD_INFORMATION file attributes that make the unnamed $DATA not the contents.
const FILE_REPARSE: u32 = 0x400;
const FILE_ENCRYPTED: u32 = 0x4000;
/// Seconds from 1601-01-01 (Windows FILETIME's epoch) to 1970-01-01.
const FILETIME_TO_UNIX: u64 = 11_644_473_600;
/// B+trees deeper than this are taken as a loop in a corrupt index.
const MAX_DEPTH: usize = 32;

/// `n` bytes at `at`, or Corrupt when the structure claims bytes it doesn't have.
fn sl(b: &[u8], at: usize, n: usize) -> Result<&[u8]> {
    b.get(at..at.checked_add(n).ok_or(Error::Corrupt)?).ok_or(Error::Corrupt)
}
fn r16(b: &[u8], at: usize) -> Result<u16> { Ok(le16(sl(b, at, 2)?, 0)) }
fn r32(b: &[u8], at: usize) -> Result<u32> { Ok(le32(sl(b, at, 4)?, 0)) }
fn r64(b: &[u8], at: usize) -> Result<u64> { Ok(le64(sl(b, at, 8)?, 0)) }
fn mul(a: u64, b: u64) -> Result<u64> { a.checked_mul(b).ok_or(Error::Corrupt) }
fn add(a: u64, b: u64) -> Result<u64> { a.checked_add(b).ok_or(Error::Corrupt) }

/// Undoes the update sequence: the last two bytes of every 512-byte stride were swapped
/// for a check value on write, so a torn multi-sector write shows up as a mismatch.
fn fixup(buf: &mut [u8], magic: &[u8; 4]) -> Result<()> {
    if buf.get(..4) != Some(&magic[..]) { return Err(Error::Corrupt); }
    let (ofs, count) = (r16(buf, 4)? as usize, r16(buf, 6)? as usize);
    if count == 0 || (count - 1) * 512 != buf.len() { return Err(Error::Corrupt); }
    let usn: [u8; 2] = sl(buf, ofs, 2)?.try_into().unwrap();
    for i in 1..count {
        let fix: [u8; 2] = sl(buf, ofs + 2 * i, 2)?.try_into().unwrap();
        let end = i * 512;
        if buf[end - 2..end] != usn { return Err(Error::Corrupt); }
        buf[end - 2..end].copy_from_slice(&fix);
    }
    Ok(())
}

/// Clusters `vcn..vcn+len` of an attribute; `lcn` None is a hole (sparse, reads as zeros).
#[derive(Clone, Copy)]
struct Run { vcn: u64, lcn: Option<u64>, len: u64 }

/// Mapping pairs: a header byte giving the widths of a length and a signed LCN delta.
fn decode_runs(b: &[u8], mut vcn: u64, out: &mut Vec<Run>) -> Result<()> {
    let (mut lcn, mut at) = (0i64, 0usize);
    loop {
        let h = *b.get(at).ok_or(Error::Corrupt)?;
        if h == 0 { return Ok(()); }
        let (ln, on) = ((h & 15) as usize, (h >> 4) as usize);
        if ln == 0 || ln > 8 || on > 8 { return Err(Error::Corrupt); }
        let len = sl(b, at + 1, ln)?.iter().rev().fold(0u64, |v, &x| v << 8 | x as u64);
        let ob = sl(b, at + 1 + ln, on)?;
        let run = if on == 0 { None } else {
            let neg = ob[on - 1] & 0x80 != 0;
            let delta = ob.iter().rev().fold(if neg { -1i64 } else { 0 }, |v, &x| v << 8 | x as i64);
            lcn = lcn.checked_add(delta).filter(|l| *l >= 0).ok_or(Error::Corrupt)?;
            Some(lcn as u64)
        };
        if len == 0 { return Err(Error::Corrupt); }
        out.push(Run { vcn, lcn: run, len });
        vcn = add(vcn, len)?;
        at += 1 + ln + on;
    }
}

/// One attribute header inside a record.
struct Raw<'a> { ty: u32, name: &'a [u8], flags: u16, nonres: bool, body: &'a [u8] }
impl Raw<'_> {
    fn value(&self) -> Result<&[u8]> { sl(self.body, r16(self.body, 20)? as usize, r32(self.body, 16)? as usize) }
    fn lowest_vcn(&self) -> Result<u64> { if self.nonres { r64(self.body, 16) } else { Ok(0) } }
}
/// The attributes of a fixed-up record, up to the end marker.
fn raw_attrs(rec: &[u8]) -> Result<Vec<Raw<'_>>> {
    let used = (r32(rec, 0x18)? as usize).min(rec.len());
    let mut at = r16(rec, 0x14)? as usize;
    let mut out = Vec::new();
    loop {
        let ty = r32(rec, at)?;
        if ty == 0xFFFF_FFFF { return Ok(out); }
        let len = r32(rec, at + 4)? as usize;
        if len < 24 || len % 8 != 0 || at + len > used { return Err(Error::Corrupt); }
        let body = &rec[at..at + len];
        let name = sl(body, le16(body, 10) as usize, body[9] as usize * 2)?;
        out.push(Raw { ty, name, flags: le16(body, 12), nonres: body[8] != 0, body });
        at += len;
    }
}

/// An attribute's value, wherever it lives. Only the first extent carries the sizes.
struct Attr { flags: u16, resident: Option<Vec<u8>>, runs: Vec<Run>, size: u64, init: u64 }
impl Attr {
    fn new(r: &Raw) -> Result<Attr> {
        if !r.nonres {
            let v = r.value()?.to_vec();
            return Ok(Attr { flags: r.flags, size: v.len() as u64, init: v.len() as u64, resident: Some(v), runs: Vec::new() });
        }
        let mut a = Attr { flags: r.flags, resident: None, runs: Vec::new(), size: r64(r.body, 48)?, init: r64(r.body, 56)? };
        a.init = a.init.min(a.size);
        a.extend(r)?;
        Ok(a)
    }
    /// Adds the runs of a later extent; each extent's LCN deltas start again from zero.
    fn extend(&mut self, r: &Raw) -> Result<()> {
        if !r.nonres || self.resident.is_some() { return Err(Error::Corrupt); }
        let ro = r16(r.body, 32)? as usize;
        decode_runs(r.body.get(ro..).ok_or(Error::Corrupt)?, r.lowest_vcn()?, &mut self.runs)
    }
    fn unreadable(&self) -> bool { self.flags & (COMPRESSED | ENCRYPTED) != 0 }
}

/// A directory's index: the root node and, for bigger ones, the INDX blocks.
struct Dir { root: Vec<u8>, alloc: Option<Attr>, block: usize }

/// A $FILE_NAME index key. Namespace 2 is a DOS 8.3 alias of another entry.
struct Key { mft: u64, name: Vec<u16>, ns: u8, dir: bool }

pub struct Ntfs {
    cluster: u64,
    clusters: u64,
    record: usize,
    /// The $MFT's $DATA: where each record lives.
    mft: Attr,
    /// $UpCase: how Windows folds case, for name lookup. Empty means fall back to Unicode.
    upcase: Vec<u16>,
    label: String,
}

/// The boot sector's size fields: positive counts clusters, negative is a power of two of bytes.
fn unit_size(v: u8, cluster: u64) -> Result<u64> {
    match v as i8 {
        n @ 1..=127 => mul(n as u64, cluster),
        n @ -31..=-1 => Ok(1u64 << (-(n as i32))),
        _ => Err(Error::Corrupt),
    }
}

impl Ntfs {
    pub fn mount(disk: &mut dyn Disk) -> Result<Self> {
        let mut b = [0u8; 512];
        disk.read(0, &mut b)?;
        if &b[3..11] != b"NTFS    " { return Err(Error::Unsupported); }
        let bps = le16(&b, 11) as u64;
        if !bps.is_power_of_two() || !(256..=4096).contains(&bps) { return Err(Error::Corrupt); }
        // Sectors per cluster; past 128 the byte is a negative power of two (64K+ clusters).
        let spc = match b[13] { v @ 1..=0x80 => v as u64, v if v >= 0xF4 => 1 << (256 - v as u32), _ => return Err(Error::Corrupt) };
        let cluster = bps * spc;
        if cluster > 2 << 20 { return Err(Error::Corrupt); }
        let clusters = le64(&b, 0x28) / spc;
        let record = unit_size(b[0x40], cluster)?;
        if clusters == 0 || record < 512 || record > 65536 || record % 512 != 0 { return Err(Error::Corrupt); }
        let mft_lcn = le64(&b, 0x30);
        let mut v = Ntfs {
            cluster, clusters, record: record as usize,
            mft: Attr { flags: 0, resident: None, runs: vec![Run { vcn: 0, lcn: Some(mft_lcn), len: record.div_ceil(cluster) }], size: record, init: record },
            upcase: Vec::new(), label: String::new(),
        };
        // Bootstrap: record 0's own $DATA first, then again through its attribute list,
        // whose extension records the first extent can now reach.
        let rec0 = v.record(disk, MFT)?;
        let first = raw_attrs(&rec0)?.into_iter().find(|r| r.ty == DATA && r.name.is_empty() && r.nonres).ok_or(Error::Corrupt)?;
        v.mft = Attr::new(&first)?;
        let a = v.attr(disk, &rec0, MFT, DATA, &[])?.ok_or(Error::Corrupt)?;
        if a.resident.is_some() || a.unreadable() { return Err(Error::Corrupt); }
        v.mft = a;

        let rec = v.record(disk, VOLUME)?;
        if let Some(a) = v.attr(disk, &rec, VOLUME, VOLUME_NAME, &[])? {
            let raw = v.load(disk, &a, 512)?;
            let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            v.label = String::from_utf16_lossy(&units);
        }
        // A volume without a sane $UpCase still mounts; lookups fold case the Unicode way.
        let up = (|| {
            let rec = v.record(disk, UPCASE)?;
            let a = v.attr(disk, &rec, UPCASE, DATA, &[])?.ok_or(Error::Corrupt)?;
            if a.size != 131072 || a.unreadable() { return Err(Error::Corrupt); }
            v.load(disk, &a, 131072)
        })();
        if let Ok(t) = up { v.upcase = t.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect(); }
        Ok(v)
    }

    /// Reads `buf` at byte `off` of an attribute; returns how many bytes it holds there.
    fn read_attr(&self, disk: &mut dyn Disk, a: &Attr, off: u64, buf: &mut [u8]) -> Result<usize> {
        if off >= a.size { return Ok(0); }
        let n = (a.size - off).min(buf.len() as u64) as usize;
        if let Some(v) = &a.resident {
            buf[..n].copy_from_slice(&v[off as usize..off as usize + n]);
            return Ok(n);
        }
        let mut done = 0;
        while done < n {
            let pos = off + done as u64;
            // Past the initialized size the bytes were never written: zeros.
            if pos >= a.init { buf[done..n].fill(0); break; }
            let vcn = pos / self.cluster;
            let run = a.runs.iter().find(|r| vcn >= r.vcn && vcn - r.vcn < r.len).ok_or(Error::Corrupt)?;
            let start = mul(run.vcn, self.cluster)?;
            let end = mul(add(run.vcn, run.len)?, self.cluster)?;
            let take = (end - pos).min((n - done) as u64).min(a.init - pos) as usize;
            let out = &mut buf[done..done + take];
            match run.lcn {
                None => out.fill(0),
                Some(lcn) => {
                    if add(lcn, run.len)? > self.clusters { return Err(Error::Corrupt); }
                    read_bytes(disk, add(mul(lcn, self.cluster)?, pos - start)?, out)?;
                }
            }
            done += take;
        }
        Ok(n)
    }
    /// An attribute's whole value, refusing one bigger than `max` bytes.
    fn load(&self, disk: &mut dyn Disk, a: &Attr, max: u64) -> Result<Vec<u8>> {
        if a.size > max { return Err(Error::Corrupt); }
        let mut v = vec![0; a.size as usize];
        self.read_attr(disk, a, 0, &mut v)?;
        Ok(v)
    }

    /// MFT record `n`, fixed up and in use.
    fn record(&self, disk: &mut dyn Disk, n: u64) -> Result<Vec<u8>> {
        let off = mul(n, self.record as u64)?;
        if add(off, self.record as u64)? > self.mft.size { return Err(Error::Corrupt); }
        let mut b = vec![0; self.record];
        self.read_attr(disk, &self.mft, off, &mut b)?;
        fixup(&mut b, b"FILE")?;
        if r16(&b, 0x16)? & 1 == 0 { return Err(Error::Corrupt); }
        Ok(b)
    }

    /// Attribute `ty` named `name` of file `n` (whose base record is `base`), gathering its
    /// extents from extension records when the file has an $ATTRIBUTE_LIST.
    fn attr(&self, disk: &mut dyn Disk, base: &[u8], n: u64, ty: u32, name: &[u8]) -> Result<Option<Attr>> {
        let raws = raw_attrs(base)?;
        let Some(list) = raws.iter().find(|r| r.ty == ATTRIBUTE_LIST) else {
            return raws.iter().find(|r| r.ty == ty && r.name == name).map(Attr::new).transpose();
        };
        let list = self.load(disk, &Attr::new(list)?, 1 << 20)?;
        let mut out: Option<Attr> = None;
        let mut at = 0;
        while at + 26 <= list.len() {
            let (et, el) = (le32(&list, at), le16(&list, at + 4) as usize);
            if el < 26 { return Err(Error::Corrupt); }
            let ename = sl(&list, at + list[at + 7] as usize, list[at + 6] as usize * 2)?;
            if et == ty && ename == name {
                let (vcn, r) = (r64(&list, at + 8)?, r64(&list, at + 16)? & 0xFFFF_FFFF_FFFF);
                let ext;
                let rec = if r == n { base } else { ext = self.record(disk, r)?; &ext };
                let raws = raw_attrs(rec)?;
                let piece = raws.iter().find(|p| p.ty == ty && p.name == name && p.lowest_vcn().ok() == Some(vcn)).ok_or(Error::Corrupt)?;
                match &mut out { None => out = Some(Attr::new(piece)?), Some(a) => a.extend(piece)? }
            }
            at += el;
        }
        Ok(out)
    }
    fn data(&self, disk: &mut dyn Disk, rec: &[u8], n: u64) -> Result<Option<Attr>> { self.attr(disk, rec, n, DATA, &[]) }

    fn open_dir(&self, disk: &mut dyn Disk, n: u64) -> Result<Dir> {
        let rec = self.record(disk, n)?;
        if r16(&rec, 0x16)? & 2 == 0 { return Err(Error::NotDir); }
        let root = self.attr(disk, &rec, n, INDEX_ROOT, I30)?.ok_or(Error::Corrupt)?;
        let root = self.load(disk, &root, 65536)?;
        let block = r32(&root, 8)? as usize;
        if block < 512 || block > 65536 || block % 512 != 0 { return Err(Error::Corrupt); }
        Ok(Dir { root, alloc: self.attr(disk, &rec, n, INDEX_ALLOCATION, I30)?, block })
    }
    /// The INDX block at index VCN `vcn`: in clusters, or 512-byte units when blocks are
    /// smaller than a cluster.
    fn node(&self, disk: &mut dyn Disk, dir: &Dir, vcn: u64) -> Result<Vec<u8>> {
        let unit = if dir.block as u64 >= self.cluster { self.cluster } else { 512 };
        let a = dir.alloc.as_ref().ok_or(Error::Corrupt)?;
        if a.unreadable() { return Err(Error::Corrupt); }
        let mut b = vec![0; dir.block];
        if self.read_attr(disk, a, mul(vcn, unit)?, &mut b)? != b.len() { return Err(Error::Corrupt); }
        fixup(&mut b, b"INDX")?;
        Ok(b)
    }

    fn fold(&self, c: u16) -> u16 {
        if let Some(&u) = self.upcase.get(c as usize) { return u; }
        let mut up = char::from_u32(c as u32).map(|ch| ch.to_uppercase());
        match up.as_mut().map(|u| (u.next(), u.next())) {
            Some((Some(u), None)) if (u as u32) < 0x10000 => u as u16,
            _ => c,
        }
    }
    /// Windows' file name collation: compare upper-cased UTF-16 units.
    fn collate(&self, a: &[u16], b: &[u16]) -> Ordering {
        a.iter().map(|&c| self.fold(c)).cmp(b.iter().map(|&c| self.fold(c)))
    }

    /// Walks a node and everything below it, in name order. Depth and `budget` (how many
    /// INDX blocks the index holds) bound a corrupt tree that points back into itself.
    fn walk(&self, disk: &mut dyn Disk, dir: &Dir, node: &[u8], hdr: usize, depth: usize, budget: &mut u64, out: &mut Vec<Key>) -> Result<()> {
        if depth > MAX_DEPTH { return Err(Error::Corrupt); }
        for (key, sub) in entries(node, hdr)? {
            if let Some(vcn) = sub {
                *budget = budget.checked_sub(1).ok_or(Error::Corrupt)?;
                let b = self.node(disk, dir, vcn)?;
                self.walk(disk, dir, &b, 24, depth + 1, budget, out)?;
            }
            if let Some(k) = key { out.push(k); }
        }
        Ok(())
    }
    /// Descends the B+tree of directory `n` to `name`, matching case the way Windows does.
    fn lookup(&self, disk: &mut dyn Disk, n: u64, name: &[u16]) -> Result<u64> {
        let dir = self.open_dir(disk, n)?;
        let mut node = entries(&dir.root, 16)?;
        for _ in 0..MAX_DEPTH {
            let mut next = None;
            for (key, sub) in node {
                match key.map(|k| (self.collate(name, &k.name), k.mft)) {
                    Some((Ordering::Equal, mft)) => return Ok(mft),
                    Some((Ordering::Greater, _)) => continue,
                    _ => { next = sub; break; }
                }
            }
            let Some(vcn) = next else { return Err(Error::NotFound) };
            node = entries(&self.node(disk, &dir, vcn)?, 24)?;
        }
        Err(Error::Corrupt)
    }
    fn resolve(&self, disk: &mut dyn Disk, path: &[u8]) -> Result<u64> {
        let mut n = ROOT;
        for p in parts(path)? {
            let name: Vec<u16> = core::str::from_utf8(p).map_err(|_| Error::Name)?.encode_utf16().collect();
            n = self.lookup(disk, n, &name)?;
        }
        Ok(n)
    }
    fn stat_of(&self, disk: &mut dyn Disk, n: u64) -> Result<Stat> {
        let rec = self.record(disk, n)?;
        let mtime = match raw_attrs(&rec)?.iter().find(|r| r.ty == STANDARD_INFORMATION && !r.nonres) {
            Some(si) => (r64(si.value()?, 8)? / 10_000_000).saturating_sub(FILETIME_TO_UNIX),
            None => 0,
        };
        if r16(&rec, 0x16)? & 2 != 0 { return Ok(Stat { kind: Kind::Dir, size: 0, mtime }); }
        let size = self.data(disk, &rec, n)?.map_or(0, |a| a.size);
        Ok(Stat { kind: Kind::File, size, mtime })
    }
}

/// The entries of an index node whose header starts at `hdr`: each key (none for the
/// closing entry) and the VCN of the child node holding the names before it.
fn entries(node: &[u8], hdr: usize) -> Result<Vec<(Option<Key>, Option<u64>)>> {
    let first = hdr + r32(node, hdr)? as usize;
    let end = hdr + r32(node, hdr + 4)? as usize;
    if end > node.len() { return Err(Error::Corrupt); }
    let (mut at, mut out) = (first, Vec::new());
    loop {
        if at + 16 > end { return Err(Error::Corrupt); }
        let (el, kl, flags) = (le16(node, at + 8) as usize, le16(node, at + 10) as usize, le16(node, at + 12));
        if el < 16 || at + el > end { return Err(Error::Corrupt); }
        let sub = if flags & 1 != 0 { Some(r64(node, at + el - 8)?) } else { None };
        if flags & 2 != 0 { out.push((None, sub)); return Ok(out); }
        let key = sl(node, at + 16, kl)?;
        let units = sl(key, 66, *key.get(64).ok_or(Error::Corrupt)? as usize * 2)?;
        out.push((Some(Key {
            mft: le64(node, at) & 0xFFFF_FFFF_FFFF,
            name: units.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect(),
            ns: key[65],
            dir: r32(key, 56)? & 0x1000_0000 != 0,
        }), sub));
        at += el;
    }
}

impl Volume for Ntfs {
    fn format(&self) -> Format { Format::Ntfs }
    fn label(&self) -> String { self.label.clone() }
    fn read_only(&self) -> bool { true }
    /// Free space is the clear bits of $Bitmap, counted on demand rather than kept.
    fn statfs(&mut self, disk: &mut dyn Disk) -> Result<(u64, u64)> {
        let rec = self.record(disk, BITMAP)?;
        let a = self.data(disk, &rec, BITMAP)?.ok_or(Error::Corrupt)?;
        if a.unreadable() || a.size < self.clusters.div_ceil(8) { return Err(Error::Corrupt); }
        let (mut used, mut chunk, mut off) = (0u64, vec![0u8; 4096], 0u64);
        while off * 8 < self.clusters {
            let n = self.read_attr(disk, &a, off, &mut chunk)?;
            for (i, &byte) in chunk[..n].iter().enumerate() {
                let bit0 = (off + i as u64) * 8;
                if bit0 >= self.clusters { break; }
                let valid = (self.clusters - bit0).min(8) as u32;
                used += (byte as u32 & ((1u32 << valid) - 1)).count_ones() as u64;
            }
            off += n as u64;
        }
        let total = self.clusters * self.cluster;
        Ok((total, total - used.min(self.clusters) * self.cluster))
    }
    fn stat(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<Stat> {
        let n = self.resolve(disk, path)?;
        self.stat_of(disk, n)
    }
    /// Sizes come from each file's own record: the copies in the index go stale on Windows.
    fn list(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<Vec<Entry>> {
        let n = self.resolve(disk, path)?;
        let dir = self.open_dir(disk, n)?;
        let mut keys = Vec::new();
        let mut budget = dir.alloc.as_ref().map_or(0, |a| a.size / dir.block as u64).min(1 << 16);
        self.walk(disk, &dir, &dir.root, 16, 0, &mut budget, &mut keys)?;
        let mut out = Vec::new();
        for k in keys {
            if k.ns == 2 || k.name == [b'.' as u16] || (n == ROOT && k.name.first() == Some(&(b'$' as u16))) { continue; }
            let size = if k.dir { 0 } else { self.stat_of(disk, k.mft)?.size };
            out.push(Entry { name: String::from_utf16_lossy(&k.name), kind: if k.dir { Kind::Dir } else { Kind::File }, size });
        }
        Ok(out)
    }
    fn read(&mut self, disk: &mut dyn Disk, path: &[u8], offset: u64, buf: &mut [u8]) -> Result<usize> {
        let n = self.resolve(disk, path)?;
        let rec = self.record(disk, n)?;
        if r16(&rec, 0x16)? & 2 != 0 { return Err(Error::IsDir); }
        if let Some(si) = raw_attrs(&rec)?.iter().find(|r| r.ty == STANDARD_INFORMATION && !r.nonres) {
            if r32(si.value()?, 32)? & (FILE_REPARSE | FILE_ENCRYPTED) != 0 { return Err(Error::Unsupported); }
        }
        let Some(a) = self.data(disk, &rec, n)? else { return Ok(0) };
        if a.unreadable() { return Err(Error::Unsupported); }
        self.read_attr(disk, &a, offset, buf)
    }
}
