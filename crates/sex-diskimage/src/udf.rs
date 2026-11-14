// SPDX-License-Identifier: GPL-3.0-only
//! UDF 1.02 to 2.60 (ECMA-167 / OSTA UDF), read-only: DVDs, Blu-ray discs and
//! UDF hard-disk images.

use alloc::string::String;
use alloc::vec::Vec;

use crate::util::{self, le16, le32, le64, read_exact, read_vec};
use crate::{ByteSource, Entry, EntryKind, Error, Node, NodeRef, Result};

const MAX_DIR: u64 = 64 << 20;
const MAX_AED: usize = 4096;
const MAX_LINK: u64 = 64 * 1024;

const TAG_PVD: u16 = 1;
const TAG_AVDP: u16 = 2;
const TAG_VDP: u16 = 3;
const TAG_PD: u16 = 5;
const TAG_LVD: u16 = 6;
const TAG_TD: u16 = 8;
const TAG_FSD: u16 = 256;
const TAG_FID: u16 = 257;
const TAG_AED: u16 = 258;
const TAG_IE: u16 = 259;
const TAG_TE: u16 = 260;
const TAG_FE: u16 = 261;
const TAG_EFE: u16 = 266;

/// A descriptor tag whose checksum adds up.
fn tag_ok(b: &[u8], id: u16) -> bool {
    if b.len() < 16 || le16(b, 0) != id {
        return false;
    }
    let sum = b[..16].iter().enumerate().filter(|(i, _)| *i != 4).fold(0u8, |s, (_, v)| s.wrapping_add(*v));
    sum == b[4]
}

/// A Volume Recognition Sequence with an NSR descriptor at 32 KiB, and an anchor.
pub(crate) fn detect<S: ByteSource + ?Sized>(src: &S) -> Result<bool> {
    if !has_nsr(src)? {
        return Ok(false);
    }
    Ok(find_anchor(src)?.is_some())
}

fn has_nsr<S: ByteSource + ?Sized>(src: &S) -> Result<bool> {
    for step in [2048u64, 4096, 512] {
        let mut at = 32768u64;
        for _ in 0..64 {
            if at + 6 > src.len() {
                break;
            }
            let mut id = [0u8; 6];
            read_exact(src, at, &mut id)?;
            match &id[1..6] {
                b"NSR02" | b"NSR03" => return Ok(true),
                b"BEA01" | b"CD001" | b"CDW02" | b"BOOT2" | b"TEA01" => {}
                _ => break,
            }
            if &id[1..6] == b"TEA01" {
                break;
            }
            at += step.max(2048);
        }
    }
    Ok(false)
}

/// (sector size, anchor bytes)
fn find_anchor<S: ByteSource + ?Sized>(src: &S) -> Result<Option<(u64, Vec<u8>)>> {
    for ss in [2048u64, 512, 4096, 1024] {
        let n = src.len() / ss;
        for loc in [256, n.saturating_sub(1), n.saturating_sub(257)] {
            if loc < 256 || (loc + 1) * ss > src.len() {
                continue;
            }
            let b = read_vec(src, loc * ss, 512)?;
            if tag_ok(&b, TAG_AVDP) && le32(&b, 12) as u64 == loc {
                return Ok(Some((ss, b)));
            }
        }
    }
    Ok(None)
}

#[derive(Debug, Clone)]
enum Map {
    /// Physical (type 1) or sparable partition: first sector of the partition.
    Phys { start: u64 },
    /// Metadata partition: the metadata file's runs (absolute byte offsets) in
    /// order of file offset; `u64::MAX` runs are unrecorded.
    Meta { runs: Vec<(u64, u64)> },
}

#[derive(Clone, Copy, Debug)]
struct LongAd {
    lbn: u32,
    part: u16,
}

fn long_ad(b: &[u8], at: usize) -> LongAd {
    LongAd { lbn: le32(b, at + 4), part: le16(b, at + 8) }
}

/// What a file entry says about a file.
struct Info {
    file_type: u8,
    size: u64,
    mtime: Option<i64>,
    mode: Option<u32>,
    data: Data,
}

enum Data {
    Embedded(Vec<u8>),
    /// (absolute byte offset or `u64::MAX` for zeros, length)
    Runs(Vec<(u64, u64)>),
}

/// A UDF volume.
pub struct Udf<S> {
    src: S,
    /// Logical block size (= sector size on every real disc).
    lbs: u64,
    maps: Vec<Map>,
    label: String,
    root: LongAd,
    revision: u16,
}

impl<S: ByteSource> Udf<S> {
    pub fn open(src: S) -> Result<Self> {
        let (ss, avdp) = find_anchor(&src)?.ok_or(Error::NotRecognized)?;
        let mut parts: Vec<(u16, u64)> = Vec::new(); // (partition number, start sector)
        let mut lvd: Option<Vec<u8>> = None;
        let mut pvd_label: Option<String> = None;
        // Main sequence, then the reserve one if the main one is unreadable.
        for ext_at in [16usize, 24] {
            let mut len = le32(&avdp, ext_at) as u64;
            let mut loc = le32(&avdp, ext_at + 4) as u64;
            let mut hops = 0;
            'seq: while len >= ss && hops < 8 {
                hops += 1;
                let mut next = None;
                for i in 0..(len / ss).min(256) {
                    let Ok(d) = read_vec(&src, (loc + i) * ss, ss as usize) else { break 'seq };
                    let id = le16(&d, 0);
                    if !tag_ok(&d, id) {
                        break 'seq;
                    }
                    match id {
                        TAG_PVD if pvd_label.is_none() => pvd_label = Some(dstring(&d[24..56])),
                        TAG_PD => {
                            let num = le16(&d, 22);
                            if !parts.iter().any(|p| p.0 == num) {
                                parts.push((num, le32(&d, 188) as u64));
                            }
                        }
                        TAG_LVD if lvd.is_none() => lvd = Some(d),
                        TAG_VDP => {
                            next = Some(le32(&d, 20) as u64);
                            len = le32(&d, 16) as u64;
                            break;
                        }
                        TAG_TD => break 'seq,
                        _ => {}
                    }
                }
                match next {
                    Some(n) => loc = n,
                    None => break,
                }
            }
            if lvd.is_some() && !parts.is_empty() {
                break;
            }
        }
        let lvd = lvd.ok_or(Error::Corrupt("no UDF logical volume descriptor"))?;
        if parts.is_empty() {
            return Err(Error::Corrupt("no UDF partition descriptor"));
        }
        let lbs = le32(&lvd, 212) as u64;
        if !lbs.is_power_of_two() || !(512..=65536).contains(&lbs) {
            return Err(Error::Corrupt("bad UDF block size"));
        }
        if lbs != ss {
            return Err(Error::Unsupported("UDF block size differs from sector size"));
        }
        let revision = le16(&lvd, 216 + 24);
        let nmaps = le32(&lvd, 268) as usize;
        let table_len = le32(&lvd, 264) as usize;
        let table = lvd.get(440..440 + table_len.min(lvd.len().saturating_sub(440))).unwrap_or(&[]);
        let start_of = |num: u16| parts.iter().find(|p| p.0 == num).map(|p| p.1);
        // Metadata maps need the physical maps to be resolvable first: (index, phys ref, file lbn, mirror lbn).
        let mut pending_meta: Vec<(usize, u16, u32, u32)> = Vec::new();
        let mut maps: Vec<Map> = Vec::new();
        let mut p = 0;
        for _ in 0..nmaps.min(64) {
            if p + 2 > table.len() {
                break;
            }
            let ty = table[p];
            let ml = table[p + 1] as usize;
            if ml < 6 || p + ml > table.len() {
                return Err(Error::Corrupt("bad UDF partition map"));
            }
            let m = &table[p..p + ml];
            match ty {
                1 => maps.push(Map::Phys { start: start_of(le16(m, 4)).ok_or(Error::Corrupt("partition map names a missing partition"))? }),
                2 => {
                    let ident = &m[5..28];
                    let num = le16(m, 38);
                    let start = start_of(num).ok_or(Error::Corrupt("partition map names a missing partition"))?;
                    if ident.starts_with(b"*UDF Metadata Partition") {
                        pending_meta.push((maps.len(), num, le32(m, 40), le32(m, 44)));
                        maps.push(Map::Phys { start }); // placeholder
                    } else if ident.starts_with(b"*UDF Sparable Partition") {
                        // Pressed and finalised media: sparing tables are empty, read in place.
                        maps.push(Map::Phys { start });
                    } else if ident.starts_with(b"*UDF Virtual Partition") {
                        return Err(Error::Unsupported("UDF virtual partition (CD-R/DVD-R written incrementally)"));
                    } else {
                        return Err(Error::Unsupported("unknown UDF partition map"));
                    }
                }
                _ => return Err(Error::Corrupt("bad UDF partition map")),
            }
            p += ml;
        }
        if maps.is_empty() {
            return Err(Error::Corrupt("UDF volume has no partition maps"));
        }
        let mut udf = Udf { src, lbs, maps, label: String::new(), root: LongAd { lbn: 0, part: 0 }, revision };
        for (idx, num, file, mirror) in pending_meta {
            // The physical map that holds the metadata file is the type-1 map of the same partition.
            let start = start_of(num).unwrap();
            let phys_ref = udf.maps.iter().position(|m| matches!(m, Map::Phys { start: s } if *s == start)).unwrap_or(idx) as u16;
            let mut runs = None;
            for lbn in [file, mirror] {
                if let Ok(info) = udf.info(LongAd { lbn, part: phys_ref }) {
                    if let Data::Runs(r) = info.data {
                        runs = Some(r);
                        break;
                    }
                }
            }
            udf.maps[idx] = Map::Meta { runs: runs.ok_or(Error::Corrupt("unreadable UDF metadata file"))? };
        }
        let fsd_ad = long_ad(&lvd, 248);
        let fsd = udf.read_block(fsd_ad.part, fsd_ad.lbn)?;
        if !tag_ok(&fsd, TAG_FSD) {
            return Err(Error::Corrupt("no UDF file set descriptor"));
        }
        udf.root = long_ad(&fsd, 400);
        let lv = dstring(&lvd[84..212]);
        udf.label = if lv.is_empty() { pvd_label.unwrap_or_default() } else { lv };
        Ok(udf)
    }

    pub fn label(&self) -> &str {
        &self.label
    }
    /// The UDF revision the volume claims, as BCD (`0x0250` = 2.50).
    pub fn revision(&self) -> u16 {
        self.revision
    }
    pub fn root(&self) -> Node {
        Node(NodeRef::Udf { part: self.root.part, lbn: self.root.lbn })
    }
    pub fn source(&self) -> &S {
        &self.src
    }

    /// Absolute byte offset of a logical block.
    fn addr(&self, part: u16, lbn: u32) -> Result<u64> {
        match self.maps.get(part as usize).ok_or(Error::Corrupt("bad UDF partition reference"))? {
            Map::Phys { start } => Ok((start + lbn as u64) * self.lbs),
            Map::Meta { runs } => {
                let mut off = lbn as u64 * self.lbs;
                for &(at, len) in runs {
                    if off < len {
                        if at == u64::MAX {
                            return Err(Error::Corrupt("UDF metadata block not recorded"));
                        }
                        return Ok(at + off);
                    }
                    off -= len;
                }
                Err(Error::Corrupt("UDF metadata block out of range"))
            }
        }
    }

    fn read_block(&self, part: u16, lbn: u32) -> Result<Vec<u8>> {
        read_vec(&self.src, self.addr(part, lbn)?, self.lbs as usize)
    }

    fn info(&self, icb: LongAd) -> Result<Info> {
        let mut icb = icb;
        let mut fe = self.read_block(icb.part, icb.lbn)?;
        for _ in 0..8 {
            if tag_ok(&fe, TAG_IE) {
                icb = long_ad(&fe, 36);
                fe = self.read_block(icb.part, icb.lbn)?;
            } else {
                break;
            }
        }
        let ext = if tag_ok(&fe, TAG_FE) {
            false
        } else if tag_ok(&fe, TAG_EFE) {
            true
        } else if tag_ok(&fe, TAG_TE) {
            return Err(Error::Corrupt("UDF terminal entry where a file was expected"));
        } else {
            return Err(Error::Corrupt("bad UDF file entry"));
        };
        let file_type = fe[16 + 11];
        let icb_flags = le16(&fe, 16 + 18);
        let size = le64(&fe, 56);
        let mtime = udf_time(&fe[if ext { 92 } else { 84 }..]);
        let perm = le32(&fe, 44);
        let mode = Some(((perm >> 10) & 7) << 6 | ((perm >> 5) & 7) << 3 | (perm & 7));
        let (l_ea, l_ad, base) = if ext { (le32(&fe, 208), le32(&fe, 212), 216usize) } else { (le32(&fe, 168), le32(&fe, 172), 176usize) };
        let ad_at = base + l_ea as usize;
        let ad_end = ad_at.checked_add(l_ad as usize).filter(|e| *e <= fe.len()).ok_or(Error::Corrupt("UDF allocation descriptors overflow"))?;
        let ads = &fe[ad_at..ad_end];
        let data = match icb_flags & 7 {
            3 => Data::Embedded(ads[..(size as usize).min(ads.len())].to_vec()),
            kind @ 0..=2 => Data::Runs(self.runs(ads, kind, icb.part, size)?),
            _ => return Err(Error::Corrupt("bad UDF allocation type")),
        };
        Ok(Info { file_type, size, mtime, mode, data })
    }

    fn runs(&self, first: &[u8], kind: u16, part: u16, size: u64) -> Result<Vec<(u64, u64)>> {
        let ad_len = match kind {
            0 => 8,
            1 => 16,
            _ => 20,
        };
        let mut out: Vec<(u64, u64)> = Vec::new();
        let mut total = 0u64;
        let mut buf = first.to_vec();
        for _ in 0..MAX_AED {
            let mut next: Option<(u16, u32)> = None;
            for d in buf.chunks_exact(ad_len) {
                let raw = le32(d, 0);
                let len = (raw & 0x3FFF_FFFF) as u64;
                let ty = raw >> 30;
                if len == 0 {
                    break;
                }
                let (lbn, p) = match kind {
                    0 => (le32(d, 4), part),
                    1 => (le32(d, 4), le16(d, 8)),
                    _ => (le32(d, 12), le16(d, 16)),
                };
                if ty == 3 {
                    next = Some((p, lbn));
                    break;
                }
                if total >= size {
                    continue;
                }
                let take = len.min(size - total);
                let at = if ty == 0 { self.addr(p, lbn)? } else { u64::MAX };
                match out.last_mut() {
                    Some(last) if last.0 != u64::MAX && at != u64::MAX && last.0 + last.1 == at => last.1 += take,
                    _ => out.push((at, take)),
                }
                total += take;
            }
            let Some((p, lbn)) = next else { break };
            let aed = self.read_block(p, lbn)?;
            if !tag_ok(&aed, TAG_AED) {
                return Err(Error::Corrupt("bad UDF allocation extent"));
            }
            let l = (le32(&aed, 20) as usize).min(aed.len() - 24);
            buf = aed[24..24 + l].to_vec();
        }
        if total < size {
            out.push((u64::MAX, size - total)); // short allocation: the rest reads as zeros
        }
        Ok(out)
    }

    fn icb(node: &Node) -> Result<LongAd> {
        match node.0 {
            NodeRef::Udf { part, lbn } => Ok(LongAd { lbn, part }),
            _ => Err(Error::NotAFile),
        }
    }

    pub fn file_len(&self, file: &Node) -> Result<u64> {
        let i = self.info(Self::icb(file)?)?;
        if i.file_type == 4 {
            return Err(Error::NotAFile);
        }
        Ok(i.size)
    }

    pub fn read(&self, file: &Node, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let i = self.info(Self::icb(file)?)?;
        if i.file_type == 4 {
            return Err(Error::NotAFile);
        }
        self.read_info(&i, offset, buf)
    }

    fn read_info(&self, i: &Info, offset: u64, buf: &mut [u8]) -> Result<usize> {
        match &i.data {
            Data::Embedded(d) => {
                if offset >= d.len() as u64 {
                    return Ok(0);
                }
                let s = &d[offset as usize..];
                let n = s.len().min(buf.len());
                buf[..n].copy_from_slice(&s[..n]);
                Ok(n)
            }
            Data::Runs(r) => util::read_runs(&self.src, r, offset, buf),
        }
    }

    fn whole(&self, i: &Info, limit: u64) -> Result<Vec<u8>> {
        util::read_to_vec(i.size, limit, |o, b| self.read_info(i, o, b))
    }

    pub fn list_dir(&self, dir: &Node) -> Result<Vec<Entry>> {
        let di = self.info(Self::icb(dir)?)?;
        if di.file_type != 4 {
            return Err(Error::NotADirectory);
        }
        let data = self.whole(&di, MAX_DIR)?;
        let mut out = Vec::new();
        let mut p = 0usize;
        while p + 38 <= data.len() {
            let f = &data[p..];
            if !tag_ok(f, TAG_FID) {
                return Err(Error::Corrupt("bad UDF file identifier"));
            }
            let chars = f[18];
            let l_fi = f[19] as usize;
            let icb = long_ad(f, 20);
            let l_iu = le16(f, 36) as usize;
            let total = (38 + l_iu + l_fi + 3) & !3;
            if p + 38 + l_iu + l_fi > data.len() {
                return Err(Error::Corrupt("UDF file identifier past folder end"));
            }
            p += total;
            if chars & (4 | 8) != 0 || l_fi == 0 {
                continue; // deleted, or the parent entry
            }
            let name = cs0(&f[38 + l_iu..38 + l_iu + l_fi]);
            let node = Node(NodeRef::Udf { part: icb.part, lbn: icb.lbn });
            let hidden = chars & 1 != 0;
            let e = match self.info(icb) {
                Ok(i) => {
                    let kind = match i.file_type {
                        4 => EntryKind::Dir,
                        12 => EntryKind::Symlink(symlink(&self.whole(&i, MAX_LINK)?)),
                        _ if chars & 2 != 0 => EntryKind::Dir,
                        _ => EntryKind::File,
                    };
                    let size = if kind == EntryKind::File { i.size } else { 0 };
                    Entry { name, kind, size, mtime: i.mtime, mode: i.mode, hidden, synthetic: false, node }
                }
                Err(_) => continue, // an entry we cannot read is left out, not fatal
            };
            out.push(e);
        }
        Ok(out)
    }
}

/// OSTA Compressed Unicode (CS0): first byte 8 = one byte per char, 16 = UTF-16BE.
fn cs0(b: &[u8]) -> String {
    match b.first() {
        Some(8) | Some(254) => b[1..].iter().map(|&c| c as char).collect(),
        Some(16) | Some(255) => util::utf16be(&b[1..]),
        _ => String::new(),
    }
}

/// A dstring field: CS0 text whose used length is the field's last byte.
fn dstring(f: &[u8]) -> String {
    let Some(&n) = f.last() else { return String::new() };
    let n = (n as usize).min(f.len() - 1);
    cs0(&f[..n]).trim_end_matches(['\0', ' ']).into()
}

/// ECMA-167 path components to `a/b/../c` text.
fn symlink(b: &[u8]) -> String {
    let mut out = String::new();
    let mut p = 0;
    while p + 4 <= b.len() {
        let ty = b[p];
        let l = b[p + 1] as usize;
        let id = b.get(p + 4..p + 4 + l).unwrap_or(&[]);
        p += 4 + l;
        let part: String = match ty {
            1 | 2 => {
                out.clear();
                out.push('/');
                continue;
            }
            3 => "..".into(),
            4 => ".".into(),
            5 => cs0(id),
            _ => continue,
        };
        if !out.is_empty() && !out.ends_with('/') {
            out.push('/');
        }
        out.push_str(&part);
    }
    out
}

fn udf_time(b: &[u8]) -> Option<i64> {
    if b.len() < 12 {
        return None;
    }
    let tt = le16(b, 0);
    let mut tz = (tt & 0x0FFF) as i64;
    if tz >= 0x800 {
        tz -= 0x1000;
    }
    if tz == -2047 || tt >> 12 != 1 {
        tz = 0;
    }
    let year = le16(b, 2) as i16 as i64;
    if year == 0 {
        return None;
    }
    util::unix_time(year, b[4] as u32, b[5] as u32, b[6] as u32, b[7] as u32, b[8] as u32, tz)
}
