// SPDX-License-Identifier: GPL-3.0-only
//! FAT12, FAT16 and FAT32, read and write, with long (VFAT) names: what nearly every USB
//! stick, SD card and camera card carries.
//!
//! Writes are ordered so that a stick pulled half way loses at most the change in hand:
//! clusters are allocated and linked before an entry points at them, a renamed entry is
//! written in its new place before the old one is deleted, and every FAT copy is kept
//! the same. Times come from [`Volume::set_now`]; without a clock, files are dated
//! 1980-01-01 as on any FAT device with no clock.
use crate::{days_from_civil, le16, le32, parts, read_bytes, split_last, write_bytes, Disk, Entry, Error, Format, Kind, Result, Stat, Volume, SECTOR};
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

const ATTR_RO: u8 = 0x01;
const ATTR_VOLUME: u8 = 0x08;
const ATTR_DIR: u8 = 0x10;
const ATTR_ARCHIVE: u8 = 0x20;
const ATTR_LFN: u8 = 0x0F;
const SLOT: usize = 32;
const MAX_FILE: u64 = 0xFFFF_FFFF;

/// A FAT boot sector: sane geometry and a jump instruction.
pub fn looks_like_fat(b: &[u8]) -> bool {
    let bps = le16(b, 11);
    let spc = b[13];
    (b[0] == 0xEB || b[0] == 0xE9) && matches!(bps, 512 | 1024 | 2048 | 4096) && spc != 0 && spc.is_power_of_two()
        && le16(b, 14) != 0 && b[16] != 0 && (le16(b, 22) != 0 || le32(b, 36) != 0) && (le16(b, 19) != 0 || le32(b, 32) != 0)
}

/// Where a directory's entries live.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Dir { FixedRoot, Chain(u32) }

/// One directory entry as found: its names, what it points at, and which slots it uses.
#[derive(Clone, Debug)]
struct Found {
    name: String,
    short: [u8; 11],
    attr: u8,
    cluster: u32,
    size: u32,
    /// Slot index of the first long-name slot (or the short entry when there is none).
    first: usize,
    /// Slot index of the short entry.
    main: usize,
    /// Bytes 12..32 of the short entry (case flags, times, cluster, size).
    tail: [u8; 20],
}
impl Found {
    fn is_dir(&self) -> bool { self.attr & ATTR_DIR != 0 }
    fn stat(&self) -> Stat {
        let (time, date) = (le16(&self.tail, 22 - 12 + 0), le16(&self.tail, 24 - 12));
        Stat { kind: if self.is_dir() { Kind::Dir } else { Kind::File }, size: if self.is_dir() { 0 } else { self.size as u64 }, mtime: unix_time(date, time) }
    }
}

/// A directory read whole: its raw slots and the byte on disk each one lives at.
struct Listing { dir: Dir, raw: Vec<u8>, at: Vec<u64> }
impl Listing { fn slots(&self) -> usize { self.at.len() } fn slot(&self, i: usize) -> &[u8] { &self.raw[i * SLOT..(i + 1) * SLOT] } }

pub struct Fat {
    format: Format,
    /// Bytes per FAT sector (512 to 4096, unlike the disk's own 512).
    bps: u64,
    cluster_bytes: u64,
    fat_start: u64,
    fat_bytes: u64,
    fats: u64,
    root_start: u64,
    root_slots: u64,
    data_start: u64,
    /// Highest valid cluster number + 1.
    clusters_end: u32,
    root_cluster: u32,
    fsinfo: Option<u64>,
    label: String,
    next_free: u32,
    free: Option<u32>,
    /// One FAT sector, the one last used, as (FAT-relative sector, bytes).
    cache: Option<(u64, Vec<u8>)>,
    fsinfo_dirty: bool,
    /// FAT date and time to stamp on what changes.
    now: (u16, u16),
}

fn unix_time(date: u16, time: u16) -> u64 {
    if date == 0 { return 0; }
    let (y, m, d) = (1980 + (date >> 9) as i64, ((date >> 5) & 15) as u32, (date & 31) as u32);
    if !(1..=12).contains(&m) || d == 0 { return 0; }
    let days = days_from_civil(y, m, d);
    (days * 86400) as u64 + (time >> 11) as u64 * 3600 + ((time >> 5) & 63) as u64 * 60 + (time & 31) as u64 * 2
}
/// Unix seconds to a FAT (date, time); before 1980 is 1980-01-01.
fn fat_time(unix: u64) -> (u16, u16) {
    let days = (unix / 86400) as i64;
    let secs = unix % 86400;
    // Civil from days (Howard Hinnant's algorithm).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u16;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u16;
    let y = yoe + era * 400 + (m <= 2) as i64;
    if !(1980..=2107).contains(&y) { return (0x21, 0); }
    (((y - 1980) as u16) << 9 | m << 5 | d, ((secs / 3600) as u16) << 11 | (((secs / 60) % 60) as u16) << 5 | ((secs % 60) / 2) as u16)
}

/// Characters a long name may not hold.
fn bad_char(c: char) -> bool { (c as u32) < 0x20 || "\"*/:<>?\\|".contains(c) }
fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || a.chars().flat_map(char::to_lowercase).eq(b.chars().flat_map(char::to_lowercase))
}
/// The 8.3 name as shown: `NAME.EXT`, with the lowercase flags Windows sets.
fn short_display(s: &[u8; 11], flags: u8) -> String {
    let mut out = String::new();
    let base = s[..8].iter().rposition(|c| *c != b' ').map_or(0, |i| i + 1);
    let ext = s[8..].iter().rposition(|c| *c != b' ').map_or(0, |i| i + 1);
    for (i, c) in s[..base].iter().enumerate() {
        let c = if i == 0 && *c == 0x05 { 0xE5 } else { *c };
        out.push(if flags & 0x08 != 0 { c.to_ascii_lowercase() } else { c } as char);
    }
    if ext > 0 {
        out.push('.');
        for c in &s[8..8 + ext] { out.push(if flags & 0x10 != 0 { c.to_ascii_lowercase() } else { *c } as char); }
    }
    out
}
fn short_sum(s: &[u8; 11]) -> u8 { s.iter().fold(0u8, |sum, c| sum.rotate_right(1).wrapping_add(*c)) }
fn short_char_ok(c: u8) -> bool { c.is_ascii_alphanumeric() || b"!#$%&'()-@^_`{}~".contains(&c) }
/// The name as an exact 8.3 entry, if it is one (any case, with case flags).
fn exact_short(name: &str) -> Option<([u8; 11], u8)> {
    let b = name.as_bytes();
    let (base, ext) = match b.iter().rposition(|c| *c == b'.') { Some(i) => (&b[..i], &b[i + 1..]), None => (b, &b[..0]) };
    if base.is_empty() || base.len() > 8 || ext.len() > 3 || (b.contains(&b'.') && ext.is_empty()) { return None; }
    if !base.iter().chain(ext).all(|c| short_char_ok(*c)) { return None; }
    let case = |p: &[u8]| -> Option<u8> {
        let lower = p.iter().any(u8::is_ascii_lowercase);
        let upper = p.iter().any(u8::is_ascii_uppercase);
        if lower && upper { None } else { Some(lower as u8) }
    };
    let (bl, el) = (case(base)?, case(ext)?);
    let mut s = [b' '; 11];
    for (i, c) in base.iter().enumerate() { s[i] = c.to_ascii_uppercase(); }
    for (i, c) in ext.iter().enumerate() { s[8 + i] = c.to_ascii_uppercase(); }
    if s[0] == 0xE5 { s[0] = 0x05; }
    Some((s, bl * 0x08 | el * 0x10))
}
/// `BASIS~N.EXT` for a long name: uppercase, invalid characters as `_`.
fn basis(name: &str) -> ([u8; 8], usize, [u8; 3], usize) {
    let trimmed = name.trim_start_matches('.');
    let (b, e) = match trimmed.rfind('.') { Some(i) => (&trimmed[..i], &trimmed[i + 1..]), None => (trimmed, "") };
    let map = |c: char| -> Option<u8> {
        if c == ' ' || c == '.' { return None; }
        let u = c.to_ascii_uppercase();
        Some(if u.is_ascii() && short_char_ok(u as u8) { u as u8 } else { b'_' })
    };
    let (mut base, mut bn, mut ext, mut en) = ([b' '; 8], 0, [b' '; 3], 0);
    for c in b.chars().filter_map(map) { if bn < 8 { base[bn] = c; bn += 1; } }
    for c in e.chars().filter_map(map) { if en < 3 { ext[en] = c; en += 1; } }
    if bn == 0 { base[0] = b'_'; bn = 1; }
    (base, bn, ext, en)
}

impl Fat {
    pub fn mount(disk: &mut dyn Disk) -> Result<Self> {
        let mut b = [0u8; SECTOR];
        disk.read(0, &mut b)?;
        if !looks_like_fat(&b) || b[510] != 0x55 || b[511] != 0xAA { return Err(Error::Unsupported); }
        let bps = le16(&b, 11) as u64;
        let spc = b[13] as u64;
        let reserved = le16(&b, 14) as u64;
        let fats = b[16] as u64;
        let root_slots = le16(&b, 17) as u64;
        let total = if le16(&b, 19) != 0 { le16(&b, 19) as u64 } else { le32(&b, 32) as u64 };
        let fat_sectors = if le16(&b, 22) != 0 { le16(&b, 22) as u64 } else { le32(&b, 36) as u64 };
        let root_sectors = (root_slots * SLOT as u64).div_ceil(bps);
        let data_sector = reserved + fats * fat_sectors + root_sectors;
        if data_sector >= total || total * bps > disk.sectors() * SECTOR as u64 + bps { return Err(Error::Corrupt); }
        let clusters = (total - data_sector) / spc;
        let format = if clusters < 4085 { Format::Fat12 } else if clusters < 65525 { Format::Fat16 } else { Format::Fat32 };
        let need = match format { Format::Fat12 => (clusters + 2) * 3 / 2, Format::Fat16 => (clusters + 2) * 2, _ => (clusters + 2) * 4 };
        if fat_sectors * bps < need { return Err(Error::Corrupt); }
        let (label_at, root_cluster, fsinfo) = if format == Format::Fat32 {
            if root_slots != 0 { return Err(Error::Corrupt); }
            let fsinfo = le16(&b, 48) as u64;
            (71, le32(&b, 44) & 0x0FFF_FFFF, (fsinfo != 0 && fsinfo != 0xFFFF && fsinfo < reserved).then_some(fsinfo * bps))
        } else { (43, 0, None) };
        let mut fat = Fat {
            format, bps, cluster_bytes: spc * bps, fat_start: reserved * bps, fat_bytes: fat_sectors * bps, fats,
            root_start: (reserved + fats * fat_sectors) * bps, root_slots, data_start: data_sector * bps,
            clusters_end: (clusters + 2).min(0x0FFF_FFF7) as u32, root_cluster, fsinfo, label: String::new(), next_free: 2, free: None,
            cache: None, fsinfo_dirty: false, now: (0x21, 0),
        };
        if format == Format::Fat32 && !(2..fat.clusters_end).contains(&root_cluster) { return Err(Error::Corrupt); }
        // FSInfo's hints; the free count is recounted anyway before it is trusted.
        if let Some(at) = fat.fsinfo {
            let mut s = [0u8; SECTOR];
            read_bytes(disk, at, &mut s)?;
            if le32(&s, 0) == 0x4161_5252 && le32(&s, 484) == 0x6141_7272 {
                let hint = le32(&s, 492);
                if (2..fat.clusters_end).contains(&hint) { fat.next_free = hint; }
            }
        }
        // The label: the root's volume entry wins over the boot sector's copy.
        let boot_label = trim_label(&b[label_at..label_at + 11]);
        fat.label = boot_label;
        if let Ok(root) = fat.load(disk, fat.root_dir()) {
            for i in 0..root.slots() {
                let s = root.slot(i);
                if s[0] == 0 { break; }
                if s[0] != 0xE5 && s[11] & 0x3F == ATTR_VOLUME { fat.label = trim_label(&s[..11]); break; }
            }
        }
        Ok(fat)
    }
    fn root_dir(&self) -> Dir { if self.format == Format::Fat32 { Dir::Chain(self.root_cluster) } else { Dir::FixedRoot } }
    fn cluster_at(&self, c: u32) -> u64 { self.data_start + (c as u64 - 2) * self.cluster_bytes }
    fn eoc(&self) -> u32 { match self.format { Format::Fat12 => 0xFFF, Format::Fat16 => 0xFFFF, _ => 0x0FFF_FFFF } }
    fn is_end(&self, v: u32) -> bool { v < 2 || v >= self.clusters_end }

    /// One FAT byte, through a one-sector cache of the first FAT.
    fn fat_byte(&mut self, disk: &mut dyn Disk, at: u64) -> Result<u8> {
        let sector = at / self.bps;
        if self.cache.as_ref().map(|c| c.0) != Some(sector) {
            let mut buf = vec![0u8; self.bps as usize];
            read_bytes(disk, self.fat_start + sector * self.bps, &mut buf)?;
            self.cache = Some((sector, buf));
        }
        Ok(self.cache.as_ref().unwrap().1[(at % self.bps) as usize])
    }
    fn get(&mut self, disk: &mut dyn Disk, c: u32) -> Result<u32> {
        if !(2..self.clusters_end).contains(&c) { return Err(Error::Corrupt); }
        Ok(match self.format {
            Format::Fat12 => {
                let at = c as u64 * 3 / 2;
                let v = self.fat_byte(disk, at)? as u32 | (self.fat_byte(disk, at + 1)? as u32) << 8;
                if c & 1 == 1 { v >> 4 } else { v & 0xFFF }
            }
            Format::Fat16 => { let at = c as u64 * 2; self.fat_byte(disk, at)? as u32 | (self.fat_byte(disk, at + 1)? as u32) << 8 }
            _ => {
                let at = c as u64 * 4;
                let mut v = 0u32;
                for i in 0..4 { v |= (self.fat_byte(disk, at + i)? as u32) << (8 * i); }
                v & 0x0FFF_FFFF
            }
        })
    }
    /// Sets one entry in every FAT copy.
    fn set(&mut self, disk: &mut dyn Disk, c: u32, value: u32) -> Result<()> {
        if !(2..self.clusters_end).contains(&c) { return Err(Error::Corrupt); }
        let (at, bytes): (u64, Vec<u8>) = match self.format {
            Format::Fat12 => {
                let at = c as u64 * 3 / 2;
                let old = self.fat_byte(disk, at)? as u16 | (self.fat_byte(disk, at + 1)? as u16) << 8;
                let v = if c & 1 == 1 { old & 0x000F | (value as u16) << 4 } else { old & 0xF000 | value as u16 & 0xFFF };
                (at, v.to_le_bytes().to_vec())
            }
            Format::Fat16 => (c as u64 * 2, (value as u16).to_le_bytes().to_vec()),
            _ => {
                let at = c as u64 * 4;
                let mut high = 0u32;
                for i in 0..4 { high |= (self.fat_byte(disk, at + i)? as u32) << (8 * i); }
                (at, (high & 0xF000_0000 | value & 0x0FFF_FFFF).to_le_bytes().to_vec())
            }
        };
        for copy in 0..self.fats { write_bytes(disk, self.fat_start + copy * self.fat_bytes + at, &bytes)?; }
        // Keep the cached sector true (an entry may straddle two sectors in FAT12).
        if let Some((sector, buf)) = self.cache.as_mut() {
            for (i, b) in bytes.iter().enumerate() {
                let byte = at + i as u64;
                if byte / self.bps == *sector { buf[(byte % self.bps) as usize] = *b; }
            }
        }
        Ok(())
    }
    /// Every cluster of a chain, in order.
    fn chain(&mut self, disk: &mut dyn Disk, first: u32) -> Result<Vec<u32>> {
        let mut out = Vec::new();
        let mut c = first;
        while !self.is_end(c) {
            if out.len() as u32 >= self.clusters_end { return Err(Error::Corrupt); }
            out.push(c);
            c = self.get(disk, c)?;
        }
        // A free or reserved value inside a chain: it was cut short.
        if !out.is_empty() && c < 2 { return Err(Error::Corrupt); }
        Ok(out)
    }
    fn count_free(&mut self, disk: &mut dyn Disk) -> Result<u32> {
        if let Some(f) = self.free { return Ok(f); }
        let mut n = 0;
        for c in 2..self.clusters_end { if self.get(disk, c)? == 0 { n += 1; } }
        self.free = Some(n);
        Ok(n)
    }
    /// A free cluster, marked end-of-chain, linked after `prev` if there is one.
    fn allocate(&mut self, disk: &mut dyn Disk, prev: Option<u32>, zero: bool) -> Result<u32> {
        let span = self.clusters_end - 2;
        let start = if (2..self.clusters_end).contains(&self.next_free) { self.next_free } else { 2 };
        let mut found = None;
        for i in 0..span {
            let c = 2 + (start - 2 + i) % span;
            if self.get(disk, c)? == 0 { found = Some(c); break; }
        }
        let c = found.ok_or(Error::NoSpace)?;
        if zero { self.zero_cluster(disk, c)?; }
        let eoc = self.eoc();
        self.set(disk, c, eoc)?;
        if let Some(p) = prev { self.set(disk, p, c)?; }
        self.next_free = if c + 1 < self.clusters_end { c + 1 } else { 2 };
        if let Some(f) = self.free.as_mut() { *f = f.saturating_sub(1); }
        self.fsinfo_dirty = true;
        Ok(c)
    }
    fn free_chain(&mut self, disk: &mut dyn Disk, first: u32) -> Result<()> {
        for c in self.chain(disk, first)? {
            self.set(disk, c, 0)?;
            if let Some(f) = self.free.as_mut() { *f += 1; }
        }
        self.fsinfo_dirty = true;
        Ok(())
    }
    fn zero_cluster(&mut self, disk: &mut dyn Disk, c: u32) -> Result<()> {
        let zeros = vec![0u8; self.cluster_bytes as usize];
        write_bytes(disk, self.cluster_at(c), &zeros)
    }

    fn load(&mut self, disk: &mut dyn Disk, dir: Dir) -> Result<Listing> {
        let mut raw = Vec::new();
        let mut at = Vec::new();
        match dir {
            Dir::FixedRoot => {
                raw.resize(self.root_slots as usize * SLOT, 0);
                read_bytes(disk, self.root_start, &mut raw)?;
                for i in 0..self.root_slots { at.push(self.root_start + i * SLOT as u64); }
            }
            Dir::Chain(first) => {
                // A directory can't exceed 65536 entries (2 MiB).
                for c in self.chain(disk, first)? {
                    let from = raw.len();
                    raw.resize(from + self.cluster_bytes as usize, 0);
                    read_bytes(disk, self.cluster_at(c), &mut raw[from..])?;
                    let base = self.cluster_at(c);
                    for i in 0..self.cluster_bytes / SLOT as u64 { at.push(base + i * SLOT as u64); }
                    if raw.len() > 2 * 1024 * 1024 { return Err(Error::Corrupt); }
                }
            }
        }
        Ok(Listing { dir, raw, at })
    }
    /// Entries in a listing, long names assembled; `.`, `..` and the label left out.
    fn entries(&self, l: &Listing) -> Vec<Found> {
        let mut out = Vec::new();
        let mut lfn: Vec<u16> = Vec::new();
        let (mut lfn_first, mut lfn_sum, mut lfn_next) = (0usize, 0u8, 0u8);
        for i in 0..l.slots() {
            let s = l.slot(i);
            if s[0] == 0 { break; }
            if s[0] == 0xE5 { lfn_next = 0; continue; }
            if s[11] & 0x3F == ATTR_LFN {
                let seq = s[0] & 0x1F;
                if s[0] & 0x40 != 0 {
                    lfn.clear();
                    lfn.resize(seq as usize * 13, 0xFFFF);
                    lfn_first = i; lfn_sum = s[13]; lfn_next = seq;
                }
                if seq == 0 || seq != lfn_next || s[13] != lfn_sum { lfn_next = 0; continue; }
                let base = (seq as usize - 1) * 13;
                for (k, off) in [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30].iter().enumerate() { lfn[base + k] = le16(s, *off); }
                lfn_next = seq - 1;
                continue;
            }
            let mut short = [0u8; 11];
            short.copy_from_slice(&s[..11]);
            let has_lfn = lfn_next == 0 && !lfn.is_empty() && lfn_sum == short_sum(&short) && i > 0;
            let name = if has_lfn {
                let end = lfn.iter().position(|c| *c == 0 || *c == 0xFFFF).unwrap_or(lfn.len());
                char::decode_utf16(lfn[..end].iter().copied()).map(|r| r.unwrap_or('\u{FFFD}')).collect()
            } else { short_display(&short, s[12]) };
            let first = if has_lfn { lfn_first } else { i };
            lfn.clear(); lfn_next = 0;
            if s[11] & ATTR_VOLUME != 0 || &short == b".          " || &short == b"..         " { continue; }
            let cluster = (le16(s, 20) as u32) << 16 & if self.format == Format::Fat32 { 0x0FFF_0000 } else { 0 } | le16(s, 26) as u32;
            let mut tail = [0u8; 20];
            tail.copy_from_slice(&s[12..32]);
            out.push(Found { name, short, attr: s[11], cluster, size: le32(s, 28), first, main: i, tail });
        }
        out
    }
    /// Walks a path to the directory holding its last part.
    fn dir_of(&mut self, disk: &mut dyn Disk, dirs: &[&[u8]]) -> Result<Dir> {
        let mut dir = self.root_dir();
        for part in dirs {
            let l = self.load(disk, dir)?;
            let name = core::str::from_utf8(part).map_err(|_| Error::Name)?;
            let f = self.entries(&l).into_iter().find(|e| same_name(&e.name, name)).ok_or(Error::NotFound)?;
            if !f.is_dir() { return Err(Error::NotDir); }
            dir = if f.cluster == 0 { self.root_dir() } else { Dir::Chain(f.cluster) };
        }
        Ok(dir)
    }
    /// The entry a path names, with its directory's listing.
    fn find(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<(Listing, Found)> {
        let (dirs, last) = split_last(path)?;
        let dir = self.dir_of(disk, &dirs)?;
        let l = self.load(disk, dir)?;
        let name = core::str::from_utf8(last).map_err(|_| Error::Name)?;
        let f = self.entries(&l).into_iter().find(|e| same_name(&e.name, name)).ok_or(Error::NotFound)?;
        Ok((l, f))
    }
    fn file(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<(Listing, Found)> {
        let (l, f) = self.find(disk, path)?;
        if f.is_dir() { return Err(Error::IsDir); }
        Ok((l, f))
    }
    /// Rewrites a short entry's cluster, size and modification time.
    fn update(&mut self, disk: &mut dyn Disk, l: &Listing, f: &Found, cluster: u32, size: u32) -> Result<()> {
        let mut s = [0u8; SLOT];
        s.copy_from_slice(l.slot(f.main));
        s[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
        s[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
        s[28..32].copy_from_slice(&size.to_le_bytes());
        s[22..24].copy_from_slice(&self.now.1.to_le_bytes());
        s[24..26].copy_from_slice(&self.now.0.to_le_bytes());
        s[18..20].copy_from_slice(&self.now.0.to_le_bytes());
        s[11] |= ATTR_ARCHIVE;
        write_bytes(disk, l.at[f.main], &s)
    }
    fn delete_slots(&mut self, disk: &mut dyn Disk, l: &Listing, f: &Found) -> Result<()> {
        for i in f.first..=f.main { write_bytes(disk, l.at[i], &[0xE5])?; }
        Ok(())
    }

    /// Writes a new entry set for `name` into `dir`, growing the directory if it must.
    /// `tail` is bytes 12..32 of the short entry (times, cluster, size).
    fn add(&mut self, disk: &mut dyn Disk, dir: Dir, name: &str, attr: u8, mut tail: [u8; 20]) -> Result<()> {
        if name.is_empty() || name.chars().any(bad_char) || name.ends_with(' ') || name.ends_with('.') { return Err(Error::Name); }
        let units: Vec<u16> = name.encode_utf16().collect();
        if units.len() > 255 { return Err(Error::Name); }
        let mut l = self.load(disk, dir)?;
        let existing = self.entries(&l);
        if existing.iter().any(|e| same_name(&e.name, name)) { return Err(Error::Exists); }
        let taken = |s: &[u8; 11]| (0..l.slots()).take_while(|i| l.slot(*i)[0] != 0).any(|i| l.slot(i)[..11] == s[..] && l.slot(i)[11] & 0x3F != ATTR_LFN);
        let (short, flags, long) = match exact_short(name) {
            Some((s, f)) if !taken(&s) => (s, f, false),
            _ => {
                let (base, bn, ext, en) = basis(name);
                let mut found = None;
                for n in 1..1_000_000u32 {
                    let digits = { let mut d = [0u8; 7]; let mut k = 7; let mut v = n; while v > 0 { k -= 1; d[k] = b'0' + (v % 10) as u8; v /= 10; } (d, k) };
                    let tag_len = 1 + (7 - digits.1);
                    let keep = bn.min(8 - tag_len);
                    let mut s = [b' '; 11];
                    s[..keep].copy_from_slice(&base[..keep]);
                    s[keep] = b'~';
                    s[keep + 1..keep + tag_len].copy_from_slice(&digits.0[digits.1..]);
                    s[8..8 + en].copy_from_slice(&ext[..en]);
                    if !taken(&s) { found = Some(s); break; }
                }
                (found.ok_or(Error::NoSpace)?, 0, true)
            }
        };
        tail[0] = flags;
        let lfn_slots = if long { units.len().div_ceil(13) } else { 0 };
        let need = lfn_slots + 1;
        // A run of free slots, or the end of the directory.
        let mut start = None;
        let mut run = 0;
        for i in 0..l.slots() {
            let b = l.slot(i)[0];
            if b == 0 { if l.slots() - i >= need { start = Some(i - run); } break; }
            if b == 0xE5 { run += 1; if run == need { start = Some(i + 1 - need); break; } } else { run = 0; }
        }
        let start = match start {
            Some(s) => s,
            None => {
                let Dir::Chain(first) = dir else { return Err(Error::NoSpace) };
                // Grow by whole clusters; the new space starts with zeros (free, end).
                let begin = (0..l.slots()).find(|i| l.slot(*i)[0] == 0).unwrap_or(l.slots());
                let free_tail = l.slots() - begin;
                let per = (self.cluster_bytes / SLOT as u64) as usize;
                let more = (need - free_tail.min(need)).div_ceil(per);
                if l.slots() + more * per > 65536 { return Err(Error::NoSpace); }
                let mut last = *self.chain(disk, first)?.last().ok_or(Error::Corrupt)?;
                for _ in 0..more { last = self.allocate(disk, Some(last), true)?; }
                l = self.load(disk, dir)?;
                begin
            }
        };
        let sum = short_sum(&short);
        for k in 0..lfn_slots {
            let seq = (lfn_slots - k) as u8;
            let mut s = [0u8; SLOT];
            s[0] = seq | if k == 0 { 0x40 } else { 0 };
            s[11] = ATTR_LFN;
            s[13] = sum;
            let base = (seq as usize - 1) * 13;
            for (j, off) in [1, 3, 5, 7, 9, 14, 16, 18, 20, 22, 24, 28, 30].iter().enumerate() {
                let c = match units.get(base + j) { Some(c) => *c, None if base + j == units.len() => 0, None => 0xFFFF };
                s[*off..*off + 2].copy_from_slice(&c.to_le_bytes());
            }
            write_bytes(disk, l.at[start + k], &s)?;
        }
        let mut s = [0u8; SLOT];
        s[..11].copy_from_slice(&short);
        s[11] = attr;
        s[12..32].copy_from_slice(&tail);
        write_bytes(disk, l.at[start + lfn_slots], &s)
    }
    fn new_tail(&self, cluster: u32, size: u32) -> [u8; 20] {
        let mut t = [0u8; 20];
        t[2..4].copy_from_slice(&self.now.1.to_le_bytes());
        t[4..6].copy_from_slice(&self.now.0.to_le_bytes());
        t[6..8].copy_from_slice(&self.now.0.to_le_bytes());
        t[8..10].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
        t[10..12].copy_from_slice(&self.now.1.to_le_bytes());
        t[12..14].copy_from_slice(&self.now.0.to_le_bytes());
        t[14..16].copy_from_slice(&(cluster as u16).to_le_bytes());
        t[16..20].copy_from_slice(&size.to_le_bytes());
        t
    }
    /// Makes a file `len` bytes long: clusters added (and the new bytes zeroed) or freed.
    /// Returns the (possibly new) first cluster.
    /// Bytes from `old` up to `zero_end` are zeroed; past that the caller writes them.
    fn resize(&mut self, disk: &mut dyn Disk, first: u32, old: u64, len: u64, zero_end: u64) -> Result<u32> {
        if len > MAX_FILE { return Err(Error::TooBig); }
        let have = if first == 0 { Vec::new() } else { self.chain(disk, first)? };
        let need = len.div_ceil(self.cluster_bytes) as usize;
        let mut first = first;
        if need > have.len() {
            if (need - have.len()) as u32 > self.count_free(disk)? { return Err(Error::NoSpace); }
            let mut last = have.last().copied();
            for _ in have.len()..need {
                let c = self.allocate(disk, last, false)?;
                if last.is_none() { first = c; }
                last = Some(c);
            }
        } else if need < have.len() {
            if need == 0 { self.free_chain(disk, first)?; first = 0; } else {
                let eoc = self.eoc();
                self.set(disk, have[need - 1], eoc)?;
                self.free_chain(disk, have[need])?;
            }
        }
        let zero_end = zero_end.min(len);
        let mut at = old;
        while at < zero_end {
            let take = (zero_end - at).min(256 * 1024);
            self.write_data(disk, first, at, &vec![0u8; take as usize])?;
            at += take;
        }
        Ok(first)
    }
    /// Copies bytes into a chain long enough to hold them.
    fn write_data(&mut self, disk: &mut dyn Disk, first: u32, offset: u64, data: &[u8]) -> Result<()> {
        if data.is_empty() { return Ok(()); }
        let chain = self.chain(disk, first)?;
        let mut done = 0usize;
        while done < data.len() {
            let at = offset + done as u64;
            let index = (at / self.cluster_bytes) as usize;
            let within = at % self.cluster_bytes;
            // Neighbouring clusters that are also neighbours on disk go in one write.
            let mut run = 1;
            while index + run < chain.len() && chain[index + run] == chain[index] + run as u32 { run += 1; }
            let c = *chain.get(index).ok_or(Error::Corrupt)?;
            let take = ((run as u64 * self.cluster_bytes - within) as usize).min(data.len() - done);
            write_bytes(disk, self.cluster_at(c) + within, &data[done..done + take])?;
            done += take;
        }
        Ok(())
    }
}
fn trim_label(b: &[u8]) -> String {
    let end = b.iter().rposition(|c| *c != b' ' && *c != 0).map_or(0, |i| i + 1);
    if &b[..end] == b"NO NAME" { return String::new(); }
    b[..end].iter().map(|c| *c as char).collect()
}

impl Volume for Fat {
    fn format(&self) -> Format { self.format }
    fn label(&self) -> String { self.label.clone() }
    fn read_only(&self) -> bool { false }
    fn set_now(&mut self, unix: u64) { self.now = fat_time(unix); }
    fn statfs(&mut self, disk: &mut dyn Disk) -> Result<(u64, u64)> {
        let free = self.count_free(disk)? as u64;
        Ok(((self.clusters_end as u64 - 2) * self.cluster_bytes, free * self.cluster_bytes))
    }
    fn stat(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<Stat> {
        if parts(path)?.is_empty() { return Ok(Stat { kind: Kind::Dir, size: 0, mtime: 0 }); }
        Ok(self.find(disk, path)?.1.stat())
    }
    fn list(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<Vec<Entry>> {
        let dirs = parts(path)?;
        let dir = self.dir_of(disk, &dirs)?;
        let l = self.load(disk, dir)?;
        Ok(self.entries(&l).into_iter().map(|e| { let s = e.stat(); Entry { name: e.name, kind: s.kind, size: s.size } }).collect())
    }
    fn read(&mut self, disk: &mut dyn Disk, path: &[u8], offset: u64, buf: &mut [u8]) -> Result<usize> {
        let (_, f) = self.file(disk, path)?;
        let size = f.size as u64;
        if offset >= size || buf.is_empty() { return Ok(0); }
        let want = (buf.len() as u64).min(size - offset) as usize;
        let chain = self.chain(disk, f.cluster)?;
        let mut done = 0;
        while done < want {
            let at = offset + done as u64;
            let index = (at / self.cluster_bytes) as usize;
            let within = at % self.cluster_bytes;
            let c = *chain.get(index).ok_or(Error::Corrupt)?;
            let mut run = 1;
            while index + run < chain.len() && chain[index + run] == c + run as u32 { run += 1; }
            let take = ((run as u64 * self.cluster_bytes - within) as usize).min(want - done);
            read_bytes(disk, self.cluster_at(c) + within, &mut buf[done..done + take])?;
            done += take;
        }
        Ok(want)
    }
    fn write(&mut self, disk: &mut dyn Disk, path: &[u8], offset: u64, data: &[u8]) -> Result<usize> {
        let (l, f) = self.file(disk, path)?;
        if f.attr & ATTR_RO != 0 { return Err(Error::ReadOnly); }
        let end = offset.checked_add(data.len() as u64).ok_or(Error::TooBig)?;
        let old = f.size as u64;
        let mut first = f.cluster;
        if end > old { first = self.resize(disk, first, old, end, offset)?; }
        self.write_data(disk, first, offset, data)?;
        // A power cut must never leave the entry pointing at clusters the FAT does not
        // hold yet, or a size covering bytes not yet written: those reach the stick first.
        if end > old { disk.flush()?; }
        self.update(disk, &l, &f, first, end.max(old) as u32)?;
        Ok(data.len())
    }
    fn create(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<()> {
        let (dirs, last) = split_last(path)?;
        let dir = self.dir_of(disk, &dirs)?;
        let name = core::str::from_utf8(last).map_err(|_| Error::Name)?;
        let tail = self.new_tail(0, 0);
        self.add(disk, dir, name, ATTR_ARCHIVE, tail)
    }
    fn mkdir(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<()> {
        let (dirs, last) = split_last(path)?;
        let dir = self.dir_of(disk, &dirs)?;
        let name = core::str::from_utf8(last).map_err(|_| Error::Name)?;
        if name.is_empty() || name.chars().any(bad_char) { return Err(Error::Name); }
        let l = self.load(disk, dir)?;
        if self.entries(&l).iter().any(|e| same_name(&e.name, name)) { return Err(Error::Exists); }
        // The new directory's cluster, with `.` and `..`, before any entry points at it.
        let c = self.allocate(disk, None, true)?;
        let parent = match dir { Dir::Chain(p) if p != self.root_cluster || self.format != Format::Fat32 => p, _ => 0 };
        let mut both = [0u8; 2 * SLOT];
        for (k, (n, target)) in [(&b".          ", c), (&b"..         ", parent)].iter().enumerate() {
            let s = &mut both[k * SLOT..(k + 1) * SLOT];
            s[..11].copy_from_slice(&n[..]);
            s[11] = ATTR_DIR;
            s[12..32].copy_from_slice(&self.new_tail(*target, 0));
        }
        write_bytes(disk, self.cluster_at(c), &both)?;
        disk.flush()?;
        let tail = self.new_tail(c, 0);
        self.add(disk, dir, name, ATTR_DIR, tail).map_err(|e| { let _ = self.free_chain(disk, c); e })
    }
    fn remove(&mut self, disk: &mut dyn Disk, path: &[u8]) -> Result<()> {
        let (l, f) = self.find(disk, path)?;
        if f.is_dir() {
            if f.cluster == 0 { return Err(Error::Corrupt); }
            let inner = self.load(disk, Dir::Chain(f.cluster))?;
            if !self.entries(&inner).is_empty() { return Err(Error::NotEmpty); }
        }
        self.delete_slots(disk, &l, &f)?;
        // The entry goes before its clusters are freed: a cut in between leaves lost
        // clusters, never an entry sharing clusters with the next file written.
        if f.cluster != 0 { disk.flush()?; self.free_chain(disk, f.cluster)?; }
        Ok(())
    }
    fn rename(&mut self, disk: &mut dyn Disk, from: &[u8], to: &[u8]) -> Result<()> {
        let (l, f) = self.find(disk, from)?;
        let (dirs, last) = split_last(to)?;
        // A directory can't move into itself.
        let fp = parts(from)?;
        let tp = parts(to)?;
        if f.is_dir() && tp.len() > fp.len() && fp.iter().zip(&tp).all(|(a, b)| same_name(core::str::from_utf8(a).unwrap_or(""), core::str::from_utf8(b).unwrap_or(""))) { return Err(Error::Name); }
        let dir = self.dir_of(disk, &dirs)?;
        let name = core::str::from_utf8(last).map_err(|_| Error::Name)?;
        let same_dir = dir == l.dir;
        // Only the case changes: the old entry goes first, or the new one would clash.
        let mut tail = f.tail;
        tail[0] = 0;
        if same_dir && same_name(&f.name, name) {
            if f.name == name { return Ok(()); }
            self.delete_slots(disk, &l, &f)?;
            // Should the new name not fit, the old one goes back.
            return self.add(disk, dir, name, f.attr, tail).map_err(|e| { let _ = self.add(disk, dir, &f.name, f.attr, tail); e });
        }
        self.add(disk, dir, name, f.attr, tail)?;
        // Reload: adding may have reused or grown slots of the same directory.
        let (l2, f2) = if same_dir { self.find(disk, from)? } else { (l, f.clone()) };
        self.delete_slots(disk, &l2, &f2)?;
        if f.is_dir() && !same_dir && f.cluster != 0 {
            let parent = match dir { Dir::Chain(p) if p != self.root_cluster || self.format != Format::Fat32 => p, _ => 0 };
            let at = self.cluster_at(f.cluster) + SLOT as u64;
            let mut s = [0u8; SLOT];
            read_bytes(disk, at, &mut s)?;
            if &s[..11] == b"..         " {
                s[20..22].copy_from_slice(&((parent >> 16) as u16).to_le_bytes());
                s[26..28].copy_from_slice(&(parent as u16).to_le_bytes());
                write_bytes(disk, at, &s)?;
            }
        }
        Ok(())
    }
    fn truncate(&mut self, disk: &mut dyn Disk, path: &[u8], len: u64) -> Result<()> {
        let (l, f) = self.file(disk, path)?;
        if f.attr & ATTR_RO != 0 { return Err(Error::ReadOnly); }
        let old = f.size as u64;
        if len >= old {
            let first = self.resize(disk, f.cluster, old, len, len)?;
            if len > old { disk.flush()?; }
            return self.update(disk, &l, &f, first, len as u32);
        }
        // Shorter: the entry first, then the clusters past the end are freed.
        let have = if f.cluster == 0 { Vec::new() } else { self.chain(disk, f.cluster)? };
        let need = len.div_ceil(self.cluster_bytes) as usize;
        let first = if need == 0 { 0 } else { f.cluster };
        self.update(disk, &l, &f, first, len as u32)?;
        if need < have.len() {
            disk.flush()?;
            if need == 0 { self.free_chain(disk, f.cluster)?; } else {
                let eoc = self.eoc();
                self.set(disk, have[need - 1], eoc)?;
                self.free_chain(disk, have[need])?;
            }
        }
        Ok(())
    }
    fn sync(&mut self, disk: &mut dyn Disk) -> Result<()> {
        if let (Some(at), true) = (self.fsinfo, self.fsinfo_dirty) {
            let mut s = [0u8; SECTOR];
            read_bytes(disk, at, &mut s)?;
            if le32(&s, 0) == 0x4161_5252 && le32(&s, 484) == 0x6141_7272 {
                let free = self.free.unwrap_or(0xFFFF_FFFF);
                s[488..492].copy_from_slice(&free.to_le_bytes());
                s[492..496].copy_from_slice(&self.next_free.to_le_bytes());
                write_bytes(disk, at, &s)?;
            }
            self.fsinfo_dirty = false;
        }
        disk.flush()
    }
}
