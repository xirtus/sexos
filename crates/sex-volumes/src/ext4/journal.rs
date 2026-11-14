// SPDX-License-Identifier: GPL-3.0-only
//! Replaying an ext3/ext4 journal (jbd2) that Linux left behind: a stick pulled out, or a
//! machine that lost power, before its last transactions reached their home blocks.
//!
//! The journal is read the way the format defines it: from the journal superblock's
//! `s_start` and `s_sequence`, block by block around the log, every descriptor's data
//! blocks counted, every revoke remembered, until a block isn't the next one the log
//! expects. Only transactions whose commit block was found are replayed; a block a later
//! transaction revoked is not. Then the journal is marked empty and the volume no longer
//! needs recovery, exactly as Linux leaves it after mounting.
//!
//! Refused (the volume stays read only and untouched): fast commits, and journal
//! features this doesn't know. Block checksums are not verified.
use super::{Ext4, Cache};
use crate::{read_bytes, write_bytes, Disk, Error, Result};
use alloc::{collections::BTreeMap, vec::Vec};

const MAGIC: u32 = 0xC03B_3998;
const DESCRIPTOR: u32 = 1;
const COMMIT: u32 = 2;
const SUPER_V1: u32 = 3;
const SUPER_V2: u32 = 4;
const REVOKE: u32 = 5;
const F_REVOKE: u32 = 0x1;
const F_64BIT: u32 = 0x2;
const F_ASYNC_COMMIT: u32 = 0x4;
const F_CSUM_V2: u32 = 0x8;
const F_CSUM_V3: u32 = 0x10;
const KNOWN: u32 = F_REVOKE | F_64BIT | F_ASYNC_COMMIT | F_CSUM_V2 | F_CSUM_V3;
const TAG_ESCAPE: u32 = 0x1;
const TAG_SAME_UUID: u32 = 0x2;
const TAG_LAST: u32 = 0x8;

fn be32(b: &[u8], at: usize) -> u32 { u32::from_be_bytes(b[at..at + 4].try_into().unwrap()) }

/// The journal as a file: its blocks in log order, mapped to the volume's.
struct Log { map: Vec<u64>, first: u64, last: u64, bs: usize }
impl Log {
    fn next(&self, at: u64) -> u64 { if at + 1 >= self.last { self.first } else { at + 1 } }
    fn read(&self, v: &Ext4, disk: &mut dyn Disk, at: u64) -> Result<Vec<u8>> {
        let phys = *self.map.get(at as usize).ok_or(Error::Corrupt)?;
        v.block(disk, phys)
    }
}

/// One committed transaction's blocks: (home block, log position, escaped).
struct Tx { seq: u32, blocks: Vec<(u64, u64, bool)> }

/// Replays the journal if it holds anything. Ok(n): n blocks were written home, the
/// journal is empty and the volume no longer needs recovery.
pub(super) fn replay(v: &mut Ext4, disk: &mut dyn Disk, inum: u32) -> Result<usize> {
    let ino = v.inode(disk, inum)?;
    let blocks = ino.size / v.bs;
    if blocks < 2 || blocks > 1 << 24 { return Err(Error::Corrupt); }
    let mut map = Vec::with_capacity(blocks as usize);
    let mut cache = Cache::default();
    while (map.len() as u64) < blocks {
        let (phys, run) = v.map(disk, &mut cache, &ino, map.len() as u64)?;
        let phys = phys.ok_or(Error::Corrupt)?;
        for i in 0..run.min(blocks - map.len() as u64) { map.push(phys + i); }
    }
    let mut sb = v.block(disk, map[0])?;
    let kind = be32(&sb, 4);
    if be32(&sb, 0) != MAGIC || !(kind == SUPER_V1 || kind == SUPER_V2) { return Err(Error::Corrupt); }
    if be32(&sb, 0x0C) as u64 != v.bs { return Err(Error::Corrupt); }
    let incompat = if kind == SUPER_V2 { be32(&sb, 0x28) } else { 0 };
    if incompat & !KNOWN != 0 { return Err(Error::Unsupported); }
    let (maxlen, first) = (be32(&sb, 0x10) as u64, be32(&sb, 0x14) as u64);
    let (mut seq, start) = (be32(&sb, 0x18), be32(&sb, 0x1C) as u64);
    if maxlen > blocks || first == 0 || first >= maxlen { return Err(Error::Corrupt); }
    let log = Log { map, first, last: maxlen, bs: v.bs as usize };
    let mut written = 0;
    if start != 0 {
        if start < first || start >= maxlen { return Err(Error::Corrupt); }
        let wide = incompat & F_64BIT != 0;
        let csum = incompat & (F_CSUM_V2 | F_CSUM_V3) != 0;
        let tag = if incompat & F_CSUM_V3 != 0 { 16 } else { 8 + if wide { 4 } else { 0 } + if incompat & F_CSUM_V2 != 0 { 2 } else { 0 } };
        // Pass 1: walk the log. Transactions without their commit are dropped.
        let mut done: Vec<Tx> = Vec::new();
        let mut revoked: BTreeMap<u64, u32> = BTreeMap::new();
        let mut open = Tx { seq, blocks: Vec::new() };
        let mut open_revokes: Vec<u64> = Vec::new();
        let mut at = start;
        for _ in 0..maxlen {
            let b = log.read(v, disk, at)?;
            if be32(&b, 0) != MAGIC || be32(&b, 8) != seq { break; }
            match be32(&b, 4) {
                DESCRIPTOR => {
                    let end = log.bs - if csum { 4 } else { 0 };
                    let mut p = 12;
                    loop {
                        if p + tag > end { break; }
                        let lo = be32(&b, p) as u64;
                        let (flags, hi) = if incompat & F_CSUM_V3 != 0 { (be32(&b, p + 4), be32(&b, p + 8) as u64) }
                            else { (u16::from_be_bytes([b[p + 6], b[p + 7]]) as u32, if wide { be32(&b, p + 8) as u64 } else { 0 }) };
                        at = log.next(at);
                        open.blocks.push((if wide { hi << 32 | lo } else { lo }, at, flags & TAG_ESCAPE != 0));
                        p += tag + if flags & TAG_SAME_UUID == 0 { 16 } else { 0 };
                        if flags & TAG_LAST != 0 { break; }
                    }
                }
                REVOKE => {
                    let used = (be32(&b, 12) as usize).min(log.bs);
                    let size = if wide { 8 } else { 4 };
                    let mut p = 16;
                    while p + size <= used {
                        open_revokes.push(if wide { (be32(&b, p) as u64) << 32 | be32(&b, p + 4) as u64 } else { be32(&b, p) as u64 });
                        p += size;
                    }
                }
                COMMIT => {
                    for r in open_revokes.drain(..) { let e = revoked.entry(r).or_insert(seq); *e = (*e).max(seq); }
                    done.push(core::mem::replace(&mut open, Tx { seq: seq.wrapping_add(1), blocks: Vec::new() }));
                    seq = seq.wrapping_add(1);
                }
                _ => break,
            }
            at = log.next(at);
        }
        // Pass 2: each committed block home, unless a transaction at or after it revoked it.
        for tx in &done {
            for &(home, pos, escaped) in &tx.blocks {
                if revoked.get(&home).is_some_and(|r| r.wrapping_sub(tx.seq) as i32 >= 0) { continue; }
                if home == 0 || home >= v.blocks { return Err(Error::Corrupt); }
                let mut data = log.read(v, disk, pos)?;
                if escaped { data[..4].copy_from_slice(&MAGIC.to_be_bytes()); }
                write_bytes(disk, home * v.bs, &data)?;
                written += 1;
            }
        }
        disk.flush()?;
    }
    // The journal is empty from here on, and the next transaction is the one after.
    sb[0x18..0x1C].copy_from_slice(&seq.to_be_bytes());
    sb[0x1C..0x20].fill(0);
    if kind == SUPER_V2 && incompat & (F_CSUM_V2 | F_CSUM_V3) != 0 {
        sb[0xFC..0x100].fill(0);
        let c = super::write::crc32c(!0, &sb[..1024]);
        sb[0xFC..0x100].copy_from_slice(&c.to_be_bytes());
    }
    write_bytes(disk, log.map[0] * v.bs, &sb[..1024])?;
    // The volume no longer needs recovery.
    let mut s = [0u8; 1024];
    read_bytes(disk, 1024, &mut s)?;
    let incompat_fs = u32::from_le_bytes(s[0x60..0x64].try_into().unwrap()) & !0x4;
    s[0x60..0x64].copy_from_slice(&incompat_fs.to_le_bytes());
    if u32::from_le_bytes(s[0x64..0x68].try_into().unwrap()) & 0x400 != 0 {
        let c = super::write::crc32c(!0, &s[..0x3FC]);
        s[0x3FC..0x400].copy_from_slice(&c.to_le_bytes());
    }
    write_bytes(disk, 1024, &s)?;
    disk.flush()?;
    Ok(written)
}
