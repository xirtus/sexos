// SPDX-License-Identifier: GPL-3.0-only
//! MBR (primary and extended) and GPT partition tables.
use crate::{le32, le64, Disk, Result, SECTOR};
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Partition { pub start: u64, pub len: u64 }

pub fn table(disk: &mut dyn Disk) -> Result<Vec<Partition>> {
    let mut mbr = [0u8; SECTOR];
    disk.read(0, &mut mbr)?;
    let mut out = Vec::new();
    if mbr[510] != 0x55 || mbr[511] != 0xAA { return Ok(out); }
    let sectors = disk.sectors();
    let entry = |i: usize| { let e = &mbr[446 + 16 * i..462 + 16 * i]; (e[4], le32(e, 8) as u64, le32(e, 12) as u64) };
    if (0..4).any(|i| entry(i).0 == 0xEE) { return gpt(disk); }
    for i in 0..4 {
        let (kind, start, len) = entry(i);
        if kind == 0 || len == 0 || start + len > sectors { continue; }
        if matches!(kind, 0x05 | 0x0F | 0x85) { extended(disk, start, len, &mut out)?; } else { out.push(Partition { start, len }); }
    }
    Ok(out)
}
/// Logical partitions: a chain of boot records, each one entry and a link.
fn extended(disk: &mut dyn Disk, base: u64, size: u64, out: &mut Vec<Partition>) -> Result<()> {
    let mut at = base;
    for _ in 0..128 {
        let mut ebr = [0u8; SECTOR];
        disk.read(at, &mut ebr)?;
        if ebr[510] != 0x55 || ebr[511] != 0xAA { break; }
        let (kind, start, len) = (ebr[446 + 4], le32(&ebr, 446 + 8) as u64, le32(&ebr, 446 + 12) as u64);
        if kind != 0 && len != 0 && at + start + len <= base + size { out.push(Partition { start: at + start, len }); }
        let next = le32(&ebr, 462 + 8) as u64;
        if ebr[462 + 4] == 0 || next == 0 || next >= size { break; }
        at = base + next;
    }
    Ok(())
}
fn gpt(disk: &mut dyn Disk) -> Result<Vec<Partition>> {
    let mut head = [0u8; SECTOR];
    disk.read(1, &mut head)?;
    let mut out = Vec::new();
    if &head[..8] != b"EFI PART" { return Ok(out); }
    let (entries_lba, count, size) = (le64(&head, 72), le32(&head, 80) as u64, le32(&head, 84) as u64);
    if size < 128 || size > 4096 || count > 1024 { return Ok(out); }
    let mut sector = [0u8; SECTOR];
    let sectors = disk.sectors();
    for i in 0..count {
        let byte = i * size;
        if byte % SECTOR as u64 == 0 || i == 0 { disk.read(entries_lba + byte / SECTOR as u64, &mut sector)?; }
        let at = (byte % SECTOR as u64) as usize;
        if at + 128 > SECTOR { continue; }
        let e = &sector[at..at + 128];
        if e[..16].iter().all(|b| *b == 0) { continue; }
        let (first, last) = (le64(e, 32), le64(e, 40));
        if last < first || last >= sectors { continue; }
        out.push(Partition { start: first, len: last - first + 1 });
    }
    Ok(out)
}
