// SPDX-License-Identifier: GPL-3.0-only
//! exFAT, read and write: what SD cards and big USB sticks come formatted as.
//!
//! A file is a directory entry set (File 0x85, Stream Extension 0xC0, then File Name 0xC1
//! entries of 15 UTF-16 units each) naming its clusters: either one run marked NoFatChain,
//! where the FAT isn't consulted at all, or a FAT chain. Whether a cluster is in use is
//! the allocation bitmap's business alone; the FAT only links chains. Names compare
//! through the volume's own up-case table, as Windows does.
//!
//! The volume keeps geometry, the up-case pairs, a free-cluster count and a few FAT and
//! bitmap sectors; everything else is read again on each call.
use crate::{days_from_civil, le16, le32, le64, parts, read_bytes, split_last, write_bytes, Disk, Entry, Error, Format, Kind, Result, Stat, Volume, SECTOR};
use alloc::{string::String, vec, vec::Vec};

const EOC: u32 = 0xFFFF_FFFF;
/// 1980-01-01 00:00, the DOS epoch: a volume isn't handed a clock, so new entries get this.
const STAMP: u32 = 0x0021_0000;
const ATTR_DIR: u16 = 0x10;
const ATTR_ARCHIVE: u16 = 0x20;
/// The spec's cap on a directory, which also caps what one listing reads into memory.
const MAX_DIR: u64 = 256 << 20;

/// Where a file's data lives: a run from `first`, or a FAT chain from it. 0 = no clusters.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Ext { first: u32, contig: bool }

/// A file or directory as its entry set says, and where that set lives (the parent's
/// extent and the index of its first entry); the root has no set.
#[derive(Clone)]
struct Node { dir: bool, ext: Ext, size: u64, valid: u64, set: Vec<u8>, at: Option<(Ext, u64)> }

pub struct ExFat {
    /// log2 of the cluster size in bytes.
    cshift: u32,
    /// Byte offsets of the active FAT and of cluster 2.
    fat: u64,
    heap: u64,
    count: u32,
    root: Ext,
    root_len: u64,
    /// The allocation bitmap's clusters, in order.
    bitmap: Vec<u32>,
    free: u64,
    hint: u32,
    /// The up-case table's non-identity pairs, sorted; empty means ASCII only.
    upcase: Vec<(u16, u16)>,
    label: String,
    /// A few recently used FAT and bitmap sectors.
    cache: Vec<(u64, [u8; SECTOR])>,
    /// The last chain position looked up (first cluster, index, cluster): sequential
    /// reads of a chained file don't walk the chain from its start each time.
    cursor: Option<(u32, u64, u32)>,
}

impl ExFat {
    pub fn mount(disk: &mut dyn Disk) -> Result<Self> {
        let mut b = [0u8; SECTOR];
        disk.read(0, &mut b)?;
        if &b[3..11] != b"EXFAT   " || b[11..64].iter().any(|x| *x != 0) || b[510..512] != [0x55, 0xAA] { return Err(Error::Corrupt); }
        let (bps, spc) = (b[108] as u32, b[109] as u32);
        if !(9..=12).contains(&bps) || bps + spc > 25 { return Err(Error::Corrupt); }
        let (vol_len, fat_off, fat_len) = (le64(&b, 72), le32(&b, 80) as u64, le32(&b, 84) as u64);
        let (heap_off, count, root) = (le32(&b, 88) as u64, le32(&b, 92), le32(&b, 96));
        let (fats, active) = (b[110] as u64, (le16(&b, 106) & 1) as u64);
        if fats == 0 || fats > 2 || active >= fats || fat_off < 24 || count == 0 || count > 0xFFFF_FFF5
            || (fat_len << bps) < (count as u64 + 2) * 4 || heap_off < fat_off + fat_len * fats
            || heap_off + ((count as u64) << spc) > vol_len || vol_len > (disk.sectors() * SECTOR as u64) >> bps
        { return Err(Error::Corrupt); }
        let mut v = ExFat {
            cshift: bps + spc, fat: (fat_off + active * fat_len) << bps, heap: heap_off << bps, count,
            root: Ext { first: root, contig: false }, root_len: 0, bitmap: Vec::new(), free: 0, hint: 2,
            upcase: Vec::new(), label: String::new(), cache: Vec::new(), cursor: None,
        };
        v.root_len = v.chain_len(disk, root)? << v.cshift;
        if v.root_len > MAX_DIR { return Err(Error::Corrupt); }
        let mut data = vec![0u8; v.root_len as usize];
        v.read_ext(disk, v.root, 0, &mut data)?;
        let (mut bm, mut up) = (None, None);
        for e in data.chunks(32) {
            match e[0] {
                0 => break,
                // The first bitmap; a second one only exists for TexFAT's second FAT.
                0x81 if e[1] & 1 == 0 && bm.is_none() => bm = Some((le32(e, 20), le64(e, 24))),
                0x82 => up = Some((le32(e, 20), le64(e, 24))),
                0x83 => {
                    let n = (e[1] as usize).min(11);
                    v.label = decode(&(0..n).map(|i| le16(e, 2 + 2 * i)).collect::<Vec<_>>());
                }
                _ => {}
            }
        }
        let (first, len) = bm.ok_or(Error::Corrupt)?;
        if len < (count as u64).div_ceil(8) || len > MAX_DIR { return Err(Error::Corrupt); }
        for i in 0..len.div_ceil(1 << v.cshift) {
            let c = v.cluster_at(disk, Ext { first, contig: false }, i)?;
            v.bitmap.push(c);
        }
        // The table is at most one u16 per UTF-16 unit; a bad one falls back to ASCII.
        if let Some((first, len)) = up.filter(|u| u.1 >= 2 && u.1 <= 2 << 16) {
            let mut t = vec![0u8; len as usize];
            v.read_ext(disk, Ext { first, contig: false }, 0, &mut t)?;
            let (mut ch, mut i) = (0u32, 0);
            while i + 1 < t.len() && ch < 0x10000 {
                let u = le16(&t, i);
                // 0xFFFF n: the next n units map to themselves (the compressed form).
                if u == 0xFFFF && i + 3 < t.len() { ch += le16(&t, i + 2) as u32; i += 4; continue; }
                if u as u32 != ch { v.upcase.push((ch as u16, u)); }
                ch += 1;
                i += 2;
            }
        }
        let bytes = (count as u64).div_ceil(8);
        let mut used = 0u64;
        let mut s = [0u8; SECTOR];
        for at in (0..bytes).step_by(SECTOR) {
            let take = (bytes - at).min(SECTOR as u64) as usize;
            read_bytes(disk, v.bm_byte(at), &mut s[..take])?;
            for (k, byte) in s[..take].iter().enumerate() {
                let bits = (count as u64 - (at + k as u64) * 8).min(8);
                used += (*byte as u32 & ((1u32 << bits) - 1)).count_ones() as u64;
            }
        }
        v.free = count as u64 - used;
        Ok(v)
    }

    fn csize(&self) -> u64 { 1 << self.cshift }
    fn clusters(&self, bytes: u64) -> u64 { bytes.div_ceil(self.csize()) }
    fn ok(&self, c: u32) -> bool { c >= 2 && c - 2 < self.count }
    fn offset(&self, c: u32) -> u64 { self.heap + (((c - 2) as u64) << self.cshift) }
    /// Byte `b` of the allocation bitmap, on the disk.
    fn bm_byte(&self, b: u64) -> u64 { self.offset(self.bitmap[(b >> self.cshift) as usize]) + (b & (self.csize() - 1)) }

    fn sector(&mut self, disk: &mut dyn Disk, lba: u64) -> Result<usize> {
        if let Some(i) = self.cache.iter().position(|e| e.0 == lba) { return Ok(i); }
        let mut buf = [0u8; SECTOR];
        disk.read(lba, &mut buf)?;
        if self.cache.len() >= 8 { self.cache.remove(0); }
        self.cache.push((lba, buf));
        Ok(self.cache.len() - 1)
    }
    fn put_sector(&mut self, disk: &mut dyn Disk, lba: u64) -> Result<()> {
        match self.cache.iter().position(|e| e.0 == lba) { Some(i) => disk.write(lba, &self.cache[i].1), None => Ok(()) }
    }
    fn fat_get(&mut self, disk: &mut dyn Disk, c: u32) -> Result<u32> {
        let at = self.fat + c as u64 * 4;
        let i = self.sector(disk, at / SECTOR as u64)?;
        Ok(le32(&self.cache[i].1, (at % SECTOR as u64) as usize))
    }
    fn fat_set(&mut self, disk: &mut dyn Disk, c: u32, next: u32) -> Result<()> {
        let at = self.fat + c as u64 * 4;
        let (lba, w) = (at / SECTOR as u64, (at % SECTOR as u64) as usize);
        let i = self.sector(disk, lba)?;
        self.cache[i].1[w..w + 4].copy_from_slice(&next.to_le_bytes());
        disk.write(lba, &self.cache[i].1)
    }
    fn bit(&mut self, disk: &mut dyn Disk, c: u32) -> Result<bool> {
        let at = self.bm_byte((c - 2) as u64 / 8);
        let i = self.sector(disk, at / SECTOR as u64)?;
        Ok(self.cache[i].1[(at % SECTOR as u64) as usize] >> ((c - 2) % 8) & 1 != 0)
    }
    /// Marks `n` clusters from `c` used or free, writing each bitmap sector once.
    fn set_bits(&mut self, disk: &mut dyn Disk, c: u32, n: u64, on: bool) -> Result<()> {
        if !self.ok(c) || (c - 2) as u64 + n > self.count as u64 { return Err(Error::Corrupt); }
        let mut dirty = None;
        for k in 0..n {
            let bit = (c - 2) as u64 + k;
            let at = self.bm_byte(bit / 8);
            let lba = at / SECTOR as u64;
            if dirty != Some(lba) {
                if let Some(l) = dirty { self.put_sector(disk, l)?; }
                dirty = Some(lba);
            }
            let i = self.sector(disk, lba)?;
            let (byte, mask) = (&mut self.cache[i].1[(at % SECTOR as u64) as usize], 1u8 << (bit % 8));
            if (*byte & mask != 0) != on {
                *byte ^= mask;
                self.free = if on { self.free.saturating_sub(1) } else { self.free + 1 };
            }
        }
        if let Some(l) = dirty { self.put_sector(disk, l)?; }
        Ok(())
    }
    /// The first run of `n` free clusters at or after `from`, else from the start.
    fn find_run(&mut self, disk: &mut dyn Disk, n: u64, from: u32) -> Result<Option<u32>> {
        let end = self.count as u64 + 2;
        for start in [(from as u64).clamp(2, end), 2] {
            let mut run = 0;
            for c in start..end {
                if self.bit(disk, c as u32)? { run = 0; continue; }
                run += 1;
                if run == n { return Ok(Some((c + 1 - n) as u32)); }
            }
            if start == 2 { break; }
        }
        Ok(None)
    }

    fn chain_len(&mut self, disk: &mut dyn Disk, first: u32) -> Result<u64> {
        let (mut c, mut n) = (first, 1);
        loop {
            if !self.ok(c) { return Err(Error::Corrupt); }
            let next = self.fat_get(disk, c)?;
            if next >= 0xFFFF_FFF8 { return Ok(n); }
            n += 1;
            if n > self.count as u64 { return Err(Error::Corrupt); }
            c = next;
        }
    }
    /// The `idx`th cluster of an extent.
    fn cluster_at(&mut self, disk: &mut dyn Disk, ext: Ext, idx: u64) -> Result<u32> {
        if idx >= self.count as u64 { return Err(Error::Corrupt); }
        if ext.contig {
            let c = ext.first as u64 + idx;
            return if c <= u32::MAX as u64 && self.ok(c as u32) { Ok(c as u32) } else { Err(Error::Corrupt) };
        }
        let (mut i, mut c) = match self.cursor { Some((f, ci, cc)) if f == ext.first && ci <= idx => (ci, cc), _ => (0, ext.first) };
        while i < idx {
            if !self.ok(c) { return Err(Error::Corrupt); }
            c = self.fat_get(disk, c)?;
            i += 1;
        }
        if !self.ok(c) { return Err(Error::Corrupt); }
        self.cursor = Some((ext.first, idx, c));
        Ok(c)
    }
    /// The disk byte offset of byte `at` of an extent, and how many bytes from there on
    /// are contiguous on disk (up to `left`).
    fn span(&mut self, disk: &mut dyn Disk, ext: Ext, at: u64, left: usize) -> Result<(u64, usize)> {
        let within = at & (self.csize() - 1);
        let take = if ext.contig {
            self.cluster_at(disk, ext, (at + left as u64 - 1) >> self.cshift)?;
            left
        } else { left.min((self.csize() - within) as usize) };
        let c = self.cluster_at(disk, ext, at >> self.cshift)?;
        Ok((self.offset(c) + within, take))
    }
    fn read_ext(&mut self, disk: &mut dyn Disk, ext: Ext, off: u64, buf: &mut [u8]) -> Result<()> {
        let mut done = 0;
        while done < buf.len() {
            let (at, take) = self.span(disk, ext, off + done as u64, buf.len() - done)?;
            read_bytes(disk, at, &mut buf[done..done + take])?;
            done += take;
        }
        Ok(())
    }
    fn write_ext(&mut self, disk: &mut dyn Disk, ext: Ext, off: u64, data: &[u8]) -> Result<()> {
        let mut done = 0;
        while done < data.len() {
            let (at, take) = self.span(disk, ext, off + done as u64, data.len() - done)?;
            write_bytes(disk, at, &data[done..done + take])?;
            done += take;
        }
        Ok(())
    }
    fn zero(&mut self, disk: &mut dyn Disk, ext: Ext, mut off: u64, mut len: u64) -> Result<()> {
        let zeros = vec![0u8; len.min(64 << 10) as usize];
        while len > 0 {
            let take = len.min(zeros.len() as u64);
            self.write_ext(disk, ext, off, &zeros[..take as usize])?;
            off += take;
            len -= take;
        }
        Ok(())
    }

    /// Grows an extent from `have` to `want` clusters. A new extent takes a free run if
    /// there is one; a run grows in place while the clusters after it are free, and
    /// otherwise turns into a FAT chain (NoFatChain files have no FAT entries to extend).
    fn grow(&mut self, disk: &mut dyn Disk, ext: &mut Ext, have: u64, want: u64) -> Result<()> {
        if want <= have { return Ok(()); }
        let need = want - have;
        if need > self.free { return Err(Error::NoSpace); }
        self.cursor = None;
        let mut prev = None;
        if have == 0 {
            if let Some(c) = self.find_run(disk, need, self.hint)? {
                self.set_bits(disk, c, need, true)?;
                *ext = Ext { first: c, contig: true };
                self.hint = (c as u64 + need) as u32;
                return Ok(());
            }
            *ext = Ext { first: 0, contig: false };
        } else if ext.contig {
            let next = ext.first as u64 + have;
            if next + need <= self.count as u64 + 2 && self.run_free(disk, next as u32, need)? {
                return self.set_bits(disk, next as u32, need, true);
            }
            let last = (next - 1) as u32;
            self.cluster_at(disk, *ext, have - 1)?;
            for c in ext.first..last { self.fat_set(disk, c, c + 1)?; }
            self.fat_set(disk, last, EOC)?;
            ext.contig = false;
            prev = Some(last);
        } else {
            prev = Some(self.cluster_at(disk, *ext, have - 1)?);
        }
        for _ in 0..need {
            let c = self.find_run(disk, 1, prev.map_or(self.hint, |p| p + 1))?.ok_or(Error::NoSpace)?;
            self.set_bits(disk, c, 1, true)?;
            self.fat_set(disk, c, EOC)?;
            match prev { Some(p) => self.fat_set(disk, p, c)?, None => ext.first = c }
            prev = Some(c);
        }
        self.hint = prev.map_or(2, |p| p + 1);
        self.cursor = None;
        Ok(())
    }
    fn run_free(&mut self, disk: &mut dyn Disk, c: u32, n: u64) -> Result<bool> {
        for k in 0..n { if self.bit(disk, c + k as u32)? { return Ok(false); } }
        Ok(true)
    }
    /// Shrinks an extent from `have` to `want` clusters, freeing the rest.
    fn shrink(&mut self, disk: &mut dyn Disk, ext: &mut Ext, have: u64, want: u64) -> Result<()> {
        if want >= have { return Ok(()); }
        self.cursor = None;
        if ext.contig {
            self.cluster_at(disk, *ext, have - 1)?;
            self.set_bits(disk, ext.first + want as u32, have - want, false)?;
        } else {
            let mut c = if want == 0 { ext.first } else {
                let last = self.cluster_at(disk, *ext, want - 1)?;
                let next = self.fat_get(disk, last)?;
                self.fat_set(disk, last, EOC)?;
                next
            };
            for _ in want..have {
                if !self.ok(c) { return Err(Error::Corrupt); }
                let next = self.fat_get(disk, c)?;
                self.fat_set(disk, c, 0)?;
                self.set_bits(disk, c, 1, false)?;
                c = next;
            }
        }
        if want == 0 { *ext = Ext { first: 0, contig: false }; }
        self.cursor = None;
        Ok(())
    }

    fn up(&self, u: u16) -> u16 {
        if self.upcase.is_empty() { return if (b'a' as u16..=b'z' as u16).contains(&u) { u - 32 } else { u }; }
        match self.upcase.binary_search_by_key(&u, |p| p.0) { Ok(i) => self.upcase[i].1, Err(_) => u }
    }
    fn hash(&self, name: &[u16]) -> u16 {
        let mut h = 0u16;
        for &u in name {
            let u = self.up(u);
            for b in [u as u8, (u >> 8) as u8] { h = h.rotate_right(1).wrapping_add(b as u16); }
        }
        h
    }
    fn same(&self, a: &[u16], b: &[u16]) -> bool { a.len() == b.len() && a.iter().zip(b).all(|(x, y)| self.up(*x) == self.up(*y)) }

    fn root_node(&self) -> Node { Node { dir: true, ext: self.root, size: self.root_len, valid: self.root_len, set: Vec::new(), at: None } }
    fn dir_bytes(&mut self, disk: &mut dyn Disk, d: &Node) -> Result<Vec<u8>> {
        if !d.dir { return Err(Error::NotDir); }
        if d.size > MAX_DIR || d.size % 32 != 0 { return Err(Error::Corrupt); }
        let mut data = vec![0u8; d.size as usize];
        self.read_ext(disk, d.ext, 0, &mut data)?;
        Ok(data)
    }
    /// The entry sets of a directory that hold together; anything else is skipped.
    fn children(&mut self, disk: &mut dyn Disk, d: &Node) -> Result<Vec<Node>> {
        let data = self.dir_bytes(disk, d)?;
        let (n, mut i, mut out) = (data.len() / 32, 0, Vec::new());
        while i < n && data[i * 32] != 0 {
            let sc = data[i * 32 + 1] as usize;
            if data[i * 32] == 0x85 && sc >= 2 && i + sc < n {
                let set = &data[i * 32..(i + sc + 1) * 32];
                let names = (set[35] as usize).div_ceil(15);
                if set[32] == 0xC0 && names > 0 && names < sc + 1 && (2..2 + names).all(|k| set[k * 32] == 0xC1)
                    && (1..=sc).all(|k| set[k * 32] >= 0xC0)
                {
                    out.push(node(set.to_vec(), Some((d.ext, i as u64))));
                    i += sc + 1;
                    continue;
                }
            }
            i += 1;
        }
        Ok(out)
    }
    fn find(&mut self, disk: &mut dyn Disk, d: &Node, name: &[u16]) -> Result<Option<Node>> {
        let h = self.hash(name);
        Ok(self.children(disk, d)?.into_iter().find(|n| le16(&n.set, 36) == h && self.same(&name_of(&n.set), name)))
    }
    fn lookup(&mut self, disk: &mut dyn Disk, path: &[&[u8]]) -> Result<Node> {
        let mut n = self.root_node();
        for p in path { n = self.find(disk, &n, &name16(p)?)?.ok_or(Error::NotFound)?; }
        Ok(n)
    }
    /// A parent directory and a new name in it that isn't taken.
    fn fresh(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<(Node, Vec<u16>)> {
        let (pp, last) = split_last(path)?;
        let d = self.lookup(disk, &pp)?;
        if !d.dir { return Err(Error::NotDir); }
        let name = name16(last)?;
        if self.find(disk, &d, &name)?.is_some() { return Err(Error::Exists); }
        Ok((d, name))
    }

    fn new_set(&self, name: &[u16], attr: u16, ext: Ext, size: u64) -> Vec<u8> {
        let names = name.len().div_ceil(15);
        let mut s = vec![0u8; (2 + names) * 32];
        s[0] = 0x85;
        s[1] = (1 + names) as u8;
        s[4..6].copy_from_slice(&attr.to_le_bytes());
        for at in [8, 12, 16] { s[at..at + 4].copy_from_slice(&STAMP.to_le_bytes()); }
        s[32] = 0xC0;
        s[35] = name.len() as u8;
        s[36..38].copy_from_slice(&self.hash(name).to_le_bytes());
        for (i, u) in name.iter().enumerate() {
            let e = 64 + i / 15 * 32;
            s[e] = 0xC1;
            s[e + 2 + i % 15 * 2..e + 4 + i % 15 * 2].copy_from_slice(&u.to_le_bytes());
        }
        stream(&mut s, ext, size, size);
        s
    }
    fn put_set(&mut self, disk: &mut dyn Disk, dir: Ext, idx: u64, set: &[u8]) -> Result<()> {
        for (k, e) in set.chunks(32).enumerate() {
            let (at, _) = self.span(disk, dir, (idx + k as u64) * 32, 32)?;
            write_bytes(disk, at, e)?;
        }
        Ok(())
    }
    /// Marks a set's entries not in use.
    fn drop_set(&mut self, disk: &mut dyn Disk, n: &Node) -> Result<()> {
        let (dir, idx) = n.at.ok_or(Error::Name)?;
        let mut set = n.set.clone();
        for e in set.chunks_mut(32) { e[0] &= 0x7F; }
        self.put_set(disk, dir, idx, &set)
    }
    /// Writes a node's extent and sizes back to its stream entry (or the root's geometry).
    fn save(&mut self, disk: &mut dyn Disk, n: &mut Node) -> Result<()> {
        let Some((dir, idx)) = n.at else { self.root = n.ext; self.root_len = n.size; return Ok(()); };
        stream(&mut n.set, n.ext, n.size, n.valid);
        seal(&mut n.set);
        self.put_set(disk, dir, idx, &n.set[..64])
    }
    /// Places a set in the first run of free entries of `d`, growing it when none is long enough.
    fn insert(&mut self, disk: &mut dyn Disk, d: &mut Node, set: &mut [u8]) -> Result<()> {
        seal(set);
        let need = set.len() / 32;
        let data = self.dir_bytes(disk, d)?;
        let (mut start, mut run) = (0, 0);
        for i in 0..data.len() / 32 {
            if data[i * 32] & 0x80 != 0 { run = 0; continue; }
            if run == 0 { start = i; }
            run += 1;
            if run == need { break; }
        }
        if run == 0 { start = data.len() / 32; }
        if run < need {
            let more = self.clusters(((need - run) * 32) as u64);
            if d.size + (more << self.cshift) > MAX_DIR { return Err(Error::NoSpace); }
            let have = self.clusters(d.size);
            self.grow(disk, &mut d.ext, have, have + more)?;
            self.zero(disk, d.ext, d.size, more << self.cshift)?;
            d.size += more << self.cshift;
            d.valid = d.size;
            self.save(disk, d)?;
        }
        self.put_set(disk, d.ext, start as u64, set)
    }
}

impl Volume for ExFat {
    fn format(&self) -> Format { Format::ExFat }
    fn label(&self) -> String { self.label.clone() }
    fn read_only(&self) -> bool { false }
    fn statfs(&mut self, _: &mut dyn Disk) -> Result<(u64, u64)> { Ok(((self.count as u64) << self.cshift, self.free << self.cshift)) }
    fn stat(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<Stat> {
        let n = self.lookup(disk, &parts(path)?)?;
        Ok(Stat { kind: if n.dir { Kind::Dir } else { Kind::File }, size: n.size, mtime: mtime(&n.set) })
    }
    fn list(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<Vec<Entry>> {
        let d = self.lookup(disk, &parts(path)?)?;
        Ok(self.children(disk, &d)?.into_iter()
            .map(|n| Entry { name: decode(&name_of(&n.set)), kind: if n.dir { Kind::Dir } else { Kind::File }, size: n.size })
            .collect())
    }
    fn read(&mut self, disk: &mut dyn Disk, path: &[u8], off: u64, buf: &mut [u8]) -> Result<usize> {
        let n = self.lookup(disk, &parts(path)?)?;
        if n.dir { return Err(Error::IsDir); }
        if off >= n.size { return Ok(0); }
        let len = (buf.len() as u64).min(n.size - off) as usize;
        // Past ValidDataLength the clusters hold whatever was there before: zeros instead.
        let real = n.valid.saturating_sub(off).min(len as u64) as usize;
        self.read_ext(disk, n.ext, off, &mut buf[..real])?;
        buf[real..len].fill(0);
        Ok(len)
    }
    fn write(&mut self, disk: &mut dyn Disk, path: &[u8], off: u64, data: &[u8]) -> Result<usize> {
        let mut n = self.lookup(disk, &parts(path)?)?;
        if n.dir { return Err(Error::IsDir); }
        if data.is_empty() { return Ok(0); }
        let end = off.checked_add(data.len() as u64).ok_or(Error::TooBig)?;
        let (have, want) = (self.clusters(n.size), self.clusters(end));
        self.grow(disk, &mut n.ext, have, want)?;
        // Fill any gap below the write so ValidDataLength can cover it.
        if off > n.valid { self.zero(disk, n.ext, n.valid, off - n.valid)?; }
        self.write_ext(disk, n.ext, off, data)?;
        n.valid = n.valid.max(end);
        n.size = n.size.max(end);
        self.save(disk, &mut n)?;
        Ok(data.len())
    }
    fn create(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<()> {
        let (mut d, name) = self.fresh(disk, path)?;
        let mut set = self.new_set(&name, ATTR_ARCHIVE, Ext { first: 0, contig: false }, 0);
        self.insert(disk, &mut d, &mut set)
    }
    fn mkdir(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<()> {
        let (mut d, name) = self.fresh(disk, path)?;
        let mut ext = Ext { first: 0, contig: false };
        self.grow(disk, &mut ext, 0, 1)?;
        self.zero(disk, ext, 0, self.csize())?;
        let mut set = self.new_set(&name, ATTR_DIR, ext, self.csize());
        if let Err(e) = self.insert(disk, &mut d, &mut set) { self.shrink(disk, &mut ext, 1, 0)?; return Err(e); }
        Ok(())
    }
    fn remove(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<()> {
        let (pp, last) = split_last(path)?;
        let mut full = pp;
        full.push(last);
        let n = self.lookup(disk, &full)?;
        if n.dir && !self.children(disk, &n)?.is_empty() { return Err(Error::NotEmpty); }
        // The entries go first: a crash in between leaks clusters rather than cross-linking them.
        self.drop_set(disk, &n)?;
        let mut ext = n.ext;
        let have = self.clusters(n.size);
        self.shrink(disk, &mut ext, have, 0)
    }
    fn rename(&mut self, disk: &mut dyn Disk, from: &[u8], to: &[u8]) -> Result<()> {
        let old = self.lookup(disk, &parts(from)?)?;
        if old.at.is_none() { return Err(Error::Name); }
        let (tp, last) = split_last(to)?;
        // Walk to the new parent, refusing to move a directory into itself.
        let mut d = self.root_node();
        for p in &tp {
            d = self.find(disk, &d, &name16(p)?)?.ok_or(Error::NotFound)?;
            if old.dir && d.ext.first == old.ext.first { return Err(Error::Name); }
        }
        if !d.dir { return Err(Error::NotDir); }
        let name = name16(last)?;
        // `to` may only name `from` itself (a change of case).
        if let Some(t) = self.find(disk, &d, &name)? { if t.at != old.at { return Err(Error::Exists); } }
        let mut set = self.new_set(&name, le16(&old.set, 4), old.ext, old.size);
        set[4..32].copy_from_slice(&old.set[4..32]);
        set[33] = old.set[33];
        set[40..64].copy_from_slice(&old.set[40..64]);
        // The new set goes in before the old one goes, so a crash leaves two names, not none.
        self.insert(disk, &mut d, &mut set)?;
        self.drop_set(disk, &old)
    }
    fn truncate(&mut self, disk: &mut dyn Disk, path: &[u8], len: u64) -> Result<()> {
        let mut n = self.lookup(disk, &parts(path)?)?;
        if n.dir { return Err(Error::IsDir); }
        let (have, want) = (self.clusters(n.size), self.clusters(len));
        if len < n.size { self.shrink(disk, &mut n.ext, have, want)?; } else { self.grow(disk, &mut n.ext, have, want)?; }
        // Growing leaves ValidDataLength where it was: the new bytes read as zeros without writing them.
        n.size = len;
        n.valid = n.valid.min(len);
        self.save(disk, &mut n)
    }
}

fn node(set: Vec<u8>, at: Option<(Ext, u64)>) -> Node {
    let dir = le16(&set, 4) & ATTR_DIR != 0;
    let size = le64(&set, 56);
    let valid = if dir { size } else { le64(&set, 40).min(size) };
    Node { dir, ext: Ext { first: le32(&set, 52), contig: set[33] & 2 != 0 }, size, valid, set, at }
}
/// The stream entry's NoFatChain flag, first cluster and lengths.
fn stream(set: &mut [u8], ext: Ext, size: u64, valid: u64) {
    set[33] = if ext.first == 0 { 1 } else { 1 | (ext.contig as u8) << 1 };
    set[40..48].copy_from_slice(&valid.to_le_bytes());
    set[52..56].copy_from_slice(&ext.first.to_le_bytes());
    set[56..64].copy_from_slice(&size.to_le_bytes());
}
/// SetChecksum: every byte of the set but the checksum's own two.
fn seal(set: &mut [u8]) {
    let mut h = 0u16;
    for (i, b) in set.iter().enumerate() { if i != 2 && i != 3 { h = h.rotate_right(1).wrapping_add(*b as u16); } }
    set[2..4].copy_from_slice(&h.to_le_bytes());
}
fn name_of(set: &[u8]) -> Vec<u16> {
    set[64..].chunks(32).flat_map(|e| e[2..].chunks(2).map(|u| u16::from_le_bytes([u[0], u[1]]))).take(set[35] as usize).collect()
}
fn decode(units: &[u16]) -> String { char::decode_utf16(units.iter().copied()).map(|c| c.unwrap_or('\u{FFFD}')).collect() }
/// A UTF-8 path part as exFAT can hold it: 1 to 255 UTF-16 units, none of `"*/:<>?\|` or controls.
fn name16(p: &[u8]) -> Result<Vec<u16>> {
    let s = core::str::from_utf8(p).map_err(|_| Error::Name)?;
    if s.chars().any(|c| (c as u32) < 0x20 || "\"*/:<>?\\|".contains(c)) { return Err(Error::Name); }
    let u: Vec<u16> = s.encode_utf16().collect();
    if u.is_empty() || u.len() > 255 { return Err(Error::Name); }
    Ok(u)
}
/// LastModified as seconds since 1970 UTC: local time, less its UTC offset when one is recorded.
fn mtime(set: &[u8]) -> u64 {
    if set.len() < 64 { return 0; }
    let t = le32(set, 12);
    let (mon, day) = ((t >> 21) & 15, (t >> 16) & 31);
    if mon == 0 || mon > 12 || day == 0 { return 0; }
    let mut s = days_from_civil(1980 + (t >> 25) as i64, mon, day) * 86400
        + ((t >> 11) & 31) as i64 * 3600 + ((t >> 5) & 63) as i64 * 60 + (t & 31) as i64 * 2 + set[21] as i64 / 100;
    if set[23] & 0x80 != 0 { s -= ((set[23] << 1) as i8 >> 1) as i64 * 15 * 60; }
    s.max(0) as u64
}
