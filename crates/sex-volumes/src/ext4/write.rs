// SPDX-License-Identifier: GPL-3.0-only
//! Writing ext4: files with extent trees, directories, rename, truncate and remove, so a
//! disk SexOS wrote passes Linux's `e2fsck -fn` and mounts there as it would after Linux.
//!
//! Only volumes we can keep whole are written: extents on, no journal waiting to be
//! replayed, and no read-only-compatible feature we don't maintain (quotas, bigalloc,
//! orphan file in use, the old crc16 group checksums without metadata_csum). The journal
//! is left alone and stays clean; every change is written straight to its place, and
//! group descriptors and the superblock follow at the end of each call, so a volume is
//! consistent whenever a call has returned and `sync` flushed it.
//!
//! With metadata_csum, inodes, group descriptors, bitmaps, directory blocks and extent
//! blocks get their crc32c. Directories are written linearly: an htree (indexed)
//! directory can lose entries but new names go only into plain ones. Block groups whose
//! bitmaps were never initialised (`BLOCK_UNINIT`, `INODE_UNINIT`) are not allocated from.
use super::*;
use crate::{split_last, write_bytes};

const INCOMPAT_RECOVER: u32 = 0x4;
const INCOMPAT_EXTENTS: u32 = 0x40;
const INCOMPAT_CSUM_SEED: u32 = 0x2000;
/// sparse_super, large_file, huge_file, dir_nlink, extra_isize, metadata_csum.
const RO_COMPAT_WRITE: u32 = 0x1 | 0x2 | 0x8 | 0x20 | 0x40 | 0x400;
const RO_COMPAT_METADATA_CSUM: u32 = 0x400;
const FL_INDEX: u32 = 0x1000;
const FL_CASEFOLD: u32 = 0x4000_0000;
const BG_INODE_UNINIT: u16 = 1;
const BG_BLOCK_UNINIT: u16 = 2;
/// Longest extent ext4 allows (initialised; unwritten ones are one shorter).
const EXT_MAX: u64 = 32768;

/// What writing needs beyond what reading keeps.
pub(super) struct State {
    pub on: bool,
    csum: bool,
    seed: u32,
    extra_isize: u16,
    first_ino: u32,
    free_inodes: u32,
    pub now: u64,
    /// Every group descriptor, loaded at the first write.
    descs: Vec<u8>,
    dirty: Vec<u64>,
    generation: u32,
}

impl State {
    pub fn from_super(sb: &[u8], bs: u64, incompat: u32, ro_compat: u32, inode_size: usize) -> State {
        let csum = ro_compat & RO_COMPAT_METADATA_CSUM != 0;
        let seed = if incompat & INCOMPAT_CSUM_SEED != 0 { le32(sb, 0x270) } else { crc32c(!0, &sb[0x68..0x78]) };
        let on = incompat & INCOMPAT_EXTENTS != 0 && incompat & INCOMPAT_RECOVER == 0 && ro_compat & !RO_COMPAT_WRITE == 0
            && incompat & INCOMPAT_FILETYPE != 0 && bs <= 32768 && le32(sb, 0x4C) >= 1 && le16(sb, 0x3A) & 2 == 0;
        let want = if inode_size > 128 { le16(sb, 0x15E).max(32).min((inode_size - 128) as u16) & !3 } else { 0 };
        State { on, csum, seed, extra_isize: want, first_ino: le32(sb, 0x54).max(11), free_inodes: le32(sb, 0x10), now: 0, descs: Vec::new(), dirty: Vec::new(),
                generation: le32(sb, 0x68) ^ le32(sb, 0x6C) }
    }
}

/// crc32c as ext4 uses it: the raw update, no inversion before or after.
const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 { c = if c & 1 != 0 { (c >> 1) ^ 0x82F6_3B78 } else { c >> 1 }; k += 1; }
        t[i] = c;
        i += 1;
    }
    t
}
static CRC: [u32; 256] = crc_table();
pub(crate) fn crc32c(mut crc: u32, data: &[u8]) -> u32 {
    for b in data { crc = CRC[((crc ^ *b as u32) & 0xFF) as usize] ^ (crc >> 8); }
    crc
}

fn put16(b: &mut [u8], at: usize, v: u16) { b[at..at + 2].copy_from_slice(&v.to_le_bytes()); }
fn put32(b: &mut [u8], at: usize, v: u32) { b[at..at + 4].copy_from_slice(&v.to_le_bytes()); }

/// One extent: `n` blocks from logical `l` at physical `p`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ext { l: u64, n: u64, p: u64, unwritten: bool }

fn covering(list: &[Ext], l: u64) -> Option<Ext> { list.iter().copied().find(|e| e.l <= l && l < e.l + e.n) }
fn next_start(list: &[Ext], l: u64) -> Option<u64> { list.iter().filter(|e| e.l > l).map(|e| e.l).min() }
/// Maps `n` blocks from `l` to `p`, written, replacing whatever covered them.
fn put(list: &mut Vec<Ext>, l: u64, n: u64, p: u64) {
    let mut out = Vec::with_capacity(list.len() + 2);
    for e in list.iter().copied() {
        if e.l + e.n <= l || e.l >= l + n { out.push(e); continue; }
        if e.l < l { out.push(Ext { n: l - e.l, ..e }); }
        if e.l + e.n > l + n { let cut = l + n - e.l; out.push(Ext { l: l + n, n: e.n - cut, p: e.p + cut, unwritten: e.unwritten }); }
    }
    out.push(Ext { l, n, p, unwritten: false });
    out.sort_by_key(|e| e.l);
    *list = normalise(out);
}
/// Joins neighbours that continue each other and splits extents too long for the format.
fn normalise(list: Vec<Ext>) -> Vec<Ext> {
    let mut out: Vec<Ext> = Vec::with_capacity(list.len());
    for e in list {
        if let Some(last) = out.last_mut() {
            if last.l + last.n == e.l && last.p + last.n == e.p && last.unwritten == e.unwritten { last.n += e.n; continue; }
        }
        out.push(e);
    }
    let mut split = Vec::with_capacity(out.len());
    for mut e in out {
        let max = if e.unwritten { EXT_MAX - 1 } else { EXT_MAX };
        while e.n > max { split.push(Ext { n: max, ..e }); e = Ext { l: e.l + max, n: e.n - max, p: e.p + max, unwritten: e.unwritten }; }
        split.push(e);
    }
    split
}

fn file_type(mode: u16) -> u8 {
    match mode & S_IFMT { S_IFREG => 1, S_IFDIR => 2, 0x2000 => 3, 0x6000 => 4, 0x1000 => 5, 0xC000 => 6, S_IFLNK => 7, _ => 0 }
}
fn rec_for(name_len: usize) -> usize { (8 + name_len + 3) & !3 }

impl Ext4 {
    fn ready(&mut self, disk: &mut dyn Disk) -> Result<()> {
        if !self.w.on { return Err(Error::ReadOnly); }
        if self.w.descs.is_empty() {
            let per = self.bs / self.desc_size as u64;
            let mut all = Vec::with_capacity(self.groups as usize * self.desc_size);
            let mut g = 0;
            while g < self.groups {
                let b = self.block(disk, self.desc_block(g))?;
                let n = per.min(self.groups - g) as usize;
                all.extend_from_slice(&b[..n * self.desc_size]);
                g += per;
            }
            self.w.descs = all;
        }
        Ok(())
    }
    fn desc(&self, g: u64) -> &[u8] { &self.w.descs[g as usize * self.desc_size..(g as usize + 1) * self.desc_size] }
    fn desc_mut(&mut self, g: u64) -> &mut [u8] {
        if !self.w.dirty.contains(&g) { self.w.dirty.push(g); }
        let ds = self.desc_size;
        &mut self.w.descs[g as usize * ds..(g as usize + 1) * ds]
    }
    /// A descriptor field split in a low half at `lo` and, in 64-byte descriptors, a high
    /// half at `hi`; `half` is 2 or 4 bytes.
    fn dget(&self, g: u64, lo: usize, hi: usize, half: usize) -> u64 {
        let d = self.desc(g);
        let wide = self.desc_size >= 64;
        if half == 4 { le32(d, lo) as u64 | if wide { (le32(d, hi) as u64) << 32 } else { 0 } }
        else { le16(d, lo) as u64 | if wide { (le16(d, hi) as u64) << 16 } else { 0 } }
    }
    fn dset(&mut self, g: u64, lo: usize, hi: usize, v: u64) {
        let wide = self.desc_size >= 64;
        let d = self.desc_mut(g);
        put16(d, lo, v as u16);
        if wide { put16(d, hi, (v >> 16) as u16); }
    }
    fn block_bitmap(&self, g: u64) -> u64 { self.dget(g, 0x00, 0x20, 4) }
    fn inode_bitmap(&self, g: u64) -> u64 { self.dget(g, 0x04, 0x24, 4) }
    fn group_blocks(&self, g: u64) -> u64 { self.blocks_per_group.min(self.blocks - self.first_block - g * self.blocks_per_group) }
    fn bitmap_csum(&mut self, g: u64, bitmap: &[u8], inode: bool) {
        if !self.w.csum { return; }
        let bytes = if inode { self.inodes_per_group as usize / 8 } else { self.blocks_per_group as usize / 8 };
        let c = crc32c(self.w.seed, &bitmap[..bytes]);
        let (lo, hi) = if inode { (0x1A, 0x3A) } else { (0x18, 0x38) };
        let wide = self.desc_size >= 64;
        let d = self.desc_mut(g);
        put16(d, lo, c as u16);
        if wide { put16(d, hi, (c >> 16) as u16); }
    }

    /// Up to `want` free blocks in a row, as near `goal` as can be: (first, count).
    fn alloc_blocks(&mut self, disk: &mut dyn Disk, goal: u64, want: u64) -> Result<(u64, u64)> {
        let goal = goal.clamp(self.first_block, self.blocks - 1);
        let start_group = (goal - self.first_block) / self.blocks_per_group;
        for k in 0..self.groups {
            let g = (start_group + k) % self.groups;
            if self.dget(g, 0x0C, 0x2C, 2) == 0 || le16(self.desc(g), 0x12) & BG_BLOCK_UNINIT != 0 { continue; }
            let at = self.block_bitmap(g);
            let mut bitmap = self.block(disk, at)?;
            let count = self.group_blocks(g);
            let free = |b: &[u8], i: u64| b[(i / 8) as usize] & (1 << (i % 8)) == 0;
            let from = if k == 0 { goal - self.first_block - g * self.blocks_per_group } else { 0 };
            let first = (from..count).find(|&i| free(&bitmap, i)).or_else(|| (0..from).find(|&i| free(&bitmap, i)));
            let Some(first) = first else { continue };
            let mut n = 0;
            while n < want && first + n < count && free(&bitmap, first + n) { bitmap[((first + n) / 8) as usize] |= 1 << ((first + n) % 8); n += 1; }
            write_bytes(disk, at * self.bs, &bitmap)?;
            self.bitmap_csum(g, &bitmap, false);
            let left = self.dget(g, 0x0C, 0x2C, 2) - n;
            self.dset(g, 0x0C, 0x2C, left);
            self.free -= n;
            return Ok((self.first_block + g * self.blocks_per_group + first, n));
        }
        Err(Error::NoSpace)
    }
    fn free_blocks(&mut self, disk: &mut dyn Disk, mut start: u64, mut n: u64) -> Result<()> {
        while n > 0 {
            if start < self.first_block || start >= self.blocks { return Err(Error::Corrupt); }
            let g = (start - self.first_block) / self.blocks_per_group;
            let i = start - self.first_block - g * self.blocks_per_group;
            let take = n.min(self.group_blocks(g) - i);
            let at = self.block_bitmap(g);
            let mut bitmap = self.block(disk, at)?;
            for k in i..i + take { bitmap[(k / 8) as usize] &= !(1 << (k % 8)); }
            write_bytes(disk, at * self.bs, &bitmap)?;
            self.bitmap_csum(g, &bitmap, false);
            let left = self.dget(g, 0x0C, 0x2C, 2) + take;
            self.dset(g, 0x0C, 0x2C, left);
            self.free += take;
            start += take;
            n -= take;
        }
        Ok(())
    }
    fn alloc_inode(&mut self, disk: &mut dyn Disk, near: u64, dir: bool) -> Result<u32> {
        let ipg = self.inodes_per_group as u64;
        for k in 0..self.groups {
            let g = (near + k) % self.groups;
            if self.dget(g, 0x0E, 0x2E, 2) == 0 || le16(self.desc(g), 0x12) & BG_INODE_UNINIT != 0 { continue; }
            let at = self.inode_bitmap(g);
            let mut bitmap = self.block(disk, at)?;
            let lowest = if g == 0 { self.w.first_ino as u64 - 1 } else { 0 };
            let Some(i) = (lowest..ipg).find(|&i| bitmap[(i / 8) as usize] & (1 << (i % 8)) == 0) else { continue };
            bitmap[(i / 8) as usize] |= 1 << (i % 8);
            write_bytes(disk, at * self.bs, &bitmap)?;
            self.bitmap_csum(g, &bitmap, true);
            let left = self.dget(g, 0x0E, 0x2E, 2) - 1;
            self.dset(g, 0x0E, 0x2E, left);
            if dir { let d = self.dget(g, 0x10, 0x30, 2) + 1; self.dset(g, 0x10, 0x30, d); }
            // Inodes past the "never used" mark must move it, or a checker skips them.
            let unused = self.dget(g, 0x1C, 0x32, 2);
            if self.w.csum && i >= ipg - unused.min(ipg) { self.dset(g, 0x1C, 0x32, ipg - i - 1); }
            self.w.free_inodes -= 1;
            return Ok((g * ipg + i + 1) as u32);
        }
        Err(Error::NoSpace)
    }
    fn free_inode(&mut self, disk: &mut dyn Disk, ino: u32, dir: bool) -> Result<()> {
        let ipg = self.inodes_per_group as u64;
        let (g, i) = ((ino as u64 - 1) / ipg, (ino as u64 - 1) % ipg);
        let at = self.inode_bitmap(g);
        let mut bitmap = self.block(disk, at)?;
        bitmap[(i / 8) as usize] &= !(1 << (i % 8));
        write_bytes(disk, at * self.bs, &bitmap)?;
        self.bitmap_csum(g, &bitmap, true);
        let left = self.dget(g, 0x0E, 0x2E, 2) + 1;
        self.dset(g, 0x0E, 0x2E, left);
        if dir { let d = self.dget(g, 0x10, 0x30, 2).saturating_sub(1); self.dset(g, 0x10, 0x30, d); }
        self.w.free_inodes += 1;
        Ok(())
    }

    /// Writes changed group descriptors and the superblock's counts.
    fn settle(&mut self, disk: &mut dyn Disk) -> Result<()> {
        let per = self.bs / self.desc_size as u64;
        let dirty = core::mem::take(&mut self.w.dirty);
        for g in dirty {
            let ds = self.desc_size;
            if self.w.csum {
                let mut d = self.desc(g).to_vec();
                put16(&mut d, 0x1E, 0);
                let c = crc32c(crc32c(self.w.seed, &(g as u32).to_le_bytes()), &d);
                put16(self.desc_mut(g), 0x1E, c as u16);
                self.w.dirty.clear();
            }
            let at = self.desc_block(g) * self.bs + (g % per) * ds as u64;
            let d = self.desc(g).to_vec();
            write_bytes(disk, at, &d)?;
        }
        let mut sb = [0u8; 1024];
        read_bytes(disk, 1024, &mut sb)?;
        put32(&mut sb, 0x0C, self.free as u32);
        if self.incompat & INCOMPAT_64BIT != 0 { put32(&mut sb, 0x158, (self.free >> 32) as u32); }
        put32(&mut sb, 0x10, self.w.free_inodes);
        if self.w.now != 0 { put32(&mut sb, 0x30, self.w.now as u32); }
        if self.w.csum { let c = crc32c(!0, &sb[..0x3FC]); put32(&mut sb, 0x3FC, c); }
        write_bytes(disk, 1024, &sb)
    }

    fn inode_seed(&self, ino: u32, raw: &[u8]) -> u32 { crc32c(crc32c(self.w.seed, &ino.to_le_bytes()), &raw[0x64..0x68]) }
    fn put_inode(&mut self, disk: &mut dyn Disk, ino: u32, raw: &mut [u8]) -> Result<()> {
        if self.w.csum {
            put16(raw, 0x7C, 0);
            let hi = raw.len() > 128 && le16(raw, 0x80) >= 4;
            if hi { put16(raw, 0x82, 0); }
            let c = crc32c(self.inode_seed(ino, raw), raw);
            put16(raw, 0x7C, c as u16);
            if hi { put16(raw, 0x82, (c >> 16) as u16); }
        }
        let ipg = self.inodes_per_group as u64;
        let (g, i) = ((ino as u64 - 1) / ipg, (ino as u64 - 1) % ipg);
        let table = self.table(disk, g)?;
        write_bytes(disk, table * self.bs + i * self.inode_size as u64, raw)
    }
    fn touch(&self, raw: &mut [u8]) {
        let now = self.w.now as u32;
        put32(raw, 0x0C, now);
        put32(raw, 0x10, now);
    }
    fn set_size(raw: &mut [u8], size: u64) { put32(raw, 0x04, size as u32); put32(raw, 0x6C, (size >> 32) as u32); }

    /// Every extent of an inode, in order, and the tree's own blocks.
    fn extent_list(&self, disk: &mut dyn Disk, raw: &[u8]) -> Result<(Vec<Ext>, Vec<u64>)> {
        let (mut list, mut nodes) = (Vec::new(), Vec::new());
        self.collect(disk, &raw[0x28..0x64], None, &mut list, &mut nodes, 0)?;
        Ok((list, nodes))
    }
    fn collect(&self, disk: &mut dyn Disk, node: &[u8], want: Option<u16>, list: &mut Vec<Ext>, nodes: &mut Vec<u64>, level: u32) -> Result<()> {
        if level > 5 || node.len() < 12 || le16(node, 0) != 0xF30A { return Err(Error::Corrupt); }
        let (n, depth) = (le16(node, 2) as usize, le16(node, 6));
        if want.is_some_and(|d| d != depth) || 12 + n * 12 > node.len() { return Err(Error::Corrupt); }
        for i in 0..n {
            let e = 12 + i * 12;
            if depth == 0 {
                let raw_len = le16(node, e + 4) as u64;
                let (len, unwritten) = if raw_len > EXT_MAX { (raw_len - EXT_MAX, true) } else { (raw_len, false) };
                let p = (le16(node, e + 6) as u64) << 32 | le32(node, e + 8) as u64;
                if len == 0 || p == 0 || p + len > self.blocks { return Err(Error::Corrupt); }
                list.push(Ext { l: le32(node, e) as u64, n: len, p, unwritten });
            } else {
                let child = (le16(node, e + 8) as u64) << 32 | le32(node, e + 4) as u64;
                nodes.push(child);
                let b = self.block(disk, child)?;
                self.collect(disk, &b, Some(depth - 1), list, nodes, level + 1)?;
            }
        }
        Ok(())
    }
    /// Writes `list` as the inode's extent tree, reusing the old tree's blocks, and sets
    /// the inode's block count. The inode itself is written by the caller.
    fn store(&mut self, disk: &mut dyn Disk, ino: u32, raw: &mut [u8], list: &[Ext], mut old: Vec<u64>) -> Result<()> {
        let per = ((self.bs - 12) / 12) as usize;
        // Bottom up: each level's entries are (first logical block, 12 bytes of entry).
        let mut level: Vec<(u64, [u8; 12])> = list.iter().map(|e| {
            let mut x = [0u8; 12];
            put32(&mut x, 0, e.l as u32);
            put16(&mut x, 4, if e.unwritten { (e.n + EXT_MAX) as u16 } else { e.n as u16 });
            put16(&mut x, 6, (e.p >> 32) as u16);
            put32(&mut x, 8, e.p as u32);
            (e.l, x)
        }).collect();
        let mut depth = 0u16;
        let mut used = 0u64;
        let seed = self.inode_seed(ino, raw);
        let goal = list.first().map(|e| e.p).unwrap_or(self.first_block + (ino as u64 - 1) / self.inodes_per_group as u64 * self.blocks_per_group);
        while level.len() > 4 {
            let mut up = Vec::new();
            for chunk in level.chunks(per) {
                let at = match old.pop() { Some(b) => b, None => self.alloc_blocks(disk, goal, 1)?.0 };
                used += 1;
                let mut b = vec![0u8; self.bs as usize];
                put16(&mut b, 0, 0xF30A);
                put16(&mut b, 2, chunk.len() as u16);
                put16(&mut b, 4, per as u16);
                put16(&mut b, 6, depth);
                for (i, (_, x)) in chunk.iter().enumerate() { b[12 + i * 12..24 + i * 12].copy_from_slice(x); }
                if self.w.csum { let tail = 12 + per * 12; let c = crc32c(seed, &b[..tail]); put32(&mut b, tail, c); }
                write_bytes(disk, at * self.bs, &b)?;
                let mut x = [0u8; 12];
                put32(&mut x, 0, chunk[0].0 as u32);
                put32(&mut x, 4, at as u32);
                put16(&mut x, 8, (at >> 32) as u16);
                up.push((chunk[0].0, x));
            }
            level = up;
            depth += 1;
        }
        for b in old { self.free_blocks(disk, b, 1)?; }
        let root = &mut raw[0x28..0x64];
        root.fill(0);
        put16(root, 0, 0xF30A);
        put16(root, 2, level.len() as u16);
        put16(root, 4, 4);
        put16(root, 6, depth);
        for (i, (_, x)) in level.iter().enumerate() { root[12 + i * 12..24 + i * 12].copy_from_slice(x); }
        let sectors = (list.iter().map(|e| e.n).sum::<u64>() + used) * (self.bs / 512);
        put32(raw, 0x1C, sectors as u32);
        put16(raw, 0x74, (sectors >> 32) as u16);
        Ok(())
    }
    /// A fresh inode: no blocks, an empty extent tree.
    fn fresh(&mut self, mode: u16, links: u16) -> Vec<u8> {
        let mut raw = vec![0u8; self.inode_size];
        put16(&mut raw, 0, mode);
        put32(&mut raw, 0x08, self.w.now as u32);
        self.touch(&mut raw);
        put16(&mut raw, 0x1A, links);
        put32(&mut raw, 0x20, FL_EXTENTS);
        self.w.generation = self.w.generation.wrapping_mul(0x9E37_79B9).wrapping_add(self.w.now as u32 | 1);
        put32(&mut raw, 0x64, self.w.generation);
        if self.inode_size > 128 {
            put16(&mut raw, 0x80, self.w.extra_isize);
            if self.w.extra_isize >= 0x18 { put32(&mut raw, 0x90, self.w.now as u32); }
        }
        let root = &mut raw[0x28..0x64];
        put16(root, 0, 0xF30A);
        put16(root, 4, 4);
        raw
    }

    fn check_dir(dir: &Inode, adding: bool) -> Result<()> {
        if dir.kind() != S_IFDIR { return Err(Error::NotDir); }
        if dir.flags & FL_EXTENTS == 0 || dir.flags & (FL_INLINE | FL_ENCRYPT | FL_CASEFOLD) != 0 { return Err(Error::Unsupported); }
        let _ = adding;
        Ok(())
    }
    fn tail(&self, dno: u32, dir_raw: &[u8], b: &mut [u8]) {
        if !self.w.csum { return; }
        let t = b.len() - 12;
        b[t..].fill(0);
        put16(b, t + 4, 12);
        b[t + 7] = 0xDE;
        let c = crc32c(self.inode_seed(dno, dir_raw), &b[..t]);
        put32(b, t + 8, c);
    }
    /// The directory's blocks in order: (logical, physical).
    fn dir_blocks(&self, disk: &mut dyn Disk, dir: &Inode) -> Result<Vec<(u64, u64)>> {
        let (list, _) = self.extent_list(disk, &dir.raw)?;
        let mut out = Vec::new();
        for e in list { if !e.unwritten { for k in 0..e.n { out.push((e.l + k, e.p + k)); } } }
        out.retain(|(l, _)| *l < dir.size.div_ceil(self.bs));
        Ok(out)
    }
    /// The live entries of one directory block, as (offset, rec_len), when the block is a
    /// plain one ending where a checksum tail starts (htree index blocks are not).
    fn chain(&self, b: &[u8]) -> Option<Vec<(usize, usize)>> {
        let limit = b.len() - if self.w.csum { 12 } else { 0 };
        let (mut at, mut out) = (0, Vec::new());
        while at < limit {
            let rec = le16(b, at + 4) as usize;
            if rec < 8 || at + rec > limit || 8 + b[at + 6] as usize > rec { return None; }
            out.push((at, rec));
            at += rec;
        }
        (at == limit).then_some(out)
    }
    /// Adds `name` → `ino` to directory `dno`.
    fn link(&mut self, disk: &mut dyn Disk, dno: u32, name: &[u8], ino: u32, ftype: u8) -> Result<()> {
        let mut dir = self.inode(disk, dno)?;
        Self::check_dir(&dir, true)?;
        if dir.flags & FL_INDEX != 0 { self.unindex(disk, dno, &mut dir)?; }
        let need = rec_for(name.len());
        let put_entry = |b: &mut [u8], at: usize, rec: usize| {
            put32(b, at, ino);
            put16(b, at + 4, rec as u16);
            b[at + 6] = name.len() as u8;
            b[at + 7] = ftype;
            b[at + 8..at + 8 + name.len()].copy_from_slice(name);
        };
        for (_, p) in self.dir_blocks(disk, &dir)? {
            let mut b = self.block(disk, p)?;
            let Some(chain) = self.chain(&b) else { return Err(Error::Corrupt) };
            for (at, rec) in chain {
                let live = le32(&b, at) != 0;
                let used = if live { rec_for(b[at + 6] as usize) } else { 0 };
                if rec - used < need { continue; }
                if live { put16(&mut b, at + 4, used as u16); put_entry(&mut b, at + used, rec - used); } else { put_entry(&mut b, at, rec); }
                self.tail(dno, &dir.raw, &mut b);
                write_bytes(disk, p * self.bs, &b)?;
                self.touch(&mut dir.raw);
                return self.put_inode(disk, dno, &mut dir.raw);
            }
        }
        // No room: one more block.
        let (mut list, nodes) = self.extent_list(disk, &dir.raw)?;
        let l = dir.size.div_ceil(self.bs);
        let goal = list.last().map(|e| e.p + e.n).unwrap_or(self.first_block);
        let (p, _) = self.alloc_blocks(disk, goal, 1)?;
        let mut b = vec![0u8; self.bs as usize];
        let limit = b.len() - if self.w.csum { 12 } else { 0 };
        put_entry(&mut b, 0, limit);
        self.tail(dno, &dir.raw, &mut b);
        write_bytes(disk, p * self.bs, &b)?;
        put(&mut list, l, 1, p);
        self.store(disk, dno, &mut dir.raw, &list, nodes)?;
        Self::set_size(&mut dir.raw, (l + 1) * self.bs);
        self.touch(&mut dir.raw);
        self.put_inode(disk, dno, &mut dir.raw)
    }
    /// An indexed (htree) directory becomes a plain one, so a name can go anywhere: its
    /// index blocks become empty entry blocks and the flag goes. Every name stays where
    /// it was. Linux reads it as it is, and `e2fsck -D` indexes it again.
    fn unindex(&mut self, disk: &mut dyn Disk, dno: u32, dir: &mut Inode) -> Result<()> {
        let limit = self.bs as usize - if self.w.csum { 12 } else { 0 };
        for (l, p) in self.dir_blocks(disk, dir)? {
            let mut b = self.block(disk, p)?;
            if self.chain(&b).is_some() && l != 0 { continue; }
            if l == 0 {
                // The root: ".", then ".." whose record ran over the index to the end.
                if &b[8..9] != b"." || le16(&b, 4) != 12 || &b[20..22] != b".." { return Err(Error::Corrupt); }
                b[24..].fill(0);
                put16(&mut b, 16, (limit - 12) as u16);
            } else {
                // An interior index node: an unused entry over the whole block.
                if le32(&b, 0) != 0 || le16(&b, 4) as u64 != self.bs { return Err(Error::Corrupt); }
                b.fill(0);
                put16(&mut b, 4, limit as u16);
            }
            self.tail(dno, &dir.raw, &mut b);
            write_bytes(disk, p * self.bs, &b)?;
        }
        dir.flags &= !FL_INDEX;
        put32(&mut dir.raw, 0x20, dir.flags);
        self.put_inode(disk, dno, &mut dir.raw)?;
        *dir = self.inode(disk, dno)?;
        Ok(())
    }
    /// Takes `name` out of directory `dno`; returns its inode number.
    fn unlink(&mut self, disk: &mut dyn Disk, dno: u32, name: &[u8]) -> Result<u32> {
        let mut dir = self.inode(disk, dno)?;
        Self::check_dir(&dir, false)?;
        for (_, p) in self.dir_blocks(disk, &dir)? {
            let mut b = self.block(disk, p)?;
            let Some(chain) = self.chain(&b) else { continue };
            for (i, &(at, rec)) in chain.iter().enumerate() {
                let len = b[at + 6] as usize;
                if le32(&b, at) == 0 || &b[at + 8..at + 8 + len] != name { continue; }
                let ino = le32(&b, at);
                if i > 0 { let (prev, prec) = chain[i - 1]; put16(&mut b, prev + 4, (prec + rec) as u16); } else { put32(&mut b, at, 0); }
                self.tail(dno, &dir.raw, &mut b);
                write_bytes(disk, p * self.bs, &b)?;
                self.touch(&mut dir.raw);
                self.put_inode(disk, dno, &mut dir.raw)?;
                return Ok(ino);
            }
        }
        Err(Error::NotFound)
    }
    fn parent(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<(u32, Vec<u8>)> {
        let (dir, name) = split_last(path)?;
        if name.len() > 255 || name.is_empty() { return Err(Error::Name); }
        let name = name.to_vec();
        let (dno, d) = self.find(disk, &dir)?;
        if d.kind() != S_IFDIR { return Err(Error::NotDir); }
        Ok((dno, name))
    }
    fn exists(&mut self, disk: &mut dyn Disk, dno: u32, name: &[u8]) -> Result<Option<u32>> {
        let d = self.inode(disk, dno)?;
        Ok(self.entries(disk, &d)?.into_iter().find(|e| e.1 == name).map(|e| e.0))
    }
    fn add_links(&mut self, disk: &mut dyn Disk, ino: u32, by: i32) -> Result<()> {
        let mut i = self.inode(disk, ino)?;
        let links = (le16(&i.raw, 0x1A) as i32 + by).clamp(1, 64999) as u16;
        put16(&mut i.raw, 0x1A, links);
        self.touch(&mut i.raw);
        self.put_inode(disk, ino, &mut i.raw)
    }

    pub(super) fn make(&mut self, disk: &mut dyn Disk, path: &[u8], dir: bool) -> Result<()> {
        self.ready(disk)?;
        let (dno, name) = self.parent(disk, path)?;
        if self.exists(disk, dno, &name)?.is_some() { return Err(Error::Exists); }
        Self::check_dir(&self.inode(disk, dno)?, true)?;
        let near = (dno as u64 - 1) / self.inodes_per_group as u64;
        let ino = self.alloc_inode(disk, near, dir)?;
        let r = (|| -> Result<()> {
            if !dir {
                let mut raw = self.fresh(S_IFREG | 0o644, 1);
                self.put_inode(disk, ino, &mut raw)?;
                return self.link(disk, dno, &name, ino, 1);
            }
            let mut raw = self.fresh(S_IFDIR | 0o755, 2);
            let goal = self.first_block + near * self.blocks_per_group;
            let (p, _) = self.alloc_blocks(disk, goal, 1)?;
            let mut b = vec![0u8; self.bs as usize];
            let limit = b.len() - if self.w.csum { 12 } else { 0 };
            put32(&mut b, 0, ino); put16(&mut b, 4, 12); b[6] = 1; b[7] = 2; b[8] = b'.';
            put32(&mut b, 12, dno); put16(&mut b, 16, (limit - 12) as u16); b[18] = 2; b[19] = 2; b[20] = b'.'; b[21] = b'.';
            self.tail(ino, &raw, &mut b);
            write_bytes(disk, p * self.bs, &b)?;
            self.store(disk, ino, &mut raw, &[Ext { l: 0, n: 1, p, unwritten: false }], Vec::new())?;
            Self::set_size(&mut raw, self.bs);
            self.put_inode(disk, ino, &mut raw)?;
            self.link(disk, dno, &name, ino, 2)?;
            self.add_links(disk, dno, 1)
        })();
        if r.is_err() { let _ = self.free_inode(disk, ino, dir); }
        self.settle(disk)?;
        r
    }

    pub(super) fn write_file(&mut self, disk: &mut dyn Disk, path: &[u8], offset: u64, data: &[u8]) -> Result<usize> {
        self.ready(disk)?;
        let (ino, mut node) = self.find(disk, &parts(path)?)?;
        Self::check_file(&node)?;
        if data.is_empty() { return Ok(0); }
        let end = offset.checked_add(data.len() as u64).ok_or(Error::TooBig)?;
        if end.div_ceil(self.bs) >= 1 << 32 { return Err(Error::TooBig); }
        let (mut list, nodes) = self.extent_list(disk, &node.raw)?;
        let bs = self.bs;
        let mut pos = offset;
        let mut failed = None;
        while pos < end {
            let l = pos / bs;
            let last = (end - 1) / bs;
            let found = covering(&list, l);
            if let Some(e) = found.filter(|e| !e.unwritten) {
                let run = (e.l + e.n - l).min(last - l + 1);
                let upto = ((l + run) * bs).min(end);
                if let Err(x) = write_bytes(disk, (e.p + l - e.l) * bs + pos % bs, &data[(pos - offset) as usize..(upto - offset) as usize]) { failed = Some(x); break; }
                pos = upto;
                continue;
            }
            // A hole or unwritten blocks: whole blocks, zero where the data doesn't reach.
            let (run, p) = match found {
                Some(e) => ((e.l + e.n - l).min(last - l + 1), e.p + l - e.l),
                None => {
                    let hole = next_start(&list, l).map(|s| s - l).unwrap_or(u64::MAX);
                    let goal = list.iter().filter(|e| e.l < l).last().map(|e| e.p + (l - e.l))
                        .unwrap_or(self.first_block + (ino as u64 - 1) / self.inodes_per_group as u64 * self.blocks_per_group);
                    match self.alloc_blocks(disk, goal, hole.min(last - l + 1)) { Ok((p, n)) => (n, p), Err(x) => { failed = Some(x); break; } }
                }
            };
            let upto = ((l + run) * bs).min(end);
            let mut buf = vec![0u8; (run * bs) as usize];
            buf[(pos - l * bs) as usize..(upto - l * bs) as usize].copy_from_slice(&data[(pos - offset) as usize..(upto - offset) as usize]);
            if let Err(x) = write_bytes(disk, p * bs, &buf) { failed = Some(x); put(&mut list, l, run, p); break; }
            put(&mut list, l, run, p);
            pos = upto;
        }
        self.store(disk, ino, &mut node.raw, &list, nodes)?;
        if pos > node.size { Self::set_size(&mut node.raw, pos); }
        self.touch(&mut node.raw);
        self.put_inode(disk, ino, &mut node.raw)?;
        self.settle(disk)?;
        match failed { Some(e) if pos == offset => Err(e), _ => Ok((pos - offset) as usize) }
    }
    fn check_file(node: &Inode) -> Result<()> {
        match node.kind() { S_IFDIR => return Err(Error::IsDir), S_IFREG => {}, _ => return Err(Error::Unsupported) }
        if node.flags & FL_EXTENTS == 0 || node.flags & (FL_INLINE | FL_ENCRYPT) != 0 { return Err(Error::Unsupported); }
        Ok(())
    }

    pub(super) fn truncate_file(&mut self, disk: &mut dyn Disk, path: &[u8], len: u64) -> Result<()> {
        self.ready(disk)?;
        let (ino, mut node) = self.find(disk, &parts(path)?)?;
        Self::check_file(&node)?;
        if len.div_ceil(self.bs) >= 1 << 32 { return Err(Error::TooBig); }
        if len < node.size {
            let (list, nodes) = self.extent_list(disk, &node.raw)?;
            let keep = len.div_ceil(self.bs);
            let mut kept = Vec::new();
            for e in list {
                if e.l + e.n <= keep { kept.push(e); }
                else if e.l >= keep { self.free_blocks(disk, e.p, e.n)?; }
                else { let k = keep - e.l; self.free_blocks(disk, e.p + k, e.n - k)?; kept.push(Ext { n: k, ..e }); }
            }
            // What was past the new end must read as zeros if the file grows again.
            if len % self.bs != 0 {
                if let Some(e) = covering(&kept, len / self.bs).filter(|e| !e.unwritten) {
                    let within = len % self.bs;
                    write_bytes(disk, (e.p + len / self.bs - e.l) * self.bs + within, &vec![0u8; (self.bs - within) as usize])?;
                }
            }
            self.store(disk, ino, &mut node.raw, &kept, nodes)?;
        }
        Self::set_size(&mut node.raw, len);
        self.touch(&mut node.raw);
        self.put_inode(disk, ino, &mut node.raw)?;
        self.settle(disk)
    }

    pub(super) fn unlink_path(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<()> {
        self.ready(disk)?;
        let (dno, name) = self.parent(disk, path)?;
        let ino = self.exists(disk, dno, &name)?.ok_or(Error::NotFound)?;
        let mut node = self.inode(disk, ino)?;
        let dir = node.kind() == S_IFDIR;
        if dir && !self.entries(disk, &node)?.is_empty() { return Err(Error::NotEmpty); }
        if node.flags & FL_INLINE != 0 || node.flags & FL_EXTENTS == 0 && node.kind() != S_IFLNK { return Err(Error::Unsupported); }
        self.unlink(disk, dno, &name)?;
        let links = le16(&node.raw, 0x1A);
        if !dir && links > 1 {
            put16(&mut node.raw, 0x1A, links - 1);
            self.touch(&mut node.raw);
            self.put_inode(disk, ino, &mut node.raw)?;
            return self.settle(disk);
        }
        if node.flags & FL_EXTENTS != 0 {
            let (list, nodes) = self.extent_list(disk, &node.raw)?;
            for e in &list { self.free_blocks(disk, e.p, e.n)?; }
            self.store(disk, ino, &mut node.raw, &[], nodes)?;
        }
        put16(&mut node.raw, 0x1A, 0);
        Self::set_size(&mut node.raw, 0);
        put32(&mut node.raw, 0x14, (self.w.now as u32).max(1));
        self.touch(&mut node.raw);
        self.put_inode(disk, ino, &mut node.raw)?;
        self.free_inode(disk, ino, dir)?;
        if dir { self.add_links(disk, dno, -1)?; }
        self.settle(disk)
    }

    pub(super) fn move_path(&mut self, disk: &mut dyn Disk, from: &[u8], to: &[u8]) -> Result<()> {
        self.ready(disk)?;
        let (fp, tp) = (parts(from)?, parts(to)?);
        if tp.len() > fp.len() && tp[..fp.len()] == fp[..] { return Err(Error::Name); }
        let (sdno, sname) = self.parent(disk, from)?;
        let (ddno, dname) = self.parent(disk, to)?;
        let ino = self.exists(disk, sdno, &sname)?.ok_or(Error::NotFound)?;
        if self.exists(disk, ddno, &dname)?.is_some() { return Err(Error::Exists); }
        let node = self.inode(disk, ino)?;
        let dir = node.kind() == S_IFDIR;
        self.link(disk, ddno, &dname, ino, file_type(node.mode))?;
        self.unlink(disk, sdno, &sname)?;
        if dir && sdno != ddno {
            // Its `..` now names the new parent.
            let mut moved = self.inode(disk, ino)?;
            if let Some(&(_, p)) = self.dir_blocks(disk, &moved)?.first() {
                let mut b = self.block(disk, p)?;
                if le16(&b, 4) == 12 && b[6] == 1 && b[20..22] == *b".." {
                    put32(&mut b, 12, ddno);
                    self.tail(ino, &moved.raw, &mut b);
                    write_bytes(disk, p * self.bs, &b)?;
                }
            }
            self.touch(&mut moved.raw);
            self.put_inode(disk, ino, &mut moved.raw)?;
            self.add_links(disk, sdno, -1)?;
            self.add_links(disk, ddno, 1)?;
        }
        self.settle(disk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crc32c_matches_the_standard() {
        // The standard check value is for the inverted form.
        assert_eq!(!crc32c(!0, b"123456789"), 0xE306_9283);
    }
    #[test]
    fn extents_join_and_split() {
        let mut l = Vec::new();
        put(&mut l, 0, 10, 100);
        put(&mut l, 10, 5, 110);
        assert_eq!(l, vec![Ext { l: 0, n: 15, p: 100, unwritten: false }]);
        put(&mut l, 3, 2, 500);
        assert_eq!(l.len(), 3);
        assert_eq!(covering(&l, 4).map(|e| e.p + 4 - e.l), Some(501));
        assert_eq!(covering(&l, 5).map(|e| e.p + 5 - e.l), Some(105));
        let mut big = Vec::new();
        put(&mut big, 0, 70000, 1000);
        assert_eq!(big.iter().map(|e| e.n).collect::<Vec<_>>(), vec![32768, 32768, 4464]);
    }
}
