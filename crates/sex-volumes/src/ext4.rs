// SPDX-License-Identifier: GPL-3.0-only
//! ext2, ext3 and ext4, read only: the disks Linux machines leave behind.
//!
//! Files are found through extent trees (ext4) or classic block maps (ext2/3), small
//! files and directories may live inside the inode itself (inline data), and htree
//! directories are read linearly: their index blocks look like empty dirents to a reader
//! that doesn't care about the hash. Nothing here writes, so the journal is never
//! replayed and checksums are not verified; a volume with features we can't read
//! (compression, a journal device, dirdata) is refused at mount, an encrypted file or
//! directory when it is opened.
//!
//! Listings show regular files, directories and symlinks (as files whose contents are the
//! link's target); device nodes, FIFOs and sockets are left out. A symlink in the middle
//! of a path is not followed.
use crate::{le16, le32, parts, read_bytes, Disk, Entry, Error, Format, Kind, Result, Stat, Volume};
use alloc::{string::String, vec, vec::Vec};

const ROOT: u32 = 2;
const INCOMPAT_FILETYPE: u32 = 0x2;
const INCOMPAT_META_BG: u32 = 0x10;
const INCOMPAT_64BIT: u32 = 0x80;
const INCOMPAT_LARGEDIR: u32 = 0x4000;
/// filetype, recover, meta_bg, extents, 64bit, mmp, flex_bg, ea_inode, csum_seed,
/// largedir, inline_data, encrypt, casefold. Missing: compression, journal_dev, dirdata.
const INCOMPAT_READ: u32 = 0x2 | 0x4 | 0x10 | 0x40 | 0x80 | 0x100 | 0x200 | 0x400 | 0x2000 | 0x4000 | 0x8000 | 0x1_0000 | 0x2_0000;
const RO_COMPAT_SPARSE_SUPER: u32 = 0x1;
const FL_ENCRYPT: u32 = 0x800;
const FL_EXTENTS: u32 = 0x8_0000;
const FL_INLINE: u32 = 0x1000_0000;
const S_IFMT: u16 = 0xF000;
const S_IFDIR: u16 = 0x4000;
const S_IFREG: u16 = 0x8000;
const S_IFLNK: u16 = 0xA000;

pub struct Ext4 {
    bs: u64,
    blocks: u64,
    free: u64,
    first_block: u64,
    blocks_per_group: u64,
    inodes: u32,
    inodes_per_group: u32,
    inode_size: usize,
    desc_size: usize,
    groups: u64,
    incompat: u32,
    ro_compat: u32,
    first_meta_bg: u64,
    /// Each group's inode table block, 0 until its descriptor block has been read.
    tables: Vec<u64>,
    label: String,
}

/// One inode as stored, with the fields every caller wants pulled out.
struct Inode { raw: Vec<u8>, mode: u16, flags: u32, size: u64, mtime: u64 }
impl Inode {
    fn kind(&self) -> u16 { self.mode & S_IFMT }
    fn iblock(&self) -> &[u8] { &self.raw[0x28..0x64] }
}

/// The few blocks a single call reads more than once (indirect blocks, extent nodes).
#[derive(Default)]
struct Cache(Vec<(u64, Vec<u8>)>);

impl Ext4 {
    pub fn mount(disk: &mut dyn Disk) -> Result<Self> {
        let mut sb = [0u8; 1024];
        read_bytes(disk, 1024, &mut sb)?;
        if le16(&sb, 0x38) != 0xEF53 { return Err(Error::Corrupt); }
        let log = le32(&sb, 0x18);
        if log > 6 { return Err(Error::Corrupt); }
        let bs = 1024u64 << log;
        let rev = le32(&sb, 0x4C);
        let (incompat, ro_compat, inode_size) = if rev == 0 { (0, 0, 128) } else { (le32(&sb, 0x60), le32(&sb, 0x64), le16(&sb, 0x58) as usize) };
        if incompat & !INCOMPAT_READ != 0 { return Err(Error::Unsupported); }
        if inode_size < 128 || !inode_size.is_power_of_two() || inode_size as u64 > bs { return Err(Error::Corrupt); }
        let wide = incompat & INCOMPAT_64BIT != 0;
        let hi = |at: usize| if wide { (le32(&sb, at) as u64) << 32 } else { 0 };
        let blocks = le32(&sb, 0x04) as u64 | hi(0x150);
        let free = le32(&sb, 0x0C) as u64 | hi(0x158);
        let (first_block, blocks_per_group) = (le32(&sb, 0x14) as u64, le32(&sb, 0x20) as u64);
        let (inodes, inodes_per_group) = (le32(&sb, 0x00), le32(&sb, 0x28));
        if blocks_per_group == 0 || inodes_per_group == 0 || blocks <= first_block { return Err(Error::Corrupt); }
        // The whole volume must fit on the disk: this also bounds every table we size.
        let bytes = blocks.checked_mul(bs).ok_or(Error::Corrupt)?;
        if bytes > disk.sectors().saturating_mul(crate::SECTOR as u64) { return Err(Error::Corrupt); }
        let groups = (blocks - first_block).div_ceil(blocks_per_group);
        if inodes as u64 > groups * inodes_per_group as u64 { return Err(Error::Corrupt); }
        let desc_size = if wide { le16(&sb, 0xFE) as usize } else { 32 };
        if desc_size < 32 || !desc_size.is_power_of_two() || desc_size as u64 > bs || (wide && desc_size < 64) { return Err(Error::Corrupt); }
        let name = &sb[0x78..0x88];
        let end = name.iter().position(|b| *b == 0).unwrap_or(16);
        Ok(Self {
            bs, blocks, free, first_block, blocks_per_group, inodes, inodes_per_group, inode_size, desc_size, groups, incompat, ro_compat,
            first_meta_bg: le32(&sb, 0x104) as u64,
            tables: vec![0; groups as usize],
            label: String::from_utf8_lossy(&name[..end]).into_owned(),
        })
    }

    fn block(&self, disk: &mut dyn Disk, n: u64) -> Result<Vec<u8>> {
        if n == 0 || n >= self.blocks { return Err(Error::Corrupt); }
        let mut b = vec![0u8; self.bs as usize];
        read_bytes(disk, n * self.bs, &mut b)?;
        Ok(b)
    }
    fn cached<'c>(&self, disk: &mut dyn Disk, cache: &'c mut Cache, n: u64) -> Result<&'c [u8]> {
        let at = match cache.0.iter().position(|(k, _)| *k == n) {
            Some(i) => i,
            None => {
                let b = self.block(disk, n)?;
                if cache.0.len() >= 8 { cache.0.remove(0); }
                cache.0.push((n, b));
                cache.0.len() - 1
            }
        };
        Ok(&cache.0[at].1)
    }

    fn has_super(&self, g: u64) -> bool {
        if self.ro_compat & RO_COMPAT_SPARSE_SUPER == 0 || g <= 1 { return true; }
        [3u64, 5, 7].iter().any(|&p| { let mut x = p; while x < g { x *= p; } x == g })
    }
    /// The block holding group `g`'s descriptor: one table after the superblock, or with
    /// meta_bg one block at the start of each run of groups it describes.
    fn desc_block(&self, g: u64) -> u64 {
        let per = self.bs / self.desc_size as u64;
        let meta = g / per;
        if self.incompat & INCOMPAT_META_BG == 0 || meta < self.first_meta_bg { return self.first_block + 1 + meta; }
        let lead = meta * per;
        self.first_block + lead * self.blocks_per_group + self.has_super(lead) as u64
    }
    fn table(&mut self, disk: &mut dyn Disk, g: u64) -> Result<u64> {
        if self.tables[g as usize] == 0 {
            let per = self.bs / self.desc_size as u64;
            let b = self.block(disk, self.desc_block(g))?;
            let lead = g / per * per;
            for i in 0..per.min(self.groups - lead) {
                let d = &b[(i as usize) * self.desc_size..];
                let hi = if self.desc_size >= 64 { (le32(d, 0x28) as u64) << 32 } else { 0 };
                self.tables[(lead + i) as usize] = le32(d, 0x08) as u64 | hi;
            }
        }
        let t = self.tables[g as usize];
        if t == 0 || t >= self.blocks { return Err(Error::Corrupt); }
        Ok(t)
    }

    fn inode(&mut self, disk: &mut dyn Disk, ino: u32) -> Result<Inode> {
        if ino == 0 || ino > self.inodes { return Err(Error::Corrupt); }
        let (g, i) = (((ino - 1) / self.inodes_per_group) as u64, ((ino - 1) % self.inodes_per_group) as u64);
        let at = i * self.inode_size as u64;
        let table = self.table(disk, g)?;
        if table + at / self.bs >= self.blocks { return Err(Error::Corrupt); }
        let mut raw = vec![0u8; self.inode_size];
        read_bytes(disk, table * self.bs + at, &mut raw)?;
        let (mode, flags) = (le16(&raw, 0), le32(&raw, 0x20));
        let wide = mode & S_IFMT == S_IFREG || self.incompat & INCOMPAT_LARGEDIR != 0;
        let size = le32(&raw, 0x04) as u64 | if wide { (le32(&raw, 0x6C) as u64) << 32 } else { 0 };
        // ext4 widens the 32-bit signed time with two epoch bits in the inode's extra space.
        let extra = if raw.len() > 0x82 { le16(&raw, 0x80) as usize } else { 0 };
        let epoch = if extra >= 0x0C && raw.len() >= 0x8C { (le32(&raw, 0x88) & 3) as i64 } else { 0 };
        let mtime = (le32(&raw, 0x10) as i32 as i64 + (epoch << 32)).max(0) as u64;
        Ok(Inode { raw, mode, flags, size, mtime })
    }

    /// Where logical block `lblk` lives (None for a hole or unwritten extent) and how many
    /// following blocks share that answer.
    fn map(&self, disk: &mut dyn Disk, cache: &mut Cache, ino: &Inode, lblk: u64) -> Result<(Option<u64>, u64)> {
        if ino.flags & FL_EXTENTS != 0 { self.extent(disk, cache, ino.iblock(), lblk) } else { self.indirect(disk, cache, ino.iblock(), lblk) }
    }
    fn extent(&self, disk: &mut dyn Disk, cache: &mut Cache, root: &[u8], lblk: u64) -> Result<(Option<u64>, u64)> {
        if lblk > u32::MAX as u64 { return Ok((None, u64::MAX)); }
        let mut node: Vec<u8> = root.to_vec();
        let mut limit = 1u64 << 32; // first block past this subtree's last entry
        let mut want = None;
        for _ in 0..8 {
            if node.len() < 12 || le16(&node, 0) != 0xF30A { return Err(Error::Corrupt); }
            let (n, depth) = (le16(&node, 2) as usize, le16(&node, 6));
            if want.is_some_and(|d| d != depth) || 12 + n * 12 > node.len() { return Err(Error::Corrupt); }
            let start = |i: usize| le32(&node, 12 + i * 12) as u64;
            if depth == 0 {
                for i in 0..n {
                    let e = 12 + i * 12;
                    let (first, raw_len) = (start(i), le16(&node, e + 4) as u64);
                    let (len, unwritten) = if raw_len > 32768 { (raw_len - 32768, true) } else { (raw_len, false) };
                    if lblk < first { return Ok((None, first - lblk)); }
                    if lblk < first + len {
                        let phys = ((le16(&node, e + 6) as u64) << 32 | le32(&node, e + 8) as u64) + (lblk - first);
                        return Ok((if unwritten { None } else { Some(phys) }, first + len - lblk));
                    }
                }
                return Ok((None, limit.saturating_sub(lblk).max(1)));
            }
            let Some(i) = (0..n).rev().find(|&i| start(i) <= lblk) else {
                return Ok((None, if n == 0 { limit - lblk } else { start(0) - lblk }));
            };
            if i + 1 < n { limit = limit.min(start(i + 1)); }
            let e = 12 + i * 12;
            let child = (le16(&node, e + 8) as u64) << 32 | le32(&node, e + 4) as u64;
            node = self.cached(disk, cache, child)?.to_vec();
            want = Some(depth - 1);
        }
        Err(Error::Corrupt)
    }
    /// ext2/3 block maps: 12 direct pointers, then single, double and triple indirect.
    fn indirect(&self, disk: &mut dyn Disk, cache: &mut Cache, iblock: &[u8], lblk: u64) -> Result<(Option<u64>, u64)> {
        let p = self.bs / 4;
        let (slot, levels, idx) = if lblk < 12 { (lblk as usize, 0u32, 0) } else {
            let mut l = lblk - 12;
            let mut found = None;
            for lv in 1..=3u32 {
                let span = p.pow(lv);
                if l < span { found = Some((11 + lv as usize, lv, l)); break; }
                l -= span;
            }
            match found { Some(f) => f, None => return Ok((None, u64::MAX)) }
        };
        let mut blk = le32(iblock, slot * 4) as u64;
        for lv in (0..levels).rev() {
            // A missing pointer leaves the whole subtree under it a hole.
            if blk == 0 { let span = p.pow(lv + 1); return Ok((None, span - idx % span)); }
            let b = self.cached(disk, cache, blk)?;
            blk = le32(b, (idx / p.pow(lv) % p) as usize * 4) as u64;
        }
        Ok((if blk == 0 { None } else { Some(blk) }, 1))
    }

    /// Inline data: the 60 bytes of i_block, then the `system.data` extended attribute
    /// in the inode's spare space.
    fn inline(&self, ino: &Inode) -> Result<Vec<u8>> {
        let mut out = ino.iblock().to_vec();
        let raw = &ino.raw;
        if raw.len() <= 0x82 { return Ok(out); }
        let base = 128 + le16(raw, 0x80) as usize;
        if base + 4 > raw.len() || le32(raw, base) != 0xEA02_0000 { return Ok(out); }
        let first = base + 4;
        let mut at = first;
        while at + 16 <= raw.len() && le32(raw, at) != 0 {
            let (len, index) = (raw[at] as usize, raw[at + 1]);
            let (offs, size) = (le16(raw, at + 2) as usize, le32(raw, at + 8) as usize);
            if at + 16 + len > raw.len() { return Err(Error::Corrupt); }
            if index == 7 && &raw[at + 16..at + 16 + len] == b"data" {
                let v = raw.get(first + offs..first + offs + size).ok_or(Error::Corrupt)?;
                out.extend_from_slice(v);
                break;
            }
            at += (16 + len + 3) & !3;
        }
        Ok(out)
    }

    /// Reads `buf.len()` bytes of the inode's data at `offset`; the caller keeps it
    /// within the file's size.
    fn read_data(&self, disk: &mut dyn Disk, ino: &Inode, offset: u64, buf: &mut [u8]) -> Result<()> {
        let fast_link = ino.kind() == S_IFLNK && ino.flags & (FL_EXTENTS | FL_INLINE) == 0 && ino.size < 60;
        if ino.flags & FL_INLINE != 0 || fast_link {
            let data = if fast_link { ino.iblock().to_vec() } else { self.inline(ino)? };
            for (i, b) in buf.iter_mut().enumerate() { *b = data.get(offset as usize + i).copied().unwrap_or(0); }
            return Ok(());
        }
        let mut cache = Cache::default();
        let mut done = 0usize;
        while done < buf.len() {
            let pos = offset + done as u64;
            let within = pos % self.bs;
            let (phys, run) = self.map(disk, &mut cache, ino, pos / self.bs)?;
            let take = (run.saturating_mul(self.bs) - within).min((buf.len() - done) as u64) as usize;
            let out = &mut buf[done..done + take];
            match phys {
                None => out.fill(0),
                Some(p) => {
                    let last = p.checked_add((within + take as u64 - 1) / self.bs).ok_or(Error::Corrupt)?;
                    if p == 0 || last >= self.blocks { return Err(Error::Corrupt); }
                    read_bytes(disk, p * self.bs + within, out)?;
                }
            }
            done += take;
        }
        Ok(())
    }

    /// A directory's live entries other than `.` and `..`, as (inode, name). Kinds come
    /// from the inodes, which listings read anyway for sizes.
    fn entries(&self, disk: &mut dyn Disk, dir: &Inode) -> Result<Vec<(u32, Vec<u8>)>> {
        if dir.flags & FL_ENCRYPT != 0 { return Err(Error::Unsupported); }
        let mut out = Vec::new();
        if dir.flags & FL_INLINE != 0 {
            // The parent's number, then dirents in the rest of i_block and in the attribute.
            let data = self.inline(dir)?;
            self.dirents(&data[4..60], &mut out)?;
            self.dirents(&data[60..], &mut out)?;
            return Ok(out);
        }
        let mut cache = Cache::default();
        let (count, mut lblk) = (dir.size.div_ceil(self.bs), 0);
        while lblk < count {
            let (phys, run) = self.map(disk, &mut cache, dir, lblk)?;
            if let Some(p) = phys {
                let b = self.block(disk, p)?;
                self.dirents(&b, &mut out)?;
                lblk += 1;
            } else { lblk = lblk.saturating_add(run); }
        }
        Ok(out)
    }
    fn dirents(&self, b: &[u8], out: &mut Vec<(u32, Vec<u8>)>) -> Result<()> {
        let mut at = 0;
        while at + 8 <= b.len() {
            let raw = le16(b, at + 4) as usize;
            // 64 KiB blocks squeeze the length's top bits into its low two.
            let rec = if self.bs < 65536 { raw } else if raw == 0 || raw == 65535 { 65536 } else { (raw & 65532) | ((raw & 3) << 16) };
            // Without filetype the type byte is the name length's high byte.
            let len = if self.incompat & INCOMPAT_FILETYPE != 0 { b[at + 6] as usize } else { le16(b, at + 6) as usize };
            if rec < 8 || at + rec > b.len() || 8 + len > rec { return Err(Error::Corrupt); }
            let (ino, name) = (le32(b, at), &b[at + 8..at + 8 + len]);
            if ino != 0 && name != b"." && name != b".." { out.push((ino, name.to_vec())); }
            at += rec;
        }
        Ok(())
    }

    /// Follows `path` from the root directory.
    fn walk(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<Inode> {
        let mut cur = self.inode(disk, ROOT)?;
        for part in parts(path)? {
            match cur.kind() { S_IFDIR => {}, S_IFLNK => return Err(Error::Unsupported), _ => return Err(Error::NotDir) }
            let ino = self.entries(disk, &cur)?.into_iter().find(|e| e.1 == part).ok_or(Error::NotFound)?.0;
            cur = self.inode(disk, ino)?;
        }
        Ok(cur)
    }
}

fn kind_of(ino: &Inode) -> Option<Kind> {
    match ino.kind() { S_IFDIR => Some(Kind::Dir), S_IFREG | S_IFLNK => Some(Kind::File), _ => None }
}

impl Volume for Ext4 {
    fn format(&self) -> Format { Format::Ext4 }
    fn label(&self) -> String { self.label.clone() }
    fn read_only(&self) -> bool { true }
    fn statfs(&mut self, _: &mut dyn Disk) -> Result<(u64, u64)> { Ok((self.blocks * self.bs, self.free.min(self.blocks) * self.bs)) }
    fn stat(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<Stat> {
        let ino = self.walk(disk, path)?;
        let kind = kind_of(&ino).ok_or(Error::Unsupported)?;
        Ok(Stat { kind, size: if kind == Kind::Dir { 0 } else { ino.size }, mtime: ino.mtime })
    }
    fn list(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<Vec<Entry>> {
        let dir = self.walk(disk, path)?;
        if dir.kind() != S_IFDIR { return Err(Error::NotDir); }
        let mut out = Vec::new();
        for (n, name) in self.entries(disk, &dir)? {
            let ino = self.inode(disk, n)?;
            let Some(kind) = kind_of(&ino) else { continue };
            out.push(Entry { name: String::from_utf8_lossy(&name).into_owned(), kind, size: if kind == Kind::Dir { 0 } else { ino.size } });
        }
        Ok(out)
    }
    fn read(&mut self, disk: &mut dyn Disk, path: &[u8], offset: u64, buf: &mut [u8]) -> Result<usize> {
        let ino = self.walk(disk, path)?;
        match kind_of(&ino) { Some(Kind::Dir) => return Err(Error::IsDir), None => return Err(Error::Unsupported), _ => {} }
        if ino.flags & FL_ENCRYPT != 0 { return Err(Error::Unsupported); }
        if offset >= ino.size { return Ok(0); }
        let n = ((ino.size - offset).min(buf.len() as u64)) as usize;
        self.read_data(disk, &ino, offset, &mut buf[..n])?;
        Ok(n)
    }
}
