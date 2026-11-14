// SPDX-License-Identifier: GPL-3.0-only
//! Making an exFAT volume (Microsoft's exFAT specification, 2019): main and backup boot
//! regions with their checksum, one FAT, then the cluster heap holding the allocation
//! bitmap, the up-case table and the root directory, each chained in the FAT.
//!
//! The up-case table is the compressed form mapping only a-z to A-Z, as small formatters
//! write it; names outside ASCII then compare exactly. sex-volumes, Linux and Windows read
//! whatever table the volume carries.
use crate::{BlockDevice, Error, SECTOR};
use alloc::vec;
use alloc::vec::Vec;

/// 1 MiB: room for the boot regions, the FAT and a few clusters.
pub const MIN_SECTORS: u64 = 2048;
pub const MAX_SECTORS: u64 = 0xFFFF_FFF5u64 << 8;
const FAT_OFFSET: u64 = 128;

pub struct Geometry { pub spc_shift: u32, pub fat_len: u64, pub heap: u64, pub count: u32 }

pub fn geometry(sectors: u64) -> Option<Geometry> {
    if !(MIN_SECTORS..=MAX_SECTORS).contains(&sectors) { return None; }
    let spc_shift = match sectors { 0..=524_288 => 3, 524_289..=67_108_864 => 6, _ => 8 };
    let spc = 1u64 << spc_shift;
    let mut count = (sectors - FAT_OFFSET) / spc;
    loop {
        let fat_len = ((count + 2) * 4).div_ceil(SECTOR as u64);
        let heap = (FAT_OFFSET + fat_len).next_multiple_of(spc);
        let fits = sectors.checked_sub(heap)? / spc;
        if fits >= count { return (count >= 4).then_some(Geometry { spc_shift, fat_len, heap, count: count.min(0xFFFF_FFF5) as u32 }); }
        count = fits;
    }
}

fn boot_checksum(sectors: &[u8]) -> u32 {
    let mut c = 0u32;
    for (i, b) in sectors.iter().enumerate() {
        if i == 106 || i == 107 || i == 112 { continue; }
        c = c.rotate_right(1).wrapping_add(*b as u32);
    }
    c
}
fn table_checksum(t: &[u8]) -> u32 { t.iter().fold(0u32, |c, b| c.rotate_right(1).wrapping_add(*b as u32)) }

/// The compressed up-case table: identity except a-z.
fn upcase() -> Vec<u8> {
    let mut units: Vec<u16> = vec![0xFFFF, 0x61];
    units.extend(0x41..=0x5A);
    units.extend([0xFFFF, (0x1_0000 - 0x7B) as u16]);
    units.iter().flat_map(|u| u.to_le_bytes()).collect()
}

pub fn format(dev: &mut dyn BlockDevice, label: &str, serial: u32) -> Result<(), Error> {
    let sectors = dev.sectors();
    let g = geometry(sectors).ok_or(Error::Format)?;
    let label: Vec<u16> = label.encode_utf16().collect();
    if label.len() > 11 { return Err(Error::Format); }
    let spc = 1u64 << g.spc_shift;
    let csize = spc as usize * SECTOR;
    let bitmap_bytes = (g.count as u64).div_ceil(8);
    let bitmap_clusters = bitmap_bytes.div_ceil(csize as u64) as u32;
    let up = upcase();
    let (bitmap_c, upcase_c, root_c) = (2u32, 2 + bitmap_clusters, 3 + bitmap_clusters);
    let used = bitmap_clusters + 2;
    if used >= g.count { return Err(Error::Format); }

    // Boot region: boot sector, 8 extended boot sectors, OEM parameters, reserved, checksum.
    let mut region = vec![0u8; 12 * SECTOR];
    {
        let b = &mut region[..SECTOR];
        b[..3].copy_from_slice(&[0xEB, 0x76, 0x90]);
        b[3..11].copy_from_slice(b"EXFAT   ");
        b[72..80].copy_from_slice(&sectors.to_le_bytes());
        b[80..84].copy_from_slice(&(FAT_OFFSET as u32).to_le_bytes());
        b[84..88].copy_from_slice(&(g.fat_len as u32).to_le_bytes());
        b[88..92].copy_from_slice(&(g.heap as u32).to_le_bytes());
        b[92..96].copy_from_slice(&g.count.to_le_bytes());
        b[96..100].copy_from_slice(&root_c.to_le_bytes());
        b[100..104].copy_from_slice(&serial.to_le_bytes());
        b[104..106].copy_from_slice(&0x0100u16.to_le_bytes());
        b[108] = 9;
        b[109] = g.spc_shift as u8;
        b[110] = 1;
        b[111] = 0x80;
        b[112] = (used as u64 * 100 / g.count as u64) as u8;
        b[120..510].fill(0xF4); // hlt: not bootable
        b[510] = 0x55; b[511] = 0xAA;
    }
    for s in 1..=8 { region[s * SECTOR + 510] = 0x55; region[s * SECTOR + 511] = 0xAA; }
    let sum = boot_checksum(&region[..11 * SECTOR]);
    for w in region[11 * SECTOR..].chunks_exact_mut(4) { w.copy_from_slice(&sum.to_le_bytes()); }

    // Clear everything up to the end of the root directory's cluster, then fill in.
    let clear_end = g.heap + (used as u64) * spc;
    let zero = vec![0u8; 64 * SECTOR];
    let mut at = 0;
    while at < clear_end {
        let n = (clear_end - at).min(64);
        dev.write(at, &zero[..n as usize * SECTOR])?;
        at += n;
    }
    // FAT: media and end markers, then one chain per structure.
    let mut fat = vec![0u8; (used as usize + 2) * 4];
    let mut put = |c: u32, v: u32| fat[c as usize * 4..c as usize * 4 + 4].copy_from_slice(&v.to_le_bytes());
    put(0, 0xFFFF_FFF8);
    put(1, 0xFFFF_FFFF);
    for c in bitmap_c..upcase_c { put(c, if c + 1 == upcase_c { 0xFFFF_FFFF } else { c + 1 }); }
    put(upcase_c, 0xFFFF_FFFF);
    put(root_c, 0xFFFF_FFFF);
    fat.resize(fat.len().next_multiple_of(SECTOR), 0);
    dev.write(FAT_OFFSET, &fat)?;
    let cluster_lba = |c: u32| g.heap + (c as u64 - 2) * spc;
    // Allocation bitmap: the first `used` clusters.
    let mut bm = vec![0u8; (bitmap_clusters as usize * csize).min(bitmap_bytes.next_multiple_of(SECTOR as u64) as usize)];
    for i in 0..used as usize { bm[i / 8] |= 1 << (i % 8); }
    dev.write(cluster_lba(bitmap_c), &bm)?;
    let mut t = up.clone();
    t.resize(t.len().next_multiple_of(SECTOR), 0);
    dev.write(cluster_lba(upcase_c), &t)?;
    // Root directory: label, bitmap, up-case.
    let mut root = vec![0u8; SECTOR];
    root[0] = 0x83;
    root[1] = label.len() as u8;
    for (i, u) in label.iter().enumerate() { root[2 + 2 * i..4 + 2 * i].copy_from_slice(&u.to_le_bytes()); }
    let e = &mut root[32..64];
    e[0] = 0x81;
    e[20..24].copy_from_slice(&bitmap_c.to_le_bytes());
    e[24..32].copy_from_slice(&bitmap_bytes.to_le_bytes());
    let e = &mut root[64..96];
    e[0] = 0x82;
    e[4..8].copy_from_slice(&table_checksum(&up).to_le_bytes());
    e[20..24].copy_from_slice(&upcase_c.to_le_bytes());
    e[24..32].copy_from_slice(&(up.len() as u64).to_le_bytes());
    dev.write(cluster_lba(root_c), &root)?;
    // The boot regions last: until then nothing on the volume claims to be exFAT.
    dev.write(12, &region)?;
    dev.write(0, &region)?;
    dev.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_fits_and_the_up_case_table_is_complete() {
        for mib in [1u64, 2, 33, 256, 257, 4096] {
            let s = mib << 11;
            let g = geometry(s).unwrap();
            let spc = 1u64 << g.spc_shift;
            assert!(g.heap + g.count as u64 * spc <= s, "{mib}");
            assert!(g.fat_len * SECTOR as u64 >= (g.count as u64 + 2) * 4);
            assert_eq!(g.heap % spc, 0);
            // At most one cluster's worth wasted beyond alignment.
            assert!(s - (g.heap + g.count as u64 * spc) < spc + 2 * spc, "{mib}");
        }
        assert!(geometry(2047).is_none());
        let t = upcase();
        let mut ch = 0u32;
        let mut i = 0;
        while i < t.len() {
            let u = u16::from_le_bytes([t[i], t[i + 1]]);
            if u == 0xFFFF { ch += u16::from_le_bytes([t[i + 2], t[i + 3]]) as u32; i += 4; } else { ch += 1; i += 2; }
        }
        assert_eq!(ch, 0x1_0000);
    }
}
