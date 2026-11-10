//! Intel Matrix Storage (IMSM) metadata: the RAID layout Intel RST firmware
//! writes at the end of every member disk ("RAID On" in a PC's BIOS).
//!
//! The metadata is a Matrix Partition Block (MPB). Its first sector (the
//! anchor) is the second-to-last sector of the disk; when it is longer than one
//! sector the rest sits just before the anchor, in order. It lists the member
//! disks by serial number and up to a few volumes (RAID 0, 1, 5, 10), each with
//! where it starts on every member, the strip size and the member order.
//!
//! This crate only reads. It assembles the MPB from the disk's tail sectors,
//! checks signature, size and checksum, and maps a volume sector to the member
//! and member sector that hold it, for RAID 0 and RAID 1 volumes that are not
//! in the middle of a migration. Everything else is reported, never guessed.
//!
//! Layout facts (sizes, offsets, checksum rule, serial normalisation) match
//! what Linux mdadm reads; the fixtures in `tests/fixtures` were checked with
//! `mdadm --examine`. No mdadm code is used.
#![no_std]

pub const SECTOR: usize = 512;
/// The MPB may be longer than this, but RST volumes on PCs never are; larger
/// metadata is refused, not truncated.
pub const MAX_MPB_SECTORS: usize = 16;
/// How many sectors at the end of a disk to read: the MPB plus the last sector.
pub const TAIL_SECTORS: usize = MAX_MPB_SECTORS + 1;
pub const MAX_DISKS: usize = 8;
pub const MAX_VOLUMES: usize = 4;
pub const MAX_MEMBERS: usize = 8;

const SIGNATURE: &[u8; 24] = b"Intel Raid ISM Cfg Sig. ";
const DISK_TABLE: usize = 0xD8;
const DISK_BYTES: usize = 48;
const DEV_HEAD: usize = 80;
const VOL_HEAD: usize = 32;
const MAP_HEAD: usize = 48;
/// The disk is being rebuilt from its partner; it holds stale data.
const ORD_REBUILD: u32 = 1 << 24;

pub const DISK_SPARE: u32 = 0x01;
pub const DISK_CONFIGURED: u32 = 0x02;
pub const DISK_FAILED: u32 = 0x04;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No IMSM signature in the anchor: not a RAID member.
    NotMember,
    /// The MPB claims a size it cannot have, or larger than we read.
    BadSize,
    BadChecksum,
    /// A table runs past the end of the MPB.
    Truncated,
    /// More disks, volumes or members than this reader keeps.
    TooMany,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unreadable {
    /// RAID 5 or RAID 10 (or an unknown level): not assembled yet.
    Level(u8),
    /// The volume is being initialised, rebuilt, migrated or resized.
    Migrating,
    /// Map state says failed, or a RAID 0 member is missing.
    Failed,
    /// No member is in a state to read from.
    NoMember,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Disk {
    /// As stored: at most 16 bytes, NUL padded.
    pub serial: [u8; 16],
    pub sectors: u64,
    pub status: u32,
}

impl Disk {
    pub fn serial(&self) -> &[u8] {
        let end = self.serial.iter().position(|&b| b == 0).unwrap_or(16);
        &self.serial[..end]
    }
    pub fn failed(&self) -> bool { self.status & DISK_FAILED != 0 }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Volume {
    pub name: [u8; 16],
    /// Volume size in sectors.
    pub sectors: u64,
    pub level: u8,
    pub map_state: u8,
    pub migrating: bool,
    /// Where the volume starts on every member.
    pub start: u64,
    pub per_member: u64,
    /// Strip (chunk) size in sectors.
    pub strip: u32,
    pub members: u8,
    /// Disk-table index of each member, in member order.
    pub order: [u8; MAX_MEMBERS],
    /// Members marked as being rebuilt.
    pub rebuilding: u8,
    pub status: u32,
}

/// One piece of a volume read: `sectors` sectors from member slot `member`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    pub member: u8,
    pub lba: u64,
    pub sectors: u64,
}

impl Volume {
    pub fn name(&self) -> &[u8] {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(16);
        &self.name[..end]
    }
    pub fn uses_disk(&self, index: usize) -> bool {
        self.order[..self.members as usize].iter().any(|&d| d as usize == index)
    }
    /// The member slot that holds disk `index`, if any.
    pub fn slot_of(&self, index: usize) -> Option<u8> {
        self.order[..self.members as usize].iter().position(|&d| d as usize == index).map(|s| s as u8)
    }
    fn usable(&self, slot: usize) -> bool { self.rebuilding & (1 << slot) == 0 }

    /// Whether this reader can read the volume, given which member slots are
    /// present (bit i = member slot i was found and is not failed).
    pub fn readable(&self, present: u8) -> Result<(), Unreadable> {
        if self.migrating || self.map_state == 1 { return Err(Unreadable::Migrating); }
        if self.map_state == 3 { return Err(Unreadable::Failed); }
        let all = if self.members as usize >= 8 { 0xFF } else { (1u8 << self.members) - 1 };
        match (self.level, self.members) {
            (0, 1..) => if present & all == all && self.rebuilding == 0 { Ok(()) } else { Err(Unreadable::Failed) },
            (1, 2) => if (0..2).any(|s| present & (1 << s) != 0 && self.usable(s)) { Ok(()) } else { Err(Unreadable::NoMember) },
            (level, _) => Err(Unreadable::Level(level)),
        }
    }

    /// The first piece of reading `sectors` sectors at volume sector `lba`;
    /// call again with the rest. RAID 1 reads from the first present member.
    pub fn map(&self, lba: u64, sectors: u64, present: u8) -> Option<Extent> {
        if sectors == 0 || lba.checked_add(sectors)? > self.sectors || self.readable(present).is_err() { return None; }
        match self.level {
            0 => {
                let strip = self.strip as u64;
                let n = self.members as u64;
                if strip == 0 { return None; }
                let index = lba / strip;
                let within = lba % strip;
                let row = index / n;
                let member_lba = row.checked_mul(strip)?.checked_add(within)?;
                if member_lba >= self.per_member { return None; }
                Some(Extent { member: (index % n) as u8, lba: self.start.checked_add(member_lba)?,
                              sectors: sectors.min(strip - within) })
            }
            1 => {
                if lba + sectors > self.per_member { return None; }
                let slot = (0..2u8).find(|&s| present & (1 << s) != 0 && self.usable(s as usize))?;
                Some(Extent { member: slot, lba: self.start.checked_add(lba)?, sectors })
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Metadata {
    pub version: [u8; 6],
    pub family: u32,
    pub generation: u32,
    pub mpb_bytes: u32,
    pub disks: [Disk; MAX_DISKS],
    pub disk_count: u8,
    pub volumes: [Volume; MAX_VOLUMES],
    pub volume_count: u8,
}

fn u16_at(b: &[u8], at: usize) -> Result<u16, Error> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or(Error::Truncated)
}
fn u32_at(b: &[u8], at: usize) -> Result<u32, Error> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or(Error::Truncated)
}
fn u8_at(b: &[u8], at: usize) -> Result<u8, Error> { b.get(at).copied().ok_or(Error::Truncated) }
fn wide(b: &[u8], lo: usize, hi: usize) -> Result<u64, Error> {
    Ok(u32_at(b, lo)? as u64 | (u32_at(b, hi)? as u64) << 32)
}

/// How many sectors the MPB takes, from its anchor sector (the second-to-last
/// sector of the disk). `NotMember` when the anchor has no IMSM signature.
pub fn mpb_sectors(anchor: &[u8]) -> Result<usize, Error> {
    if anchor.len() < SECTOR || &anchor[..24] != SIGNATURE { return Err(Error::NotMember); }
    let size = u32_at(anchor, 0x24)? as usize;
    if size < DISK_TABLE + DISK_BYTES || size % 4 != 0 { return Err(Error::BadSize); }
    let sectors = size.div_ceil(SECTOR);
    if sectors > MAX_MPB_SECTORS { return Err(Error::BadSize); }
    Ok(sectors)
}

/// Put the MPB together from the last `TAIL_SECTORS` sectors of a disk (or
/// fewer, if the disk is that small) into `out`, and return its length.
pub fn assemble(tail: &[u8], out: &mut [u8; MAX_MPB_SECTORS * SECTOR]) -> Result<usize, Error> {
    if tail.len() < 2 * SECTOR || tail.len() % SECTOR != 0 { return Err(Error::Truncated); }
    let anchor_at = tail.len() - 2 * SECTOR;
    let anchor = &tail[anchor_at..anchor_at + SECTOR];
    let sectors = mpb_sectors(anchor)?;
    let extra = (sectors - 1) * SECTOR;
    if extra > anchor_at { return Err(Error::Truncated); }
    out[..SECTOR].copy_from_slice(anchor);
    out[SECTOR..SECTOR + extra].copy_from_slice(&tail[anchor_at - extra..anchor_at]);
    Ok(u32_at(anchor, 0x24)? as usize)
}

/// Read the MPB straight from the tail sectors of a disk.
pub fn from_tail(tail: &[u8]) -> Result<Metadata, Error> {
    let mut buf = [0u8; MAX_MPB_SECTORS * SECTOR];
    let len = assemble(tail, &mut buf)?;
    parse(&buf[..len])
}

/// Parse an assembled MPB.
pub fn parse(mpb: &[u8]) -> Result<Metadata, Error> {
    if mpb.len() < SECTOR.min(DISK_TABLE) || &mpb[..24] != SIGNATURE { return Err(Error::NotMember); }
    let size = u32_at(mpb, 0x24)? as usize;
    if size > mpb.len() || size < DISK_TABLE || size % 4 != 0 { return Err(Error::BadSize); }
    let mpb = &mpb[..size];
    let stored = u32_at(mpb, 0x20)?;
    let sum = mpb.chunks_exact(4).fold(0u32, |s, w| s.wrapping_add(u32::from_le_bytes([w[0], w[1], w[2], w[3]])));
    if sum.wrapping_sub(stored) != stored { return Err(Error::BadChecksum); }

    let mut m = Metadata { family: u32_at(mpb, 0x28)?, generation: u32_at(mpb, 0x2C)?, mpb_bytes: size as u32, ..Default::default() };
    m.version.copy_from_slice(&mpb[24..30]);
    let disks = u8_at(mpb, 0x38)? as usize;
    let volumes = u8_at(mpb, 0x39)? as usize;
    if disks > MAX_DISKS || volumes > MAX_VOLUMES { return Err(Error::TooMany); }
    for i in 0..disks {
        let at = DISK_TABLE + i * DISK_BYTES;
        let d = mpb.get(at..at + DISK_BYTES).ok_or(Error::Truncated)?;
        let mut serial = [0u8; 16];
        serial.copy_from_slice(&d[..16]);
        m.disks[i] = Disk { serial, sectors: wide(d, 16, 32)?, status: u32_at(d, 24)? };
    }
    m.disk_count = disks as u8;

    let mut at = DISK_TABLE + disks * DISK_BYTES;
    for v in 0..volumes {
        let dev = mpb.get(at..).ok_or(Error::Truncated)?;
        let map = DEV_HEAD + VOL_HEAD;
        let members = u8_at(dev, map + 16)? as usize;
        if members == 0 || members > MAX_MEMBERS { return Err(Error::TooMany); }
        let map_bytes = MAP_HEAD + 4 * members;
        let migrating = u8_at(dev, DEV_HEAD + 8)? != 0;
        let total = map + map_bytes * if migrating { 2 } else { 1 };
        if dev.len() < total { return Err(Error::Truncated); }
        let mut vol = Volume {
            sectors: wide(dev, 16, 20)?,
            status: u32_at(dev, 24)?,
            migrating,
            start: wide(dev, map, map + 20)?,
            per_member: wide(dev, map + 4, map + 24)?,
            strip: u16_at(dev, map + 12)? as u32,
            map_state: u8_at(dev, map + 14)?,
            level: u8_at(dev, map + 15)?,
            members: members as u8,
            ..Default::default()
        };
        vol.name.copy_from_slice(&dev[..16]);
        for s in 0..members {
            let ord = u32_at(dev, map + MAP_HEAD + 4 * s)?;
            let index = (ord & 0x00FF_FFFF) as usize;
            if index >= disks { return Err(Error::Truncated); }
            vol.order[s] = index as u8;
            if ord & ORD_REBUILD != 0 { vol.rebuilding |= 1 << s; }
        }
        // RAID 1 with four or more members is how IMSM spells RAID 10.
        if vol.level == 1 && members >= 4 { vol.level = 10; }
        m.volumes[v] = vol;
        at += total;
    }
    m.volume_count = volumes as u8;
    Ok(m)
}

impl Metadata {
    pub fn disks(&self) -> &[Disk] { &self.disks[..self.disk_count as usize] }
    pub fn volumes(&self) -> &[Volume] { &self.volumes[..self.volume_count as usize] }
    /// Which disk-table entry a drive with this serial (as the drive reports
    /// it, e.g. NVMe Identify bytes 4..24) is.
    pub fn disk_for_serial(&self, reported: &[u8]) -> Option<usize> {
        let mut norm = [0u8; 16];
        let n = normalise_serial(reported, &mut norm);
        if n == 0 { return None; }
        self.disks().iter().position(|d| d.serial() == &norm[..n])
    }
    /// The same RAID set: same family and the copies agree on the layout.
    pub fn same_set(&self, other: &Metadata) -> bool {
        self.family == other.family && self.disks() == other.disks() && self.volumes() == other.volumes()
    }
}

/// The serial as IMSM stores it: drop spaces and control bytes, ':' becomes
/// ';', and keep only the last 16 bytes.
pub fn normalise_serial(reported: &[u8], out: &mut [u8; 16]) -> usize {
    let mut tmp = [0u8; 64];
    let mut n = 0;
    for &b in reported.iter() {
        if b > 0x20 && n < tmp.len() { tmp[n] = if b == b':' { b';' } else { b }; n += 1; }
    }
    let start = n.saturating_sub(16);
    let len = n - start;
    *out = [0; 16];
    out[..len].copy_from_slice(&tmp[start..n]);
    len
}
