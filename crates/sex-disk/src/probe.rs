// SPDX-License-Identifier: GPL-3.0-only
//! What a disk holds, decided before SexOS writes a single sector to it. The disk driver
//! runs this at start without a heap: a live system booted from a USB stick must never
//! write into a disk that belongs to another operating system.
use crate::crc32::Crc32;
use crate::gpt::{Header, TYPE_SEXOS_DATA};
use crate::{BlockDev, Error, SECTOR};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// No partition table and no filesystem signature: SexOS may use the whole disk.
    Blank,
    /// SexOS's own install from before GPT: one MBR entry of type 0xEF at the boot slot.
    LegacySexos,
    /// A GPT with one SexOS store partition: SexOS works only inside it.
    Sexos { first: u64, sectors: u64 },
    /// Anything else belongs to someone else. SexOS writes nothing there except through
    /// the installer, which adds partitions only in free space.
    Foreign(Foreign),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Foreign {
    /// A GPT without a SexOS store.
    Gpt,
    /// An MBR partition table (another system, or a disk SexOS does not install to).
    Mbr,
    /// A filesystem written straight onto the disk (ext, XFS, NTFS, LUKS, ...).
    Filesystem,
    /// A table that claims to be GPT but fails its checks, or more than one SexOS store.
    Damaged,
}

/// The legacy boot slot (MBR partition 1, type EF, at this LBA).
pub const LEGACY_BOOT_LBA: u32 = 65536;

pub fn classify<D: BlockDev + ?Sized>(dev: &mut D) -> Result<Layout, Error> {
    let mut s0 = [0u8; SECTOR];
    let mut s1 = [0u8; SECTOR];
    dev.read(0, &mut s0)?;
    dev.read(1, &mut s1)?;
    let mbr = s0[510] == 0x55 && s0[511] == 0xAA;
    let protective = mbr && (0..4).any(|i| s0[446 + 16 * i + 4] == 0xEE);
    if &s1[..8] == crate::gpt::SIGNATURE || protective {
        return gpt(dev, &s1);
    }
    if mbr {
        let types: [u8; 4] = core::array::from_fn(|i| s0[446 + 16 * i + 4]);
        let start = u32::from_le_bytes(s0[446 + 8..446 + 12].try_into().unwrap());
        if types == [0xEF, 0, 0, 0] && start == LEGACY_BOOT_LBA { return Ok(Layout::LegacySexos); }
        return Ok(Layout::Foreign(Foreign::Mbr));
    }
    if &s0[0..4] == b"XFSB" || &s0[0..6] == b"LUKS\xba\xbe" || &s0[3..11] == b"NTFS    " || &s0[3..11] == b"EXFAT   " {
        return Ok(Layout::Foreign(Foreign::Filesystem));
    }
    let mut s2 = [0u8; SECTOR];
    if dev.sectors() > 2 {
        dev.read(2, &mut s2)?;
        if s2[56] == 0x53 && s2[57] == 0xEF { return Ok(Layout::Foreign(Foreign::Filesystem)); }
    }
    Ok(Layout::Blank)
}

fn gpt<D: BlockDev + ?Sized>(dev: &mut D, s1: &[u8; SECTOR]) -> Result<Layout, Error> {
    let total = dev.sectors();
    if let Ok(h) = Header::parse(s1) {
        if h.my_lba == 1 && h.last_usable < total {
            if let Some(found) = scan(dev, &h)? { return Ok(found); }
        }
    }
    // The primary is damaged: the backup at the last sector.
    let mut last = [0u8; SECTOR];
    dev.read(total - 1, &mut last)?;
    if let Ok(h) = Header::parse(&last) {
        if h.my_lba == total - 1 && h.last_usable < total {
            if let Some(found) = scan(dev, &h)? { return Ok(found); }
        }
    }
    Ok(Layout::Foreign(Foreign::Damaged))
}

/// The layout one header and its entry array describe, or None when the array fails its checksum.
fn scan<D: BlockDev + ?Sized>(dev: &mut D, header: &Header) -> Result<Option<Layout>, Error> {
    let size = header.entry_size as usize;
    if SECTOR % size != 0 && size % SECTOR != 0 { return Ok(Some(Layout::Foreign(Foreign::Damaged))); }
    let mut crc = Crc32::new();
    let total = header.entry_count as usize * size;
    let mut found: Option<(u64, u64)> = None;
    let mut many = false;
    let mut sector = [0u8; SECTOR];
    let mut done = 0usize;
    let mut lba = header.entries_lba;
    while done < total {
        if lba >= dev.sectors() { return Ok(None); }
        dev.read(lba, &mut sector)?;
        let take = (total - done).min(SECTOR);
        crc.update(&sector[..take]);
        // Entries start at multiples of `size`; with size >= 512 only the sector holding
        // an entry's start carries its type and range.
        let mut off = 0;
        while off < take {
            if (done + off) % size == 0 {
                let e = &sector[off..off + 48.min(take - off)];
                if e.len() == 48 && e[..16] == TYPE_SEXOS_DATA.0 {
                    let first = u64::from_le_bytes(e[32..40].try_into().unwrap());
                    let last = u64::from_le_bytes(e[40..48].try_into().unwrap());
                    if found.is_some() { many = true; }
                    found = Some((first, last));
                }
            }
            off += size.min(SECTOR);
        }
        done += take;
        lba += 1;
    }
    if crc.finish() != header.entries_crc { return Ok(None); }
    Ok(Some(match found {
        _ if many => Layout::Foreign(Foreign::Damaged),
        Some((first, last)) if first >= header.first_usable && first <= last && last <= header.last_usable =>
            Layout::Sexos { first, sectors: last - first + 1 },
        Some(_) => Layout::Foreign(Foreign::Damaged),
        None => Layout::Foreign(Foreign::Gpt),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpt::{Gpt, Guid, TYPE_ESP};
    use crate::MemDev;
    #[test]
    fn decides_whose_disk_it_is() {
        let mut d = MemDev(vec![0u8; 64 << 20]);
        assert_eq!(classify(&mut d).unwrap(), Layout::Blank);
        let mut g = Gpt::new_empty(d.sectors(), Guid::random([3; 16])).unwrap();
        g.add(TYPE_ESP, Guid::random([4; 16]), 2048, 4096, "EFI").unwrap();
        g.write(&mut d).unwrap();
        assert_eq!(classify(&mut d).unwrap(), Layout::Foreign(Foreign::Gpt));
        g.add(TYPE_SEXOS_DATA, Guid::random([5; 16]), 8192, 65536, "SexOS").unwrap();
        g.write(&mut d).unwrap();
        assert_eq!(classify(&mut d).unwrap(), Layout::Sexos { first: 8192, sectors: 65536 });
        // A damaged primary: the backup still finds the store.
        d.0[512 + 30] ^= 1;
        assert_eq!(classify(&mut d).unwrap(), Layout::Sexos { first: 8192, sectors: 65536 });
        // A damaged entry array: refuse.
        d.0[512 + 30] ^= 1;
        d.0[1024 + 200] ^= 1;
        let n = d.0.len(); d.0[n - 512 - 32 * 512 + 200] ^= 1;
        assert_eq!(classify(&mut d).unwrap(), Layout::Foreign(Foreign::Damaged));

        let mut m = MemDev(vec![0u8; 1 << 20]);
        m.0[446 + 4] = 0xEF; m.0[446 + 8..446 + 12].copy_from_slice(&LEGACY_BOOT_LBA.to_le_bytes()); m.0[510] = 0x55; m.0[511] = 0xAA;
        assert_eq!(classify(&mut m).unwrap(), Layout::LegacySexos);
        m.0[446 + 16 + 4] = 0x07;
        assert_eq!(classify(&mut m).unwrap(), Layout::Foreign(Foreign::Mbr));
        let mut e = MemDev(vec![0u8; 1 << 20]);
        e.0[1024 + 56] = 0x53; e.0[1024 + 57] = 0xEF;
        assert_eq!(classify(&mut e).unwrap(), Layout::Foreign(Foreign::Filesystem));
    }
}
