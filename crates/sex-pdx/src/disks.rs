// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Raw removable disks, for a program the person lets reshape one (Corset): Files
//! requests on the same wire and region as [`crate::fs`].
//!
//! Kind numbers start at 30: 1..=23 are [`crate::fs`]'s, 24..=28 [`crate::home_lock`]'s
//! and 29 the shell's OFFER.
//!
//! A run lists the disks with [`DISKS`] and moves sectors with [`DISK_IO`]. The first
//! DISK_IO on a disk asks the person, the way the open/save picker does: Files answers
//! [`crate::fs::PENDING`] until they answer, in the program that shows questions
//! ([`DISK_ASKS`], [`DISK_ANSWER`]); the question names the disk's product and size every
//! time. A yes lets that one run use that one disk until it sends [`DISK_DONE`] or exits;
//! a no answers DENIED (a later DISK_IO asks again).
//!
//! Never written, whatever the person answers: the system disk and any disk that holds
//! `/home`'s encrypted image ([`FLAG_SYSTEM`]). Files refuses those itself.
//!
//! Before the first write to a disk, Files writes out and unmounts every volume on it
//! under `/media`, and every grant into those volumes ends, as at eject. DISK_DONE scans
//! the disk again and mounts what it now holds.

/// The disks: entries from DATA, the reply's value is the bytes used. Each entry: disk
/// id (u16 LE), flags (u8), size in sectors (u64 LE), a name length byte and the short
/// name (`usb0`, `system`, an image's file name), a product length byte and the product.
pub const DISKS: u8 = 30;
/// Moves sectors between the disk and the request's data area. `offset`: the first
/// sector. `length`: [`io`]. The reply's value is the sectors moved. Every range must lie
/// inside the disk and the data area (INVALID otherwise). A write of no sectors flushes:
/// what was written before is on the medium when it answers.
pub const DISK_IO: u8 = 31;
/// `length`: the disk id. Writes the disk's cache out, ends this run's grant to it and
/// mounts its volumes again.
pub const DISK_DONE: u8 = 32;
/// The questions waiting, for the program showing them (it holds the whole filesystem,
/// `@/`). Entries from DATA: row (u8), ticket (u32 LE), a program name length byte and
/// the name, a length byte and the disk as the question names it.
pub const DISK_ASKS: u8 = 33;
/// Answers one question: `offset` the row, `length` the ticket, with [`YES`] set to let
/// the run use the disk.
pub const DISK_ANSWER: u8 = 34;
pub const YES: u64 = 1 << 32;

/// Sectors are 512 bytes; one DISK_IO moves at most this many.
pub const SECTOR: usize = 512;
pub const IO_SECTORS: u64 = (crate::fs::IO_MAX / SECTOR) as u64;
pub const WRITE: u64 = 1 << 31;
/// DISK_IO's `length`: the disk, the direction and the sector count.
pub const fn io(disk: u16, write: bool, sectors: u32) -> u64 {
    (disk as u64) << 32 | if write { WRITE } else { 0 } | (sectors as u64 & 0xFF_FFFF)
}
/// (disk, write, sectors) from DISK_IO's `length`; None if a reserved bit is set.
pub const fn decode_io(length: u64) -> Option<(u16, bool, u32)> {
    if length >> 48 != 0 || length & 0x7F00_0000 != 0 { return None; }
    Some(((length >> 32) as u16, length & WRITE != 0, (length & 0xFF_FFFF) as u32))
}

/// Disk ids: the kind in the high byte, a number in the low one.
pub const KIND_SYSTEM: u16 = 1 << 8;
pub const KIND_USB: u16 = 2 << 8;
pub const KIND_IMAGE: u16 = 3 << 8;

pub const FLAG_READ_ONLY: u8 = 1;
/// The system disk, or one holding `/home`'s encrypted image: never written.
pub const FLAG_SYSTEM: u8 = 2;
/// This run may use it now.
pub const FLAG_GRANTED: u8 = 4;
pub const ENTRY_HEAD: usize = 11;
pub const NAME_MAX: usize = 32;
pub const PRODUCT_MAX: usize = 48;

/// One entry of [`DISKS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry<'a> { pub id: u16, pub flags: u8, pub sectors: u64, pub name: &'a [u8], pub product: &'a [u8] }
impl<'a> Entry<'a> {
    /// Appends the entry at `out[at..]`; the new end, or None if it does not fit.
    pub fn encode(&self, out: &mut [u8], at: usize) -> Option<usize> {
        let (name, product) = (&self.name[..self.name.len().min(NAME_MAX)], &self.product[..self.product.len().min(PRODUCT_MAX)]);
        let end = at + ENTRY_HEAD + 1 + name.len() + product.len();
        let o = out.get_mut(at..end)?;
        o[..2].copy_from_slice(&self.id.to_le_bytes());
        o[2] = self.flags;
        o[3..11].copy_from_slice(&self.sectors.to_le_bytes());
        o[11] = name.len() as u8;
        o[12..12 + name.len()].copy_from_slice(name);
        o[12 + name.len()] = product.len() as u8;
        o[13 + name.len()..].copy_from_slice(product);
        Some(end)
    }
    /// The entry at `data[at..]` and where the next one starts.
    pub fn decode(data: &'a [u8], at: usize) -> Option<(Entry<'a>, usize)> {
        let head = data.get(at..at + ENTRY_HEAD + 1)?;
        let n = head[11] as usize;
        let name = data.get(at + 12..at + 12 + n)?;
        let p = *data.get(at + 12 + n)? as usize;
        let product = data.get(at + 13 + n..at + 13 + n + p)?;
        Some((Entry { id: u16::from_le_bytes([head[0], head[1]]), flags: head[2], sectors: u64::from_le_bytes(head[3..11].try_into().ok()?), name, product },
              at + 13 + n + p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn entries_round_trip() {
        let mut buf = [0u8; 128];
        let a = Entry { id: KIND_USB, flags: FLAG_GRANTED, sectors: 524288, name: b"usb0", product: b"QEMU QEMU HARDDISK" };
        let b = Entry { id: KIND_SYSTEM, flags: FLAG_SYSTEM | FLAG_READ_ONLY, sectors: 1 << 21, name: b"system", product: b"" };
        let end = a.encode(&mut buf, 0).unwrap();
        let end = b.encode(&mut buf, end).unwrap();
        let (x, next) = Entry::decode(&buf[..end], 0).unwrap();
        let (y, last) = Entry::decode(&buf[..end], next).unwrap();
        assert_eq!((x, y, last), (a, b, end));
        assert!(Entry::decode(&buf[..end - 1], next).is_none());
        assert!(a.encode(&mut buf[..20], 0).is_none());
    }
    #[test]
    fn io_words() {
        assert_eq!(decode_io(io(KIND_USB | 3, true, 128)), Some((KIND_USB | 3, true, 128)));
        assert_eq!(decode_io(io(KIND_IMAGE, false, 1)), Some((KIND_IMAGE, false, 1)));
        assert_eq!(decode_io(1 << 48), None);
        assert_eq!(decode_io(1 << 24), None);
        assert!(IO_SECTORS * SECTOR as u64 <= crate::fs::IO_MAX as u64);
    }
}
