// SPDX-License-Identifier: GPL-3.0-only
//! GUID Partition Tables: read (primary, else backup), find free space, add partitions and
//! write both copies back. Entries this code did not create are carried byte for byte.
use crate::crc32::{crc32, Crc32};
use crate::{BlockDev, Error, SECTOR};
use alloc::vec;
use alloc::vec::Vec;

/// A GUID as GPT stores it (first three fields little-endian).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Guid(pub [u8; 16]);
impl Guid {
    pub const ZERO: Guid = Guid([0; 16]);
    /// From the usual text order `a-b-c-d-e`.
    pub const fn new(a: u32, b: u16, c: u16, d: [u8; 8]) -> Self {
        let a = a.to_le_bytes(); let b = b.to_le_bytes(); let c = c.to_le_bytes();
        Guid([a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]])
    }
    /// A random (version 4) GUID from 16 bytes of entropy.
    pub fn random(mut bytes: [u8; 16]) -> Self {
        bytes[7] = bytes[7] & 0x0F | 0x40; // version, in the little-endian third field
        bytes[8] = bytes[8] & 0x3F | 0x80;
        Guid(bytes)
    }
    pub fn is_zero(&self) -> bool { self.0 == [0; 16] }
    /// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, as Limine's `guid://` and every tool print it.
    pub fn text(&self) -> [u8; 36] {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let b = &self.0;
        let order = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
        let mut out = [b'-'; 36];
        let mut at = 0;
        for (i, &idx) in order.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) { at += 1; }
            out[at] = HEX[(b[idx] >> 4) as usize]; out[at + 1] = HEX[(b[idx] & 15) as usize];
            at += 2;
        }
        out
    }
}

/// The EFI System partition every UEFI system boots from.
pub const TYPE_ESP: Guid = Guid::new(0xC12A7328, 0xF81F, 0x11D2, [0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9, 0x3B]);
/// A SexOS system slot: a FAT image with the kernel and its services (slot A or B).
pub const TYPE_SEXOS_BOOT: Guid = Guid::new(0x9b91c936, 0xd5fc, 0x4508, [0xa5, 0x1f, 0x2a, 0x5f, 0x03, 0xd9, 0xdb, 0x88]);
/// The SexOS store: DiskFS, the program store and the filesystem. The disk driver lets
/// SexOS write only inside this partition.
pub const TYPE_SEXOS_DATA: Guid = Guid::new(0xc4de67a4, 0xed02, 0x466c, [0x9a, 0xd5, 0x3f, 0x38, 0xc3, 0xc4, 0x76, 0xd6]);

pub const SIGNATURE: &[u8; 8] = b"EFI PART";
pub const HEADER_BYTES: usize = 92;
/// Entry arrays this code accepts: the usual 128 x 128 bytes, up to 1024 entries.
pub const MAX_ENTRIES: u32 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub my_lba: u64,
    pub alternate_lba: u64,
    pub first_usable: u64,
    pub last_usable: u64,
    pub disk_guid: Guid,
    pub entries_lba: u64,
    pub entry_count: u32,
    pub entry_size: u32,
    pub entries_crc: u32,
}
impl Header {
    /// Checks signature, size and header CRC; the entry array is checked separately.
    pub fn parse(sector: &[u8]) -> Result<Self, Error> {
        if sector.len() < SECTOR || &sector[..8] != SIGNATURE { return Err(Error::Corrupt); }
        let u32_at = |o: usize| u32::from_le_bytes(sector[o..o + 4].try_into().unwrap());
        let u64_at = |o: usize| u64::from_le_bytes(sector[o..o + 8].try_into().unwrap());
        let size = u32_at(12) as usize;
        if !(HEADER_BYTES..=SECTOR).contains(&size) { return Err(Error::Corrupt); }
        let mut copy = [0u8; SECTOR];
        copy[..size].copy_from_slice(&sector[..size]);
        copy[16..20].fill(0);
        if crc32(&copy[..size]) != u32_at(16) { return Err(Error::Corrupt); }
        let mut guid = [0u8; 16]; guid.copy_from_slice(&sector[56..72]);
        let h = Header { my_lba: u64_at(24), alternate_lba: u64_at(32), first_usable: u64_at(40), last_usable: u64_at(48),
            disk_guid: Guid(guid), entries_lba: u64_at(72), entry_count: u32_at(80), entry_size: u32_at(84), entries_crc: u32_at(88) };
        if h.entry_size < 128 || h.entry_size % 8 != 0 || h.entry_size > 1024 || h.entry_count == 0 || h.entry_count > MAX_ENTRIES
            || h.first_usable > h.last_usable { return Err(Error::Corrupt); }
        Ok(h)
    }
    pub fn entry_sectors(&self) -> u64 { (self.entry_count as u64 * self.entry_size as u64).div_ceil(SECTOR as u64) }
    pub fn encode(&self) -> [u8; SECTOR] {
        let mut s = [0u8; SECTOR];
        s[..8].copy_from_slice(SIGNATURE);
        s[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        s[12..16].copy_from_slice(&(HEADER_BYTES as u32).to_le_bytes());
        s[24..32].copy_from_slice(&self.my_lba.to_le_bytes());
        s[32..40].copy_from_slice(&self.alternate_lba.to_le_bytes());
        s[40..48].copy_from_slice(&self.first_usable.to_le_bytes());
        s[48..56].copy_from_slice(&self.last_usable.to_le_bytes());
        s[56..72].copy_from_slice(&self.disk_guid.0);
        s[72..80].copy_from_slice(&self.entries_lba.to_le_bytes());
        s[80..84].copy_from_slice(&self.entry_count.to_le_bytes());
        s[84..88].copy_from_slice(&self.entry_size.to_le_bytes());
        s[88..92].copy_from_slice(&self.entries_crc.to_le_bytes());
        let c = crc32(&s[..HEADER_BYTES]);
        s[16..20].copy_from_slice(&c.to_le_bytes());
        s
    }
}

/// One partition entry. `raw` keeps every byte (attributes, name, vendor fields).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry { pub raw: Vec<u8> }
impl Entry {
    pub fn type_guid(&self) -> Guid { Guid(self.raw[0..16].try_into().unwrap()) }
    pub fn part_guid(&self) -> Guid { Guid(self.raw[16..32].try_into().unwrap()) }
    pub fn first(&self) -> u64 { u64::from_le_bytes(self.raw[32..40].try_into().unwrap()) }
    pub fn last(&self) -> u64 { u64::from_le_bytes(self.raw[40..48].try_into().unwrap()) }
    pub fn sectors(&self) -> u64 { self.last() - self.first() + 1 }
    pub fn is_used(&self) -> bool { !self.type_guid().is_zero() }
    pub fn new(size: usize, type_guid: Guid, part_guid: Guid, first: u64, last: u64, name: &str) -> Self {
        let mut raw = vec![0u8; size];
        raw[0..16].copy_from_slice(&type_guid.0);
        raw[16..32].copy_from_slice(&part_guid.0);
        raw[32..40].copy_from_slice(&first.to_le_bytes());
        raw[40..48].copy_from_slice(&last.to_le_bytes());
        for (i, u) in name.encode_utf16().take(36).enumerate() { raw[56 + 2 * i..58 + 2 * i].copy_from_slice(&u.to_le_bytes()); }
        Entry { raw }
    }
    /// The UTF-16 name, as ASCII with `?` for anything else.
    pub fn name_ascii(&self, out: &mut [u8; 36]) -> usize {
        let mut n = 0;
        for i in 0..36 {
            let u = u16::from_le_bytes([self.raw[56 + 2 * i], self.raw[57 + 2 * i]]);
            if u == 0 { break; }
            out[n] = if u < 0x80 { u as u8 } else { b'?' }; n += 1;
        }
        n
    }
}

#[derive(Clone, Debug)]
pub struct Gpt {
    pub header: Header,
    pub entries: Vec<Entry>,
    /// The primary copy was damaged and this came from the backup.
    pub from_backup: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent { pub first: u64, pub sectors: u64 }

fn read_entries<D: BlockDev + ?Sized>(dev: &mut D, h: &Header) -> Result<Vec<Entry>, Error> {
    let n = h.entry_sectors();
    if h.entries_lba.checked_add(n).is_none_or(|end| end > dev.sectors()) { return Err(Error::Corrupt); }
    let mut bytes = vec![0u8; n as usize * SECTOR];
    dev.read(h.entries_lba, &mut bytes)?;
    let used = h.entry_count as usize * h.entry_size as usize;
    if crc32(&bytes[..used]) != h.entries_crc { return Err(Error::Corrupt); }
    Ok(bytes[..used].chunks(h.entry_size as usize).map(|c| Entry { raw: c.to_vec() }).collect())
}

fn read_at<D: BlockDev + ?Sized>(dev: &mut D, lba: u64) -> Result<Gpt, Error> {
    let mut s = [0u8; SECTOR];
    dev.read(lba, &mut s)?;
    let header = Header::parse(&s)?;
    if header.my_lba != lba || header.last_usable >= dev.sectors() { return Err(Error::Corrupt); }
    let entries = read_entries(dev, &header)?;
    Ok(Gpt { header, entries, from_backup: false })
}

impl Gpt {
    /// The primary table, or the backup at the last sector when the primary is damaged.
    pub fn read<D: BlockDev + ?Sized>(dev: &mut D) -> Result<Self, Error> {
        match read_at(dev, 1) {
            Ok(g) => Ok(g),
            Err(Error::Io) => Err(Error::Io),
            Err(_) => {
                let mut g = read_at(dev, dev.sectors() - 1)?;
                g.from_backup = true;
                Ok(g)
            }
        }
    }

    /// A new empty table covering the whole device (128 entries of 128 bytes).
    pub fn new_empty(sectors: u64, disk_guid: Guid) -> Result<Self, Error> {
        if sectors < 34 * 2 + 2048 { return Err(Error::NoSpace); }
        let entry_count = 128u32; let entry_size = 128u32;
        let header = Header { my_lba: 1, alternate_lba: sectors - 1, first_usable: 34, last_usable: sectors - 34,
            disk_guid, entries_lba: 2, entry_count, entry_size, entries_crc: 0 };
        Ok(Gpt { header, entries: (0..entry_count).map(|_| Entry { raw: vec![0; entry_size as usize] }).collect(), from_backup: false })
    }

    pub fn used(&self) -> impl Iterator<Item = (usize, &Entry)> { self.entries.iter().enumerate().filter(|(_, e)| e.is_used()) }

    /// Every partition's range is inside the usable area and no two overlap.
    pub fn check(&self) -> Result<(), Error> {
        let mut spans: Vec<(u64, u64)> = Vec::new();
        for (_, e) in self.used() {
            if e.first() > e.last() || e.first() < self.header.first_usable || e.last() > self.header.last_usable { return Err(Error::Corrupt); }
            spans.push((e.first(), e.last()));
        }
        spans.sort_unstable();
        if spans.windows(2).any(|w| w[1].0 <= w[0].1) { return Err(Error::Corrupt); }
        Ok(())
    }

    /// Unpartitioned space inside the usable area, each run starting on `align` sectors and
    /// cut to whole multiples of it, largest first.
    pub fn free(&self, align: u64) -> Vec<Extent> {
        let mut spans: Vec<(u64, u64)> = self.used().map(|(_, e)| (e.first(), e.last())).collect();
        spans.sort_unstable();
        let mut out = Vec::new();
        let mut cursor = self.header.first_usable;
        let end = self.header.last_usable + 1;
        let mut push = |from: u64, to: u64| {
            let first = from.next_multiple_of(align);
            if to > first { let sectors = (to - first) / align * align; if sectors > 0 { out.push(Extent { first, sectors }); } }
        };
        for (a, b) in spans {
            if a > cursor { push(cursor, a); }
            cursor = cursor.max(b + 1);
        }
        if end > cursor { push(cursor, end); }
        out.sort_by(|x, y| y.sectors.cmp(&x.sectors).then(x.first.cmp(&y.first)));
        out
    }

    /// Puts a partition in the first empty entry. The range must be free.
    pub fn add(&mut self, type_guid: Guid, part_guid: Guid, first: u64, sectors: u64, name: &str) -> Result<usize, Error> {
        if sectors == 0 { return Err(Error::Invalid); }
        let last = first.checked_add(sectors - 1).ok_or(Error::Invalid)?;
        if first < self.header.first_usable || last > self.header.last_usable { return Err(Error::NoSpace); }
        if self.used().any(|(_, e)| first <= e.last() && e.first() <= last) { return Err(Error::Exists); }
        let slot = self.entries.iter().position(|e| !e.is_used()).ok_or(Error::NoSpace)?;
        self.entries[slot] = Entry::new(self.header.entry_size as usize, type_guid, part_guid, first, last, name);
        Ok(slot)
    }

    pub fn find_type(&self, t: Guid) -> impl Iterator<Item = &Entry> { self.entries.iter().filter(move |e| e.type_guid() == t) }

    fn entry_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.header.entry_sectors() as usize * SECTOR);
        for e in &self.entries { out.extend_from_slice(&e.raw); }
        out.resize(self.header.entry_sectors() as usize * SECTOR, 0);
        out
    }

    /// Writes the backup copy at the end of the device, then the primary copy and a
    /// protective MBR that keeps the first sector's boot code and disk signature. The
    /// backup goes first so that a failure in between leaves one valid table.
    pub fn write<D: BlockDev + ?Sized>(&mut self, dev: &mut D) -> Result<(), Error> {
        self.check()?;
        let total = dev.sectors();
        let n = self.header.entry_sectors();
        let last_lba = total - 1;
        let backup_entries = last_lba - n;
        if backup_entries <= self.header.last_usable || self.header.entries_lba + n > self.header.first_usable { return Err(Error::Corrupt); }
        let bytes = self.entry_bytes();
        let used = self.header.entry_count as usize * self.header.entry_size as usize;
        let mut c = Crc32::new(); c.update(&bytes[..used]);
        self.header.entries_crc = c.finish();
        let primary_entries = self.header.entries_lba;
        let backup = Header { my_lba: last_lba, alternate_lba: 1, entries_lba: backup_entries, ..self.header };
        dev.write(backup_entries, &bytes)?;
        dev.write(last_lba, &backup.encode())?;
        let primary = Header { my_lba: 1, alternate_lba: last_lba, entries_lba: primary_entries, ..self.header };
        dev.write(primary_entries, &bytes)?;
        dev.write(1, &primary.encode())?;
        let mut mbr = [0u8; SECTOR];
        dev.read(0, &mut mbr)?;
        protective_mbr(&mut mbr, total);
        dev.write(0, &mbr)?;
        self.header = primary;
        self.from_backup = false;
        Ok(())
    }
}

/// Makes `mbr` a protective MBR for a disk of `total` sectors: one 0xEE entry over the
/// whole disk, boot code (bytes 0..440) and the disk signature left as they were.
pub fn protective_mbr(mbr: &mut [u8; SECTOR], total: u64) {
    mbr[446..510].fill(0);
    let e = &mut mbr[446..462];
    e[1..4].copy_from_slice(&[0x00, 0x02, 0x00]);
    e[4] = 0xEE;
    e[5..8].copy_from_slice(&[0xFF, 0xFF, 0xFF]);
    e[8..12].copy_from_slice(&1u32.to_le_bytes());
    e[12..16].copy_from_slice(&((total - 1).min(0xFFFF_FFFF) as u32).to_le_bytes());
    mbr[510] = 0x55; mbr[511] = 0xAA;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemDev;
    fn disk(mib: usize) -> MemDev<Vec<u8>> { MemDev(vec![0u8; mib << 20]) }

    #[test]
    fn guid_text_matches_the_usual_form() {
        assert_eq!(&TYPE_ESP.text(), b"c12a7328-f81f-11d2-ba4b-00a0c93ec93b");
        assert_eq!(&TYPE_SEXOS_DATA.text(), b"c4de67a4-ed02-466c-9ad5-3f38c3c476d6");
    }

    #[test]
    fn writes_reads_and_recovers_from_backup() {
        let mut d = disk(64);
        let total = d.sectors();
        let mut g = Gpt::new_empty(total, Guid::random([7; 16])).unwrap();
        g.add(TYPE_ESP, Guid::random([1; 16]), 2048, 32768, "EFI system").unwrap();
        g.write(&mut d).unwrap();
        let back = Gpt::read(&mut d).unwrap();
        assert!(!back.from_backup);
        assert_eq!(back.used().count(), 1);
        let free = back.free(2048);
        assert_eq!(free[0], Extent { first: 2048 + 32768, sectors: (total - 34 + 1 - 34816) / 2048 * 2048 });
        // Damage the primary: the backup still reads.
        d.0[512 + 20] ^= 0xFF;
        let rec = Gpt::read(&mut d).unwrap();
        assert!(rec.from_backup);
        assert_eq!(rec.used().count(), 1);
        assert!(g.add(TYPE_SEXOS_DATA, Guid::random([2; 16]), 4096, 10, "x").is_err(), "overlap refused");
    }

    #[test]
    fn sgdisk_agrees() {
        let Ok(out) = std::process::Command::new("sgdisk").arg("--version").output() else { return };
        if !out.status.success() { return; }
        let dir = std::env::temp_dir().join(format!("sexdisk-gpt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("d.img");
        std::fs::write(&img, vec![0u8; 128 << 20]).unwrap();
        // A table made by sgdisk, as another OS would leave it.
        assert!(std::process::Command::new("sgdisk").args(["-n1:2048:+40M", "-t1:ef00", "-n2:0:+30M", "-t2:8300"]).arg(&img).output().unwrap().status.success());
        let mut d = MemDev(std::fs::read(&img).unwrap());
        let mut g = Gpt::read(&mut d).unwrap();
        assert_eq!(g.used().count(), 2);
        let free = g.free(2048)[0];
        g.add(TYPE_SEXOS_DATA, Guid::random([9; 16]), free.first, free.sectors, "SexOS").unwrap();
        g.write(&mut d).unwrap();
        std::fs::write(&img, &d.0).unwrap();
        let v = std::process::Command::new("sgdisk").arg("-v").arg(&img).output().unwrap();
        let text = String::from_utf8_lossy(&v.stdout);
        assert!(text.contains("No problems found"), "{text}");
        let p = std::process::Command::new("sgdisk").arg("-p").arg(&img).output().unwrap();
        let text = String::from_utf8_lossy(&p.stdout);
        assert!(text.contains("SexOS"), "{text}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
