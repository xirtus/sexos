// SPDX-License-Identifier: GPL-3.0-only
//! FAT16/FAT32, as much as the installer needs: read files out of a FAT boot image, and add
//! SexOS's boot files to an EFI System Partition that may belong to another OS.
//!
//! Safety comes before features. The BPB is checked thoroughly and anything odd is
//! [`Error::Corrupt`] or [`Error::Unsupported`] rather than a guess. Writes never touch an
//! existing file's data or chain, except [`Fat::write_file`] replacing the file the caller
//! named. Every change is ordered for crash safety: new data clusters first, then the FAT
//! entries in every FAT copy, then the directory entry, and only then is an old chain freed.
//! A crash at any point leaves at worst lost clusters, never a damaged file.
//!
//! New names are 8.3 short entries only (uppercase, no LFN generation). Lookups match both
//! long (VFAT) names and short names, case-insensitively (ASCII).

use alloc::{string::String, vec, vec::Vec};

use crate::{BlockDev, Error, SECTOR};

const ATTR_VOLUME: u8 = 0x08;
const ATTR_DIR: u8 = 0x10;
const ATTR_ARCHIVE: u8 = 0x20;
const ATTR_LFN: u8 = 0x0F;
/// 2026-11-09 12:00:00, the timestamp SexOS stamps on what it writes (no RTC needed).
const DOS_DATE: u16 = ((2026 - 1980) << 9) | (11 << 5) | 9;
const DOS_TIME: u16 = 12 << 11;
/// Sectors per bulk transfer when scanning the FAT or writing data.
const CHUNK: usize = 64;
/// FAT's own limit on directory size: 65536 entries of 32 bytes.
const MAX_DIR_BYTES: u64 = 65536 * 32;

fn le16(b: &[u8], o: usize) -> u16 { u16::from_le_bytes([b[o], b[o + 1]]) }
fn le32(b: &[u8], o: usize) -> u32 { u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]) }
fn put16(b: &mut [u8], o: usize, v: u16) { b[o..o + 2].copy_from_slice(&v.to_le_bytes()); }
fn put32(b: &mut [u8], o: usize, v: u32) { b[o..o + 4].copy_from_slice(&v.to_le_bytes()); }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// FAT12 is read-only here: every write returns [`Error::Unsupported`].
pub enum FatKind { Fat12, Fat16, Fat32 }

/// A FAT volume's parsed geometry. All sector numbers are relative to the volume's sector 0.
#[derive(Clone, Debug)]
pub struct Fat {
    pub kind: FatKind,
    /// Always 512: other sector sizes are refused at [`Fat::open`].
    pub bytes_per_sector: u32,
    pub sectors_per_cluster: u32,
    pub reserved: u32,
    pub num_fats: u32,
    /// Sectors per FAT copy.
    pub fat_size: u32,
    /// FAT16 fixed root directory entries (0 on FAT32).
    pub root_entries: u32,
    /// FAT16: first sector of the fixed root directory (0 on FAT32).
    pub root_dir_start: u32,
    /// FAT32: first cluster of the root directory (0 on FAT16).
    pub root_cluster: u32,
    pub first_data_sector: u32,
    pub cluster_count: u32,
    pub total_sectors: u32,
    root_dir_sectors: u32,
    /// FAT32 FSInfo sector, when its signatures are valid.
    fsinfo: Option<u32>,
    /// FAT32 with mirroring disabled: the only FAT copy in use.
    active_fat: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    /// The long name if a valid LFN is present, else the 8.3 name as "NAME.EXT".
    pub name: String,
    pub is_dir: bool,
    pub size: u32,
    pub first_cluster: u32,
}

/// A contiguous run of sectors holding a file's data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    /// Relative to the FAT volume's sector 0.
    pub first_lba: u64,
    /// The whole cluster run (a multiple of the cluster size, at least the file size).
    pub sectors: u64,
}

#[derive(Clone, Copy)]
enum Loc { Root16, Chain(u32) }

struct Item {
    ent: DirEntry,
    short: [u8; 11],
    /// Where the short entry lives: sector and byte offset.
    lba: u64,
    off: usize,
}

impl Fat {
    pub fn open<D: BlockDev + ?Sized>(dev: &mut D) -> Result<Self, Error> {
        let mut b = [0u8; SECTOR];
        dev.read(0, &mut b)?;
        if b[510] != 0x55 || b[511] != 0xAA { return Err(Error::Corrupt); }
        if b[0] != 0xEB && b[0] != 0xE9 { return Err(Error::Corrupt); }
        let bps = le16(&b, 11) as u32;
        if !matches!(bps, 512 | 1024 | 2048 | 4096) { return Err(Error::Corrupt); }
        if bps != 512 { return Err(Error::Unsupported); }
        let spc = b[13] as u32;
        if spc == 0 || !spc.is_power_of_two() { return Err(Error::Corrupt); }
        let reserved = le16(&b, 14) as u32;
        if reserved == 0 { return Err(Error::Corrupt); }
        let num_fats = b[16] as u32;
        if num_fats == 0 || num_fats > 4 { return Err(Error::Corrupt); }
        let root_entries = le16(&b, 17) as u32;
        let media = b[21];
        if media != 0xF0 && media < 0xF8 { return Err(Error::Corrupt); }
        let tot16 = le16(&b, 19) as u32;
        let fat16_size = le16(&b, 22) as u32;
        let total = if tot16 != 0 { tot16 } else { le32(&b, 32) };
        if total == 0 || total as u64 > dev.sectors() { return Err(Error::Corrupt); }
        let fat32_layout = fat16_size == 0;
        let fat_size = if fat32_layout { le32(&b, 36) } else { fat16_size };
        if fat_size == 0 { return Err(Error::Corrupt); }
        let root_dir_sectors = (root_entries * 32).div_ceil(SECTOR as u32);
        let first_data = reserved as u64 + num_fats as u64 * fat_size as u64 + root_dir_sectors as u64;
        if first_data >= total as u64 { return Err(Error::Corrupt); }
        let cluster_count = ((total as u64 - first_data) / spc as u64) as u32;
        let kind = if cluster_count < 4085 {
            FatKind::Fat12
        } else if cluster_count < 65525 { FatKind::Fat16 } else { FatKind::Fat32 };
        // FAT bytes needed for entries 0..cluster_count+2: 1.5, 2 or 4 bytes each.
        let need = match kind {
            FatKind::Fat12 => ((cluster_count as u64 + 2) * 3).div_ceil(2),
            FatKind::Fat16 => (cluster_count as u64 + 2) * 2,
            FatKind::Fat32 => (cluster_count as u64 + 2) * 4,
        };
        if (fat_size as u64) * (SECTOR as u64) < need { return Err(Error::Corrupt); }
        let mut fat = Fat {
            kind, bytes_per_sector: bps, sectors_per_cluster: spc, reserved, num_fats, fat_size,
            root_entries, root_dir_start: 0, root_cluster: 0, first_data_sector: first_data as u32,
            cluster_count, total_sectors: total, root_dir_sectors, fsinfo: None, active_fat: None,
        };
        match kind {
            FatKind::Fat12 | FatKind::Fat16 => {
                if fat32_layout || root_entries == 0 { return Err(Error::Corrupt); }
                fat.root_dir_start = reserved + num_fats * fat_size;
            }
            FatKind::Fat32 => {
                if !fat32_layout || root_entries != 0 || tot16 != 0 { return Err(Error::Corrupt); }
                if le16(&b, 42) != 0 { return Err(Error::Unsupported); } // FAT32 version
                if cluster_count > 0x0FFF_FFF5 { return Err(Error::Corrupt); }
                let rc = le32(&b, 44);
                if !fat.valid_cluster(rc) { return Err(Error::Corrupt); }
                fat.root_cluster = rc;
                let ext = le16(&b, 40) as u32;
                if ext & 0x80 != 0 {
                    let a = ext & 0x0F;
                    if a >= num_fats { return Err(Error::Corrupt); }
                    fat.active_fat = Some(a);
                }
                let fs = le16(&b, 48) as u32;
                if fs != 0 && fs < reserved {
                    let mut f = [0u8; SECTOR];
                    dev.read(fs as u64, &mut f)?;
                    if fsinfo_valid(&f) { fat.fsinfo = Some(fs); }
                }
            }
        }
        Ok(fat)
    }

    pub fn list<D: BlockDev + ?Sized>(&self, dev: &mut D, path: &str) -> Result<Vec<DirEntry>, Error> {
        let comps = components(path)?;
        let loc = self.resolve_dir(dev, &comps)?;
        Ok(self.read_dir(dev, loc)?.into_iter().map(|i| i.ent).collect())
    }

    pub fn find<D: BlockDev + ?Sized>(&self, dev: &mut D, path: &str) -> Result<DirEntry, Error> {
        let comps = components(path)?;
        let Some((name, parent)) = comps.split_last() else {
            let root = if self.kind == FatKind::Fat32 { self.root_cluster } else { 0 };
            return Ok(DirEntry { name: String::from("/"), is_dir: true, size: 0, first_cluster: root });
        };
        let loc = self.resolve_dir(dev, parent)?;
        Ok(self.lookup(dev, loc, name)?.ok_or(Error::NotFound)?.ent)
    }

    pub fn read_file<D: BlockDev + ?Sized>(&self, dev: &mut D, path: &str) -> Result<Vec<u8>, Error> {
        let e = self.find(dev, path)?;
        if e.is_dir { return Err(Error::Invalid); }
        if e.size == 0 { return Ok(Vec::new()); }
        let chain = self.chain(dev, e.first_cluster)?;
        let cb = self.cluster_bytes();
        if (chain.len() as u64) < (e.size as u64).div_ceil(cb as u64) { return Err(Error::Corrupt); }
        let mut out = vec![0u8; (e.size as usize).div_ceil(cb) * cb];
        // Read runs of consecutive clusters in one transfer each (bounded by CHUNK sectors).
        let mut i = 0;
        let need = out.len() / cb;
        while i < need {
            let mut j = i + 1;
            let max_clusters = (CHUNK / self.sectors_per_cluster as usize).max(1);
            while j < need && j - i < max_clusters && chain[j] == chain[j - 1] + 1 { j += 1; }
            dev.read(self.cluster_lba(chain[i]), &mut out[i * cb..j * cb])?;
            i = j;
        }
        out.truncate(e.size as usize);
        Ok(out)
    }

    pub fn mkdir_p<D: BlockDev + ?Sized>(&self, dev: &mut D, path: &str) -> Result<(), Error> {
        self.writable()?;
        let comps = components(path)?;
        let mut loc = self.root_loc();
        let mut parent_cluster = 0u32; // ".." of a child of the root is 0, FAT16 and FAT32 alike
        let mut changed = false;
        let mut result = Ok(());
        for name in &comps {
            match self.lookup(dev, loc, name)? {
                Some(it) if it.ent.is_dir => {
                    loc = self.dir_loc(&it.ent)?;
                    parent_cluster = it.ent.first_cluster;
                }
                Some(_) => { result = Err(Error::Exists); break; }
                None => {
                    let short = match to_short(name) { Ok(s) => s, Err(e) => { result = Err(e); break; } };
                    let c = match self.alloc_run(dev, 1) { Ok(c) => c, Err(e) => { result = Err(e); break; } };
                    let mut data = vec![0u8; 64];
                    data[..32].copy_from_slice(&short_entry(b".          ", ATTR_DIR, c, 0));
                    data[32..].copy_from_slice(&short_entry(b"..         ", ATTR_DIR, parent_cluster, 0));
                    self.write_run(dev, c, 1, &data)?;
                    self.set_entries(dev, &mut [(c, self.eoc())])?;
                    changed = true;
                    if let Err(e) = self.add_entry(dev, loc, &short_entry(&short, ATTR_DIR, c, 0)) {
                        let _ = self.set_entries(dev, &mut [(c, 0)]);
                        result = Err(e);
                        break;
                    }
                    loc = Loc::Chain(c);
                    parent_cluster = c;
                }
            }
        }
        if changed { self.sync_fsinfo(dev)?; }
        result
    }

    /// Creates (or replaces, freeing the old chain AFTER the new data and entry are written)
    /// a file whose clusters are ONE contiguous run, and returns where its data lives so the
    /// caller can later rewrite it in place with raw sector writes. The parent directory must
    /// exist (`mkdir_p` first). A new name must fit 8.3; an existing file is replaced under its
    /// existing (possibly long) name. An empty file has no clusters and a zero-length extent.
    pub fn write_file<D: BlockDev + ?Sized>(&self, dev: &mut D, path: &str, data: &[u8]) -> Result<Extent, Error> {
        self.writable()?;
        let comps = components(path)?;
        let (name, parent) = comps.split_last().ok_or(Error::Invalid)?;
        let size = u32::try_from(data.len()).map_err(|_| Error::Invalid)?;
        let loc = self.resolve_dir(dev, parent)?;
        let existing = self.lookup(dev, loc, name)?;
        let (old_chain, short) = match &existing {
            Some(it) if it.ent.is_dir => return Err(Error::Exists),
            Some(it) if it.ent.first_cluster != 0 => (self.chain(dev, it.ent.first_cluster)?, it.short),
            Some(it) => (Vec::new(), it.short),
            None => (Vec::new(), to_short(name)?),
        };
        let n = (size as u64).div_ceil(self.cluster_bytes() as u64) as u32;
        let start = if n > 0 { self.alloc_run(dev, n)? } else { 0 };
        if n > 0 {
            self.write_run(dev, start, n, data)?;
            let mut ents: Vec<(u32, u32)> = (start..start + n).map(|c| (c, c + 1)).collect();
            ents[n as usize - 1].1 = self.eoc();
            self.set_entries(dev, &mut ents)?;
        }
        let r = match &existing {
            Some(it) => {
                // One sector write switches the entry from the old chain to the new one.
                let mut s = [0u8; SECTOR];
                dev.read(it.lba, &mut s)?;
                let e = &mut s[it.off..it.off + 32];
                if e[..11] != it.short { return Err(Error::Corrupt); }
                e[11] |= ATTR_ARCHIVE;
                put16(e, 18, DOS_DATE);
                put16(e, 20, if self.kind == FatKind::Fat32 { (start >> 16) as u16 } else { le16(e, 20) });
                put16(e, 22, DOS_TIME);
                put16(e, 24, DOS_DATE);
                put16(e, 26, start as u16);
                put32(e, 28, size);
                dev.write(it.lba, &s)
            }
            None => self.add_entry(dev, loc, &short_entry(&short, ATTR_ARCHIVE, start, size)),
        };
        if let Err(e) = r {
            if n > 0 {
                let mut free: Vec<(u32, u32)> = (start..start + n).map(|c| (c, 0)).collect();
                let _ = self.set_entries(dev, &mut free);
            }
            return Err(e);
        }
        if !old_chain.is_empty() {
            let mut free: Vec<(u32, u32)> = old_chain.iter().map(|&c| (c, 0)).collect();
            self.set_entries(dev, &mut free)?;
        }
        self.sync_fsinfo(dev)?;
        Ok(Extent {
            first_lba: if n > 0 { self.cluster_lba(start) } else { self.first_data_sector as u64 },
            sectors: n as u64 * self.sectors_per_cluster as u64,
        })
    }

    /// Where an existing file's data lives, only if its chain is one contiguous run;
    /// `Unsupported` if fragmented, `Corrupt` if the chain is broken or too short.
    pub fn extent<D: BlockDev + ?Sized>(&self, dev: &mut D, path: &str) -> Result<Extent, Error> {
        let e = self.find(dev, path)?;
        if e.is_dir { return Err(Error::Invalid); }
        if e.first_cluster == 0 {
            if e.size != 0 { return Err(Error::Corrupt); }
            return Ok(Extent { first_lba: self.first_data_sector as u64, sectors: 0 });
        }
        let chain = self.chain(dev, e.first_cluster)?;
        if (chain.len() as u64) < (e.size as u64).div_ceil(self.cluster_bytes() as u64) { return Err(Error::Corrupt); }
        if chain.windows(2).any(|w| w[1] != w[0] + 1) { return Err(Error::Unsupported); }
        Ok(Extent { first_lba: self.cluster_lba(chain[0]), sectors: chain.len() as u64 * self.sectors_per_cluster as u64 })
    }

    // ---- geometry ----

    /// FAT12 volumes (SexOS's own boot image) are only ever read.
    fn writable(&self) -> Result<(), Error> {
        if self.kind == FatKind::Fat12 { Err(Error::Unsupported) } else { Ok(()) }
    }
    fn cluster_bytes(&self) -> usize { self.sectors_per_cluster as usize * SECTOR }
    fn valid_cluster(&self, c: u32) -> bool { c >= 2 && (c as u64) < self.cluster_count as u64 + 2 }
    fn cluster_lba(&self, c: u32) -> u64 { self.first_data_sector as u64 + (c as u64 - 2) * self.sectors_per_cluster as u64 }
    /// Bytes per FAT entry for FAT16/32 (FAT12's packed 1.5-byte entries go through `entry_pos`).
    fn entry_bytes(&self) -> usize { if self.kind == FatKind::Fat32 { 4 } else { 2 } }
    fn eoc(&self) -> u32 { match self.kind { FatKind::Fat12 => 0xFFF, FatKind::Fat16 => 0xFFFF, FatKind::Fat32 => 0x0FFF_FFFF } }
    fn eoc_min(&self) -> u32 { match self.kind { FatKind::Fat12 => 0xFF8, FatKind::Fat16 => 0xFFF8, FatKind::Fat32 => 0x0FFF_FFF8 } }
    fn root_loc(&self) -> Loc { if self.kind == FatKind::Fat32 { Loc::Chain(self.root_cluster) } else { Loc::Root16 } }
    fn read_copy(&self) -> u32 { self.active_fat.unwrap_or(0) }
    fn fat_lba(&self, copy: u32, sec: u64) -> u64 { self.reserved as u64 + copy as u64 * self.fat_size as u64 + sec }
    /// FAT sector (within one copy) and byte offset of cluster `c`'s entry.
    fn entry_pos(&self, c: u32) -> (u64, usize) {
        let off = match self.kind {
            FatKind::Fat12 => c as u64 + c as u64 / 2,
            _ => c as u64 * self.entry_bytes() as u64,
        };
        (off / SECTOR as u64, (off % SECTOR as u64) as usize)
    }
    /// The entry at byte `off` of `b`; for FAT12, `c` (the cluster) picks the nibble half.
    fn decode_at(&self, b: &[u8], off: usize, c: u32) -> u32 {
        match self.kind {
            FatKind::Fat12 => { let v = le16(b, off) as u32; if c & 1 == 1 { v >> 4 } else { v & 0xFFF } }
            FatKind::Fat16 => le16(b, off) as u32,
            FatKind::Fat32 => le32(b, off) & 0x0FFF_FFFF,
        }
    }
    fn decode(&self, b: &[u8], off: usize) -> u32 { self.decode_at(b, off, 0) }

    // ---- FAT ----

    /// The clusters of the chain starting at `first`, refusing cycles, free or bad links and
    /// out-of-range clusters.
    fn chain<D: BlockDev + ?Sized>(&self, dev: &mut D, first: u32) -> Result<Vec<u32>, Error> {
        let mut seen = vec![0u64; (self.cluster_count as usize + 2).div_ceil(64)];
        let mut out = Vec::new();
        let mut cached = u64::MAX;
        // Two sectors: a packed FAT12 entry can straddle a sector boundary.
        let mut buf = [0u8; 2 * SECTOR];
        let mut c = first;
        loop {
            if !self.valid_cluster(c) { return Err(Error::Corrupt); }
            let (w, bit) = (c as usize / 64, 1u64 << (c % 64));
            if seen[w] & bit != 0 { return Err(Error::Corrupt); }
            seen[w] |= bit;
            out.push(c);
            let (sec, off) = self.entry_pos(c);
            if sec != cached {
                let k = if self.kind == FatKind::Fat12 && sec + 1 < self.fat_size as u64 { 2 } else { 1 };
                dev.read(self.fat_lba(self.read_copy(), sec), &mut buf[..k * SECTOR])?;
                cached = sec;
            }
            let next = self.decode_at(&buf, off, c);
            if next >= self.eoc_min() { return Ok(out); }
            c = next; // 0, 1, bad (..F7) and out-of-range values are all refused by valid_cluster
        }
    }

    /// Sets FAT entries in every FAT copy in use, one read-modify-write per touched sector
    /// per copy, each copy's other entries left as they are.
    fn set_entries<D: BlockDev + ?Sized>(&self, dev: &mut D, ents: &mut [(u32, u32)]) -> Result<(), Error> {
        self.writable()?;
        ents.sort_unstable_by_key(|e| e.0);
        if ents.iter().any(|e| !self.valid_cluster(e.0)) { return Err(Error::Invalid); }
        let copies = match self.active_fat { Some(a) => a..a + 1, None => 0..self.num_fats };
        let mut buf = [0u8; SECTOR];
        for copy in copies {
            let mut i = 0;
            while i < ents.len() {
                let sec = self.entry_pos(ents[i].0).0;
                let lba = self.fat_lba(copy, sec);
                dev.read(lba, &mut buf)?;
                while i < ents.len() && self.entry_pos(ents[i].0).0 == sec {
                    let (_, off) = self.entry_pos(ents[i].0);
                    match self.kind {
                        FatKind::Fat12 => return Err(Error::Unsupported),
                        FatKind::Fat16 => put16(&mut buf, off, ents[i].1 as u16),
                        FatKind::Fat32 => {
                            let keep = le32(&buf, off) & 0xF000_0000;
                            put32(&mut buf, off, keep | (ents[i].1 & 0x0FFF_FFFF));
                        }
                    }
                    i += 1;
                }
                dev.write(lba, &buf)?;
            }
        }
        Ok(())
    }

    /// Counts free clusters and finds the first free one and the first run of `need` free ones.
    fn scan_free<D: BlockDev + ?Sized>(&self, dev: &mut D, need: u32) -> Result<(u32, Option<u32>, Option<u32>), Error> {
        self.writable()?;
        let eb = self.entry_bytes();
        let end = self.cluster_count as u64 + 2;
        let nsec = (end * eb as u64).div_ceil(SECTOR as u64);
        let per = SECTOR / eb;
        let mut buf = vec![0u8; CHUNK * SECTOR];
        let (mut free, mut first, mut found) = (0u32, None, None);
        let (mut run_start, mut run_len) = (0u32, 0u32);
        let mut sec = 0u64;
        while sec < nsec {
            let k = (CHUNK as u64).min(nsec - sec) as usize;
            dev.read(self.fat_lba(self.read_copy(), sec), &mut buf[..k * SECTOR])?;
            for i in 0..k * per {
                let c = sec * per as u64 + i as u64;
                if c < 2 { continue; }
                if c >= end { break; }
                let c = c as u32;
                if self.decode(&buf, i * eb) == 0 {
                    free += 1;
                    first.get_or_insert(c);
                    if run_len == 0 { run_start = c; }
                    run_len += 1;
                    if need > 0 && found.is_none() && run_len >= need { found = Some(run_start); }
                } else {
                    run_len = 0;
                }
            }
            sec += k as u64;
        }
        Ok((free, first, found))
    }

    /// The first cluster of a run of `n` free clusters. Nothing is marked yet.
    fn alloc_run<D: BlockDev + ?Sized>(&self, dev: &mut D, n: u32) -> Result<u32, Error> {
        self.writable()?;
        self.scan_free(dev, n)?.2.ok_or(Error::NoSpace)
    }

    /// Writes `n` clusters from `start`, `data` then zeros.
    fn write_run<D: BlockDev + ?Sized>(&self, dev: &mut D, start: u32, n: u32, data: &[u8]) -> Result<(), Error> {
        let lba0 = self.cluster_lba(start);
        let total = n as u64 * self.sectors_per_cluster as u64;
        let mut buf = vec![0u8; CHUNK * SECTOR];
        let mut s = 0u64;
        while s < total {
            let k = (CHUNK as u64).min(total - s) as usize;
            let b = &mut buf[..k * SECTOR];
            b.fill(0);
            let off = s as usize * SECTOR;
            if off < data.len() {
                let end = data.len().min(off + b.len());
                b[..end - off].copy_from_slice(&data[off..end]);
            }
            dev.write(lba0 + s, b)?;
            s += k as u64;
        }
        Ok(())
    }

    /// Rewrites FSInfo's free count and next-free hint from a full FAT scan, if FSInfo is valid.
    fn sync_fsinfo<D: BlockDev + ?Sized>(&self, dev: &mut D) -> Result<(), Error> {
        let Some(s) = self.fsinfo else { return Ok(()) };
        let (free, first, _) = self.scan_free(dev, 0)?;
        let mut b = [0u8; SECTOR];
        dev.read(s as u64, &mut b)?;
        if !fsinfo_valid(&b) { return Ok(()); }
        put32(&mut b, 488, free);
        put32(&mut b, 492, first.unwrap_or(0xFFFF_FFFF));
        dev.write(s as u64, &b)
    }

    // ---- directories ----

    fn dir_loc(&self, e: &DirEntry) -> Result<Loc, Error> {
        if !e.is_dir { return Err(Error::NotFound); }
        if e.first_cluster == 0 { return Err(Error::Corrupt); }
        Ok(Loc::Chain(e.first_cluster))
    }

    fn resolve_dir<D: BlockDev + ?Sized>(&self, dev: &mut D, comps: &[&str]) -> Result<Loc, Error> {
        let mut loc = self.root_loc();
        for name in comps {
            let it = self.lookup(dev, loc, name)?.ok_or(Error::NotFound)?;
            loc = self.dir_loc(&it.ent)?;
        }
        Ok(loc)
    }

    /// The directory's sectors, and its last cluster (None for the FAT16 root).
    fn dir_sectors<D: BlockDev + ?Sized>(&self, dev: &mut D, loc: Loc) -> Result<(Vec<u64>, Option<u32>), Error> {
        match loc {
            Loc::Root16 => Ok(((0..self.root_dir_sectors as u64).map(|s| self.root_dir_start as u64 + s).collect(), None)),
            Loc::Chain(c) => {
                let chain = self.chain(dev, c)?;
                let spc = self.sectors_per_cluster as u64;
                let secs = chain.iter().flat_map(|&c| (0..spc).map(move |s| self.cluster_lba(c) + s)).collect();
                Ok((secs, chain.last().copied()))
            }
        }
    }

    /// The directory's files and subdirectories (no ".", "..", volume labels or deleted entries).
    fn read_dir<D: BlockDev + ?Sized>(&self, dev: &mut D, loc: Loc) -> Result<Vec<Item>, Error> {
        let (secs, _) = self.dir_sectors(dev, loc)?;
        let mut out = Vec::new();
        let mut b = [0u8; SECTOR];
        let mut lfn = Lfn::default();
        for &lba in &secs {
            dev.read(lba, &mut b)?;
            for off in (0..SECTOR).step_by(32) {
                let e = &b[off..off + 32];
                if e[0] == 0 { return Ok(out); }
                if e[0] == 0xE5 { lfn.reset(); continue; }
                let attr = e[11];
                if attr & 0x3F == ATTR_LFN { lfn.push(e); continue; }
                let mut short = [0u8; 11];
                short.copy_from_slice(&e[..11]);
                let long = lfn.take(&short);
                if attr & ATTR_VOLUME != 0 || short[0] == b'.' { continue; }
                let hi = if self.kind == FatKind::Fat32 { (le16(e, 20) as u32) << 16 } else { 0 };
                let name = long.unwrap_or_else(|| short_display(&short, e[12]));
                out.push(Item {
                    ent: DirEntry { name, is_dir: attr & ATTR_DIR != 0, size: le32(e, 28), first_cluster: hi | le16(e, 26) as u32 },
                    short, lba, off,
                });
            }
        }
        Ok(out)
    }

    fn lookup<D: BlockDev + ?Sized>(&self, dev: &mut D, loc: Loc, name: &str) -> Result<Option<Item>, Error> {
        Ok(self.read_dir(dev, loc)?.into_iter().find(|it| {
            it.ent.name.eq_ignore_ascii_case(name) || short_display(&it.short, 0).eq_ignore_ascii_case(name)
        }))
    }

    /// Writes `raw` into the directory's first free slot, growing a cluster-chained directory
    /// by one zeroed cluster when it is full. The FAT16 root cannot grow: `NoSpace`.
    fn add_entry<D: BlockDev + ?Sized>(&self, dev: &mut D, loc: Loc, raw: &[u8; 32]) -> Result<(), Error> {
        let (secs, last) = self.dir_sectors(dev, loc)?;
        let mut b = [0u8; SECTOR];
        for (i, &lba) in secs.iter().enumerate() {
            dev.read(lba, &mut b)?;
            for off in (0..SECTOR).step_by(32) {
                let first = b[off];
                if first != 0 && first != 0xE5 { continue; }
                if first == 0 {
                    // Taking the end marker: keep an end marker right after the new entry, in
                    // case the free space beyond it was never cleared.
                    if off + 32 < SECTOR {
                        b[off + 32] = 0;
                    } else if let Some(&next) = secs.get(i + 1) {
                        let mut n = [0u8; SECTOR];
                        dev.read(next, &mut n)?;
                        if n[0] != 0 { n[0] = 0; dev.write(next, &n)?; }
                    }
                }
                b[off..off + 32].copy_from_slice(raw);
                return dev.write(lba, &b);
            }
        }
        let Some(last) = last else { return Err(Error::NoSpace) };
        if secs.len() as u64 * SECTOR as u64 >= MAX_DIR_BYTES { return Err(Error::NoSpace); }
        // A full directory: a new cluster holding the entry (zeros after it), marked end of
        // chain, and only then linked from the last cluster. The link makes the entry appear.
        let c = self.alloc_run(dev, 1)?;
        self.write_run(dev, c, 1, raw)?;
        self.set_entries(dev, &mut [(c, self.eoc())])?;
        if let Err(e) = self.set_entries(dev, &mut [(last, c)]) {
            let _ = self.set_entries(dev, &mut [(c, 0)]);
            return Err(e);
        }
        Ok(())
    }
}

/// Collects a VFAT long name across its entries.
#[derive(Default)]
struct Lfn { units: Vec<u16>, expect: u8, sum: u8, active: bool }
impl Lfn {
    fn reset(&mut self) { self.active = false; self.units.clear(); }
    fn push(&mut self, e: &[u8]) {
        let ord = e[0] & 0x1F;
        if e[0] & 0x40 != 0 {
            if ord == 0 || ord > 20 { self.reset(); return; }
            self.units = vec![0xFFFF; 13 * ord as usize];
            self.sum = e[13];
            self.active = true;
        } else if !self.active || ord == 0 || ord != self.expect || e[13] != self.sum {
            self.reset();
            return;
        }
        let base = (ord as usize - 1) * 13;
        let offs = [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30];
        for (k, &o) in offs.iter().enumerate() { self.units[base + k] = le16(e, o); }
        self.expect = ord - 1;
    }
    /// The long name belonging to the short entry `short`, if a complete matching one was read.
    fn take(&mut self, short: &[u8; 11]) -> Option<String> {
        let ok = self.active && self.expect == 0 && self.sum == lfn_checksum(short);
        let units = core::mem::take(&mut self.units);
        self.active = false;
        if !ok { return None; }
        let len = units.iter().position(|&u| u == 0 || u == 0xFFFF).unwrap_or(units.len());
        let s: String = char::decode_utf16(units[..len].iter().copied()).map(|r| r.unwrap_or('\u{FFFD}')).collect();
        if s.is_empty() { None } else { Some(s) }
    }
}

fn lfn_checksum(n: &[u8; 11]) -> u8 {
    n.iter().fold(0u8, |s, &b| ((s & 1) << 7).wrapping_add(s >> 1).wrapping_add(b))
}

fn fsinfo_valid(b: &[u8]) -> bool {
    le32(b, 0) == 0x4161_5252 && le32(b, 484) == 0x6141_7272 && le32(b, 508) == 0xAA55_0000
}

/// "NAME.EXT" from a short entry, honouring the NT lowercase flags in `case` (byte 12).
fn short_display(s: &[u8; 11], case: u8) -> String {
    let conv = |b: u8, lower: bool| {
        let b = if b == 0x05 { 0xE5 } else { b };
        let c = char::from(b);
        if lower { c.to_ascii_lowercase() } else { c }
    };
    let base_len = s[..8].iter().rposition(|&b| b != b' ').map_or(0, |p| p + 1);
    let ext_len = s[8..].iter().rposition(|&b| b != b' ').map_or(0, |p| p + 1);
    let mut out: String = s[..base_len].iter().map(|&b| conv(b, case & 0x08 != 0)).collect();
    if ext_len > 0 {
        out.push('.');
        out.extend(s[8..8 + ext_len].iter().map(|&b| conv(b, case & 0x10 != 0)));
    }
    out
}

fn components(path: &str) -> Result<Vec<&str>, Error> {
    let comps: Vec<&str> = path.split(['/', '\\']).filter(|c| !c.is_empty()).collect();
    if comps.iter().any(|&c| c == "." || c == "..") { return Err(Error::Invalid); }
    Ok(comps)
}

/// The 8.3 short form of `name` (uppercased), or `Invalid` if it does not fit.
fn to_short(name: &str) -> Result<[u8; 11], Error> {
    if !name.is_ascii() { return Err(Error::Invalid); }
    let (base, ext) = match name.rfind('.') { Some(i) => (&name[..i], &name[i + 1..]), None => (name, "") };
    if base.is_empty() || base.len() > 8 || ext.len() > 3 || base.contains('.') { return Err(Error::Invalid); }
    if name.ends_with('.') { return Err(Error::Invalid); }
    let ok = |b: u8| b.is_ascii_alphanumeric() || b"!#$%&'()-@^_`{}~".contains(&b);
    let mut out = [b' '; 11];
    for (i, b) in base.bytes().enumerate() {
        if !ok(b) { return Err(Error::Invalid); }
        out[i] = b.to_ascii_uppercase();
    }
    for (i, b) in ext.bytes().enumerate() {
        if !ok(b) { return Err(Error::Invalid); }
        out[8 + i] = b.to_ascii_uppercase();
    }
    Ok(out)
}

fn short_entry(name: &[u8; 11], attr: u8, cluster: u32, size: u32) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[..11].copy_from_slice(name);
    e[11] = attr;
    put16(&mut e, 14, DOS_TIME);
    put16(&mut e, 16, DOS_DATE);
    put16(&mut e, 18, DOS_DATE);
    put16(&mut e, 20, (cluster >> 16) as u16);
    put16(&mut e, 22, DOS_TIME);
    put16(&mut e, 24, DOS_DATE);
    put16(&mut e, 26, cluster as u16);
    put32(&mut e, 28, size);
    e
}

/// Formats the whole device as FAT32 (a fresh ESP on a blank disk): 512-byte sectors, 2 FATs,
/// 32 reserved sectors, FSInfo at 1, backup boot sector at 6 (backup FSInfo at 7), root
/// directory at cluster 2 holding the volume label. `NoSpace` if the device is too small for
/// a valid FAT32 (at least 65525 clusters of one sector, about 33 MiB).
pub fn format_fat32<D: BlockDev + ?Sized>(dev: &mut D, label: &[u8; 11]) -> Result<(), Error> {
    const RESERVED: u32 = 32;
    const NFATS: u32 = 2;
    let total = dev.sectors().min(u32::MAX as u64) as u32;
    let mut spc: u32 = match total {
        0..=532_480 => 1,          // up to 260 MiB
        532_481..=16_777_216 => 8, // up to 8 GiB
        16_777_217..=33_554_432 => 16,
        33_554_433..=67_108_864 => 32,
        _ => 64,
    };
    let geometry = |spc: u32| -> Option<(u32, u32)> {
        let mut fat_size = 1u32;
        loop {
            let data = total.checked_sub(RESERVED + NFATS * fat_size)?;
            let cc = data / spc;
            let need = ((cc as u64 + 2) * 4).div_ceil(SECTOR as u64) as u32;
            if need <= fat_size { return Some((fat_size, cc)); }
            fat_size = need;
        }
    };
    let (fat_size, cc) = loop {
        match geometry(spc) {
            Some((fs, cc)) if cc >= 65525 => break (fs, cc.min(0x0FFF_FFF5)),
            _ if spc > 1 => spc /= 2,
            _ => return Err(Error::NoSpace),
        }
    };
    let first_data = RESERVED + NFATS * fat_size;
    if first_data as u64 + spc as u64 > total as u64 { return Err(Error::NoSpace); }

    // Boot sector.
    let mut bs = [0u8; SECTOR];
    bs[..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    bs[3..11].copy_from_slice(b"SEXOS   ");
    put16(&mut bs, 11, SECTOR as u16);
    bs[13] = spc as u8;
    put16(&mut bs, 14, RESERVED as u16);
    bs[16] = NFATS as u8;
    bs[21] = 0xF8;
    put16(&mut bs, 24, 63);
    put16(&mut bs, 26, 255);
    put32(&mut bs, 32, total);
    put32(&mut bs, 36, fat_size);
    put32(&mut bs, 44, 2);
    put16(&mut bs, 48, 1);
    put16(&mut bs, 50, 6);
    bs[64] = 0x80;
    bs[66] = 0x29;
    let serial = label.iter().fold(total ^ 0x5E05_2026, |h, &b| h.rotate_left(5) ^ b as u32);
    put32(&mut bs, 67, serial);
    bs[71..82].copy_from_slice(label);
    bs[82..90].copy_from_slice(b"FAT32   ");
    bs[90] = 0xF4; // hlt: a non-bootable volume's boot code
    bs[91..93].copy_from_slice(&[0xEB, 0xFD]); // jmp back to hlt
    bs[510] = 0x55;
    bs[511] = 0xAA;

    let mut fsinfo = [0u8; SECTOR];
    put32(&mut fsinfo, 0, 0x4161_5252);
    put32(&mut fsinfo, 484, 0x6141_7272);
    put32(&mut fsinfo, 488, cc - 1);
    put32(&mut fsinfo, 492, 3);
    put32(&mut fsinfo, 508, 0xAA55_0000);

    // Clear reserved area, FATs and the root cluster, then fill in.
    let zero = vec![0u8; CHUNK * SECTOR];
    let clear_end = first_data as u64 + spc as u64;
    let mut s = 0u64;
    while s < clear_end {
        let k = (CHUNK as u64).min(clear_end - s) as usize;
        dev.write(s, &zero[..k * SECTOR])?;
        s += k as u64;
    }
    let mut fat0 = [0u8; SECTOR];
    put32(&mut fat0, 0, 0x0FFF_FFF8);
    put32(&mut fat0, 4, 0x0FFF_FFFF);
    put32(&mut fat0, 8, 0x0FFF_FFFF);
    for copy in 0..NFATS {
        dev.write((RESERVED + copy * fat_size) as u64, &fat0)?;
    }
    if label != b"NO NAME    " {
        let mut root = [0u8; SECTOR];
        let mut e = short_entry(label, ATTR_VOLUME, 0, 0);
        put16(&mut e, 14, 0);
        put16(&mut e, 16, 0);
        put16(&mut e, 18, 0);
        root[..32].copy_from_slice(&e);
        dev.write(first_data as u64, &root)?;
    }
    for base in [0u64, 6] {
        dev.write(base + 1, &fsinfo)?;
        let mut sig = [0u8; SECTOR];
        sig[510] = 0x55;
        sig[511] = 0xAA;
        dev.write(base + 2, &sig)?;
    }
    // The boot sectors last: until they exist the device is not a FAT volume at all.
    dev.write(6, &bs)?;
    dev.write(0, &bs)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemDev, RoDev};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn pattern(n: usize, seed: u32) -> Vec<u8> {
        let mut x = seed.wrapping_mul(2_654_435_761) | 1;
        (0..n).map(|_| { x ^= x << 13; x ^= x >> 17; x ^= x << 5; x as u8 }).collect()
    }

    fn fresh(mib: usize) -> MemDev<Vec<u8>> {
        let mut d = MemDev(vec![0u8; mib << 20]);
        format_fat32(&mut d, b"SEXOS ESP  ").unwrap();
        d
    }

    fn fsinfo_free(fat: &Fat, d: &mut MemDev<Vec<u8>>) -> u32 {
        let mut b = [0u8; SECTOR];
        d.read(fat.fsinfo.unwrap() as u64, &mut b).unwrap();
        le32(&b, 488)
    }

    #[test]
    fn fat_format_write_read_replace() {
        let mut d = fresh(64);
        let fat = Fat::open(&mut d).unwrap();
        assert_eq!(fat.kind, FatKind::Fat32);
        assert!(fat.cluster_count >= 65525);
        assert_eq!(fat.root_cluster, 2);
        assert_eq!(fsinfo_free(&fat, &mut d), fat.cluster_count - 1);

        fat.mkdir_p(&mut d, "/EFI/SEXOS").unwrap();
        fat.mkdir_p(&mut d, "/efi/sexos").unwrap(); // idempotent, case-insensitive
        fat.mkdir_p(&mut d, "/EFI/BOOT").unwrap();
        let cfg = b"timeout: 0\n".to_vec();
        let big = pattern(300_000, 1);
        let x1 = fat.write_file(&mut d, "/EFI/SEXOS/LIMINE.CFG", &cfg).unwrap();
        let x2 = fat.write_file(&mut d, "/EFI/BOOT/BOOTX64.EFI", &big).unwrap();
        assert!(x1.sectors >= 1);
        assert_eq!(x2.sectors, (big.len() as u64).div_ceil(512));
        assert_eq!(fat.read_file(&mut d, "/EFI/SEXOS/LIMINE.CFG").unwrap(), cfg);
        assert_eq!(fat.read_file(&mut d, "/efi/boot/bootx64.efi").unwrap(), big);
        assert_eq!(fat.extent(&mut d, "/EFI/BOOT/BOOTX64.EFI").unwrap(), x2);

        // Raw sectors at the extent hold the data.
        let mut raw = vec![0u8; x2.sectors as usize * SECTOR];
        d.read(x2.first_lba, &mut raw).unwrap();
        assert_eq!(&raw[..big.len()], &big[..]);

        // Replace with a different size; old chain freed, free count exact.
        let before = fsinfo_free(&fat, &mut d);
        let big2 = pattern(100_000, 2);
        let x3 = fat.write_file(&mut d, "/EFI/BOOT/BOOTX64.EFI", &big2).unwrap();
        assert_ne!(x3.first_lba, x2.first_lba);
        assert_eq!(fat.read_file(&mut d, "/EFI/BOOT/BOOTX64.EFI").unwrap(), big2);
        let after = fsinfo_free(&fat, &mut d);
        assert_eq!(after, before + (x2.sectors - x3.sectors) as u32);
        assert_eq!(after, fat.scan_free(&mut d, 0).unwrap().0);

        // In-place rewrite through the extent is visible through the file system.
        let mut patched = big2.clone();
        patched[..4].copy_from_slice(b"SEX!");
        let mut buf = vec![0u8; x3.sectors as usize * SECTOR];
        buf[..patched.len()].copy_from_slice(&patched);
        d.write(x3.first_lba, &buf).unwrap();
        assert_eq!(fat.read_file(&mut d, "/EFI/BOOT/BOOTX64.EFI").unwrap(), patched);

        // Empty files.
        let x0 = fat.write_file(&mut d, "/EFI/SEXOS/EMPTY", b"").unwrap();
        assert_eq!(x0.sectors, 0);
        assert_eq!(fat.read_file(&mut d, "/EFI/SEXOS/EMPTY").unwrap(), b"");
        fat.write_file(&mut d, "/EFI/SEXOS/EMPTY", b"now full").unwrap();
        assert_eq!(fat.read_file(&mut d, "/EFI/SEXOS/EMPTY").unwrap(), b"now full");

        let mut root: Vec<String> = fat.list(&mut d, "/").unwrap().into_iter().map(|e| e.name).collect();
        root.sort();
        assert_eq!(root, ["EFI"]);
        let efi = fat.list(&mut d, "/EFI").unwrap();
        assert!(efi.iter().all(|e| e.is_dir));
        let mut names: Vec<_> = efi.iter().map(|e| e.name.as_str()).collect();
        names.sort();
        assert_eq!(names, ["BOOT", "SEXOS"]);
        let e = fat.find(&mut d, "/EFI/SEXOS/LIMINE.CFG").unwrap();
        assert_eq!((e.is_dir, e.size), (false, cfg.len() as u32));
        assert!(fat.find(&mut d, "/").unwrap().is_dir);

        // Errors.
        assert_eq!(fat.find(&mut d, "/EFI/NOPE"), Err(Error::NotFound));
        assert_eq!(fat.write_file(&mut d, "/NOPE/X.EFI", b"x"), Err(Error::NotFound));
        assert_eq!(fat.write_file(&mut d, "/EFI/toolongname.efi", b"x"), Err(Error::Invalid));
        assert_eq!(fat.write_file(&mut d, "/EFI/A.B.C", b"x"), Err(Error::Invalid));
        assert_eq!(fat.write_file(&mut d, "/EFI/A+B", b"x"), Err(Error::Invalid));
        assert_eq!(fat.write_file(&mut d, "/EFI/BOOT", b"x"), Err(Error::Exists));
        assert_eq!(fat.mkdir_p(&mut d, "/EFI/SEXOS/LIMINE.CFG/X"), Err(Error::Exists));
        assert_eq!(fat.read_file(&mut d, "/EFI"), Err(Error::Invalid));
        assert_eq!(fat.find(&mut d, "/EFI/../EFI"), Err(Error::Invalid));

        // Read-only access to the same bytes.
        let bytes = d.0.clone();
        let mut ro = RoDev(&bytes);
        let fat2 = Fat::open(&mut ro).unwrap();
        assert_eq!(fat2.read_file(&mut ro, "/EFI/SEXOS/LIMINE.CFG").unwrap(), cfg);
    }

    #[test]
    fn fat_directory_grows() {
        let mut d = fresh(40); // one sector per cluster: 16 entries per cluster
        let fat = Fat::open(&mut d).unwrap();
        assert_eq!(fat.sectors_per_cluster, 1);
        fat.mkdir_p(&mut d, "/MANY").unwrap();
        for i in 0..50 {
            let name = format!("/MANY/F{i}.BIN");
            fat.write_file(&mut d, &name, &pattern(700 + i, i as u32)).unwrap();
        }
        for i in 0..40 { // the root grows too
            fat.mkdir_p(&mut d, &format!("/D{i}")).unwrap();
        }
        assert_eq!(fat.list(&mut d, "/MANY").unwrap().len(), 50);
        assert_eq!(fat.list(&mut d, "/").unwrap().len(), 41);
        for i in 0..50 {
            assert_eq!(fat.read_file(&mut d, &format!("/many/f{i}.bin")).unwrap(), pattern(700 + i, i as u32));
        }
        assert_eq!(fsinfo_free(&fat, &mut d), fat.scan_free(&mut d, 0).unwrap().0);
    }

    #[test]
    fn fat_refuses_bad_volumes() {
        let mut small = MemDev(vec![0u8; 16 << 20]);
        assert_eq!(format_fat32(&mut small, b"NO NAME    "), Err(Error::NoSpace));
        assert_eq!(Fat::open(&mut small).err(), Some(Error::Corrupt));

        let mut d = fresh(64);
        let good = d.0.clone();
        d.0[11] = 0; d.0[12] = 0x10; // 4096-byte sectors
        assert_eq!(Fat::open(&mut d).err(), Some(Error::Unsupported));
        d.0.copy_from_slice(&good);
        d.0[13] = 3; // sectors per cluster not a power of two
        assert_eq!(Fat::open(&mut d).err(), Some(Error::Corrupt));
        d.0.copy_from_slice(&good);
        put32(&mut d.0, 32, 1 << 30); // larger than the device
        assert_eq!(Fat::open(&mut d).err(), Some(Error::Corrupt));
        d.0.copy_from_slice(&good);
        put32(&mut d.0, 36, 10); // FAT too small for the clusters
        assert_eq!(Fat::open(&mut d).err(), Some(Error::Corrupt));
        d.0.copy_from_slice(&good);
        put32(&mut d.0, 44, 0); // root cluster out of range
        assert_eq!(Fat::open(&mut d).err(), Some(Error::Corrupt));

        // A cyclic chain is Corrupt, not a hang.
        d.0.copy_from_slice(&good);
        let fat = Fat::open(&mut d).unwrap();
        fat.mkdir_p(&mut d, "/EFI").unwrap();
        let x = fat.write_file(&mut d, "/EFI/A.BIN", &pattern(5000, 3)).unwrap();
        let first = fat.find(&mut d, "/EFI/A.BIN").unwrap().first_cluster;
        let last = first + (x.sectors / fat.sectors_per_cluster as u64) as u32 - 1;
        fat.set_entries(&mut d, &mut [(last, first)]).unwrap();
        assert_eq!(fat.read_file(&mut d, "/EFI/A.BIN"), Err(Error::Corrupt));
        assert_eq!(fat.extent(&mut d, "/EFI/A.BIN"), Err(Error::Corrupt));
        // ...and a write that would replace it refuses before touching anything.
        let snapshot = d.0.clone();
        assert_eq!(fat.write_file(&mut d, "/EFI/A.BIN", b"x"), Err(Error::Corrupt));
        assert!(d.0 == snapshot);
        // A fragmented chain has no extent.
        let mut d2 = MemDev(good.clone());
        let fat2 = Fat::open(&mut d2).unwrap();
        fat2.mkdir_p(&mut d2, "/EFI").unwrap();
        let c = fat2.write_file(&mut d2, "/EFI/F.BIN", &pattern(3 * 512, 4)).unwrap();
        let fc = ((c.first_lba - fat2.first_data_sector as u64) / fat2.sectors_per_cluster as u64) as u32 + 2;
        let n = (c.sectors / fat2.sectors_per_cluster as u64) as u32;
        fat2.set_entries(&mut d2, &mut [(fc + n - 1, fc + n + 3), (fc + n + 3, fat2.eoc())]).unwrap();
        assert_eq!(fat2.extent(&mut d2, "/EFI/F.BIN"), Err(Error::Unsupported));
    }

    // ---- cross-checks against mtools and dosfstools ----

    fn tool(name: &str) -> Option<PathBuf> {
        for dir in ["/usr/bin", "/bin", "/sbin", "/usr/sbin", "/usr/local/bin"] {
            let p = Path::new(dir).join(name);
            if p.exists() { return Some(p); }
        }
        None
    }

    fn scratch(name: &str) -> PathBuf {
        let base = std::env::temp_dir().join("sex-disk-fat-tests");
        std::fs::create_dir_all(&base).unwrap();
        base.join(format!("{}-{name}", std::process::id()))
    }

    fn mtools(cmd: &str, args: &[&str]) -> std::process::Output {
        let out = Command::new(tool(cmd).unwrap()).args(args).env("MTOOLS_SKIP_CHECK", "1").output().unwrap();
        assert!(out.status.success(), "{cmd} {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        out
    }

    fn fsck_clean(img: &Path) {
        let Some(fsck) = tool("fsck.vfat").or_else(|| tool("fsck.fat")) else { return };
        let out = Command::new(fsck).arg("-n").arg(img).output().unwrap();
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(out.status.success(), "fsck reported problems:\n{text}");
        eprintln!("{text}");
    }

    fn have_tools() -> bool {
        let ok = ["mformat", "mcopy", "mmd", "mtype", "mdir"].iter().all(|t| tool(t).is_some());
        if !ok { eprintln!("mtools missing: skipping cross-check"); }
        ok
    }

    /// An image made by mtools, holding a Windows-style tree, extended by our code.
    fn cross_check(mib: u64, fat32: bool) {
        if !have_tools() { return; }
        let img = scratch(if fat32 { "x32.img" } else { "x16.img" });
        let i = img.to_str().unwrap();
        let _ = std::fs::remove_file(&img);
        std::fs::File::create(&img).unwrap().set_len(mib << 20).unwrap();
        let mut args = vec!["-i", i, "-v", "WINESP"];
        if fat32 { args.push("-F"); }
        args.push("::");
        mtools("mformat", &args);
        mtools("mmd", &["-i", i, "::/EFI", "::/EFI/Microsoft", "::/EFI/Microsoft/Boot"]);
        let win = pattern(150_000, 9);
        // Per-variant names: the FAT16 and FAT32 checks run in parallel threads.
        let src = scratch(if fat32 { "bootmgfw32.efi" } else { "bootmgfw16.efi" });
        std::fs::write(&src, &win).unwrap();
        mtools("mcopy", &["-i", i, src.to_str().unwrap(), "::/EFI/Microsoft/Boot/bootmgfw.efi"]);
        let bcd = scratch(if fat32 { "BCD32" } else { "BCD16" });
        std::fs::write(&bcd, pattern(20_000, 10)).unwrap();
        mtools("mcopy", &["-i", i, bcd.to_str().unwrap(), "::/EFI/Microsoft/Boot/BCD"]);
        let original = std::fs::read(&img).unwrap();

        let mut d = MemDev(original.clone());
        let fat = Fat::open(&mut d).unwrap();
        assert_eq!(fat.kind, if fat32 { FatKind::Fat32 } else { FatKind::Fat16 });
        let ms = fat.list(&mut d, "/efi").unwrap();
        assert!(ms.iter().any(|e| e.name == "Microsoft" && e.is_dir), "{ms:?}");
        let boot = fat.list(&mut d, "/EFI/MICROSOFT/BOOT").unwrap();
        assert!(boot.iter().any(|e| e.name == "bootmgfw.efi"), "{boot:?}");
        assert_eq!(fat.read_file(&mut d, "/EFI/Microsoft/Boot/bootmgfw.efi").unwrap(), win);
        assert_eq!(fat.read_file(&mut d, "/efi/microsoft/boot/BOOTMGFW.EFI").unwrap(), win);

        let loader = pattern(200_000, 11);
        let cfg = b"timeout: 3\n/SexOS\n    protocol: limine\n".to_vec();
        fat.mkdir_p(&mut d, "/EFI/SEXOS").unwrap();
        fat.mkdir_p(&mut d, "/EFI/BOOT").unwrap();
        fat.write_file(&mut d, "/EFI/SEXOS/BOOTX64.EFI", &pattern(1000, 12)).unwrap();
        fat.write_file(&mut d, "/EFI/SEXOS/BOOTX64.EFI", &loader).unwrap(); // replace
        fat.write_file(&mut d, "/EFI/SEXOS/LIMINE.CFG", &cfg).unwrap();
        fat.write_file(&mut d, "/EFI/BOOT/BOOTX64.EFI", &loader).unwrap();
        assert!(fat.extent(&mut d, "/EFI/SEXOS/BOOTX64.EFI").is_ok());
        if !fat32 {
            // The fixed FAT16 root fills up: NoSpace, and the volume stays consistent.
            let mut made = 0;
            loop {
                match fat.write_file(&mut d, &format!("/R{made}.TXT"), b"r") {
                    Ok(_) => made += 1,
                    Err(e) => { assert_eq!(e, Error::NoSpace); break; }
                }
            }
            assert_eq!(made, fat.root_entries as usize - 2); // the label and EFI take two slots
        }
        std::fs::write(&img, &d.0).unwrap();

        assert_eq!(mtools("mtype", &["-i", i, "::/EFI/SEXOS/LIMINE.CFG"]).stdout, cfg);
        assert_eq!(mtools("mtype", &["-i", i, "::/EFI/SEXOS/BOOTX64.EFI"]).stdout, loader);
        assert_eq!(mtools("mtype", &["-i", i, "::/EFI/BOOT/BOOTX64.EFI"]).stdout, loader);
        assert_eq!(mtools("mtype", &["-i", i, "::/EFI/Microsoft/Boot/bootmgfw.efi"]).stdout, win);
        let dir = String::from_utf8(mtools("mdir", &["-i", i, "::/EFI/SEXOS"]).stdout).unwrap();
        assert!(dir.contains("BOOTX64") && dir.contains("LIMINE"), "{dir}");
        let dir = String::from_utf8(mtools("mdir", &["-i", i, "::/EFI"]).stdout).unwrap();
        assert!(dir.contains("Microsoft") && dir.contains("SEXOS") && dir.contains("BOOT"), "{dir}");
        fsck_clean(&img);
        for p in [&img, &src, &bcd] { let _ = std::fs::remove_file(p); }
    }

    #[test]
    fn fat_cross_check_mtools_fat32() { cross_check(64, true); }

    #[test]
    fn fat_cross_check_mtools_fat16() { cross_check(32, false); }

    /// Our formatter's output is accepted by fsck and mtools, and files we write read back.
    #[test]
    fn fat_cross_check_own_format() {
        if !have_tools() { return; }
        for mib in [40u64, 300] {
            let img = scratch(&format!("own{mib}.img"));
            let i = img.to_str().unwrap();
            let mut d = MemDev(vec![0u8; (mib << 20) as usize]);
            format_fat32(&mut d, b"SEXOS      ").unwrap();
            let fat = Fat::open(&mut d).unwrap();
            fat.mkdir_p(&mut d, "/EFI/BOOT").unwrap();
            let data = pattern(123_457, mib as u32);
            fat.write_file(&mut d, "/EFI/BOOT/BOOTX64.EFI", &data).unwrap();
            std::fs::write(&img, &d.0).unwrap();
            fsck_clean(&img);
            assert_eq!(mtools("mtype", &["-i", i, "::/EFI/BOOT/BOOTX64.EFI"]).stdout, data);
            let info = String::from_utf8(mtools("minfo", &["-i", i, "::"]).stdout).unwrap();
            assert!(info.contains("FAT32"), "{info}");
            let _ = std::fs::remove_file(&img);
        }
    }

    /// SexOS's own boot image is FAT12 (scripts/make_boot_image.py: `mformat -T 24576 -h 64
    /// -s 32`): readable, never written.
    #[test]
    fn fat12_boot_image_reads() {
        if !have_tools() { return; }
        let img = scratch("boot12.img");
        let i = img.to_str().unwrap();
        let _ = std::fs::remove_file(&img);
        std::fs::File::create(&img).unwrap().set_len(24576 * 512).unwrap();
        mtools("mformat", &["-i", i, "-T", "24576", "-h", "64", "-s", "32", "::"]);
        mtools("mmd", &["-i", i, "::/EFI", "::/EFI/BOOT", "::/boot"]);
        let loader = pattern(300_000, 21);
        let cfg = b"timeout: 0\n/SexOS\n    protocol: limine\n    kernel_path: boot():/boot/kernel\n".to_vec();
        let kernel = pattern(1_234_567, 22);
        let files: [(&str, &[u8]); 3] = [
            ("::/EFI/BOOT/BOOTX64.EFI", &loader),
            ("::/limine.cfg", &cfg),
            ("::/boot/kernel", &kernel),
        ];
        for (k, (dst, data)) in files.iter().enumerate() {
            let src = scratch(&format!("boot12-src{k}"));
            std::fs::write(&src, data).unwrap();
            mtools("mcopy", &["-i", i, src.to_str().unwrap(), dst]);
            let _ = std::fs::remove_file(&src);
        }
        let bytes = std::fs::read(&img).unwrap();
        let mut d = MemDev(bytes.clone());
        let fat = Fat::open(&mut d).unwrap();
        assert_eq!(fat.kind, FatKind::Fat12);
        assert_eq!(fat.sectors_per_cluster, 8);
        assert_eq!(fat.read_file(&mut d, "/EFI/BOOT/BOOTX64.EFI").unwrap(), loader);
        assert_eq!(fat.read_file(&mut d, "/limine.cfg").unwrap(), cfg);
        assert_eq!(fat.read_file(&mut d, "/LIMINE.CFG").unwrap(), cfg);
        assert_eq!(fat.read_file(&mut d, "/boot/kernel").unwrap(), kernel);
        let mut root: Vec<String> = fat.list(&mut d, "/").unwrap().into_iter().map(|e| e.name).collect();
        root.sort();
        assert_eq!(root, ["EFI", "boot", "limine.cfg"]);
        let e = fat.find(&mut d, "/efi/boot/bootx64.efi").unwrap();
        assert_eq!((e.is_dir, e.size), (false, loader.len() as u32));
        let x = fat.extent(&mut d, "/EFI/BOOT/BOOTX64.EFI").unwrap(); // mtools allocates contiguously
        let mut raw = vec![0u8; x.sectors as usize * SECTOR];
        d.read(x.first_lba, &mut raw).unwrap();
        assert_eq!(&raw[..loader.len()], &loader[..]);
        // Read-only: every write is refused before touching the image.
        assert_eq!(fat.write_file(&mut d, "/EFI/BOOT/BOOTX64.EFI", b"x"), Err(Error::Unsupported));
        assert_eq!(fat.write_file(&mut d, "/NEW.CFG", b"x"), Err(Error::Unsupported));
        assert_eq!(fat.mkdir_p(&mut d, "/EFI/SEXOS"), Err(Error::Unsupported));
        assert!(d.0 == bytes);
        let mut ro = RoDev(&bytes);
        let fat2 = Fat::open(&mut ro).unwrap();
        assert_eq!(fat2.read_file(&mut ro, "/boot/kernel").unwrap(), kernel);
        let _ = std::fs::remove_file(&img);
    }

}
