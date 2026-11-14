// SPDX-License-Identifier: GPL-3.0-only
//! Filesystem formatters (FAT32, exFAT, SexFS) and partition filesystem inspector.

use crate::device::{BlockIo, SexFsAdapter, SexVolumesAdapter, SECTOR_512};
use crate::error::CorsetError;
use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use sex_volumes::exfat::ExFat;
use sex_volumes::fat::Fat;

/// Detected filesystem type on a partition or device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilesystemType {
    Fat12,
    Fat16,
    Fat32,
    ExFat,
    SexFs,
    Ext4,
    Ntfs,
    EncryptedBdsm,
    Unknown,
}

impl FilesystemType {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Fat12 => "FAT12",
            Self::Fat16 => "FAT16",
            Self::Fat32 => "FAT32",
            Self::ExFat => "exFAT",
            Self::SexFs => "SexFS (Copy-On-Write)",
            Self::Ext4 => "ext4",
            Self::Ntfs => "NTFS",
            Self::EncryptedBdsm => "BDSM Encrypted Volume (XTS-AES-256)",
            Self::Unknown => "Unknown / Unformatted",
        }
    }
}

/// Inspection summary of a formatted partition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilesystemInspection {
    pub fs_type: FilesystemType,
    pub label: String,
    pub sector_size: u32,
    pub total_sectors: u64,
    pub cluster_bytes: Option<u64>,
}

// -----------------------------------------------------------------------------
// FAT32 Formatter
// -----------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Fat32FormatOptions<'a> {
    pub volume_label: &'a str,
    pub sectors_per_cluster: Option<u8>,
}

impl<'a> Default for Fat32FormatOptions<'a> {
    fn default() -> Self {
        Self {
            volume_label: "SEXOS",
            sectors_per_cluster: None,
        }
    }
}

/// Formats `dev` as a valid FAT32 filesystem.
/// Ensures valid BPB, FSInfo, backup boot sectors (LBA 6, 7), FAT1 + FAT2, and root directory.
/// Verifies immediately with `sex-volumes` mount!
pub fn format_fat32<D: BlockIo + ?Sized>(
    dev: &mut D,
    opts: Fat32FormatOptions<'_>,
) -> Result<(), CorsetError> {
    let sector_size = dev.sector_size();
    if sector_size != SECTOR_512 {
        return Err(CorsetError::UnsupportedSectorSize(sector_size));
    }
    let total_sectors = dev.sector_count();

    // FAT32 minimum cluster threshold is 65525 clusters.
    // Minimum sectors needed with 1 sector/cluster is ~66000 sectors (~33 MiB).
    if total_sectors < 66000 {
        return Err(CorsetError::VolumeTooSmallForFat32 {
            sectors: total_sectors,
            required: 66000,
        });
    }
    if total_sectors > 0xFFFF_FFFF {
        return Err(CorsetError::InvalidParameter("volume exceeds FAT32 maximum capacity (2 TiB)"));
    }

    let reserved_sectors: u16 = 32;
    let num_fats: u8 = 2;

    // Calculate sectors_per_cluster (must be power of 2, typically 1, 2, 4, 8)
    // such that data_clusters >= 65525
    let spc = match opts.sectors_per_cluster {
        Some(s) => {
            if !matches!(s, 1 | 2 | 4 | 8 | 16 | 32 | 64) {
                return Err(CorsetError::InvalidParameter("sectors_per_cluster must be power of 2 between 1 and 64"));
            }
            s
        }
        None => {
            let mut choice = 1u8;
            for candidate in [8u8, 4, 2, 1] {
                // Approximate data sectors
                let approx_fat = ((total_sectors * 4).div_ceil(512 * candidate as u64)) as u64;
                let data_sec = total_sectors.saturating_sub(reserved_sectors as u64 + (num_fats as u64) * approx_fat);
                let clusters = data_sec / candidate as u64;
                if clusters >= 65525 {
                    choice = candidate;
                    break;
                }
            }
            choice
        }
    };

    // Calculate exact FAT size in sectors
    // Formula: data_sectors = total_sectors - reserved - 2 * fat_sectors
    // clusters = data_sectors / spc
    // fat_bytes = (clusters + 2) * 4
    // fat_sectors = ceil(fat_bytes / 512)
    // Only ever grows, so it settles on a FAT that holds every cluster (an estimate
    // that moves both ways can stop one sector short).
    let mut fat_sectors: u32 = 128;
    for _ in 0..10 {
        let data_sectors = total_sectors.saturating_sub((reserved_sectors as u64) + (num_fats as u64) * (fat_sectors as u64));
        let clusters = data_sectors / (spc as u64);
        let needed_fat_bytes = (clusters + 2) * 4;
        let new_fat_sectors = needed_fat_bytes.div_ceil(512) as u32;
        if new_fat_sectors <= fat_sectors {
            break;
        }
        fat_sectors = new_fat_sectors;
    }

    let data_sectors = total_sectors.saturating_sub((reserved_sectors as u64) + (num_fats as u64) * (fat_sectors as u64));
    let total_clusters = (data_sectors / (spc as u64)) as u32;
    if total_clusters < 65525 {
        return Err(CorsetError::VolumeTooSmallForFat32 {
            sectors: total_sectors,
            required: (65525 * spc as u64) + (reserved_sectors as u64) + (num_fats as u64) * (fat_sectors as u64),
        });
    }
    if total_clusters >= 0x0FFF_FFF5 {
        return Err(CorsetError::InvalidParameter("too many clusters for FAT32 (exceeds 0x0FFFFFFF5)"));
    }

    // Prepare Volume Label (11 ASCII bytes, space padded)
    let mut label_11 = [b' '; 11];
    let label_bytes = opts.volume_label.as_bytes();
    for (i, &b) in label_bytes.iter().take(11).enumerate() {
        label_11[i] = b.to_ascii_uppercase();
    }

    // 1. Sector 0: Boot Sector / BPB
    let mut boot_sector = [0u8; 512];
    boot_sector[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]); // JMP short 0x58, NOP
    boot_sector[3..11].copy_from_slice(b"MSWIN4.1");
    boot_sector[11..13].copy_from_slice(&512u16.to_le_bytes()); // Bytes per sector
    boot_sector[13] = spc; // Sectors per cluster
    boot_sector[14..16].copy_from_slice(&reserved_sectors.to_le_bytes());
    boot_sector[16] = num_fats;
    boot_sector[17..19].copy_from_slice(&0u16.to_le_bytes()); // Root entries 0 for FAT32
    boot_sector[19..21].copy_from_slice(&0u16.to_le_bytes()); // Total sectors 16 (0)
    boot_sector[21] = 0xF8; // Media descriptor (Fixed disk)
    boot_sector[22..24].copy_from_slice(&0u16.to_le_bytes()); // FAT size 16 (0)
    boot_sector[24..26].copy_from_slice(&63u16.to_le_bytes()); // Sectors per track
    boot_sector[26..28].copy_from_slice(&255u16.to_le_bytes()); // Heads
    boot_sector[28..32].copy_from_slice(&0u32.to_le_bytes()); // Hidden sectors
    boot_sector[32..36].copy_from_slice(&(total_sectors as u32).to_le_bytes()); // Total sectors 32

    // FAT32 Extended BPB
    boot_sector[36..40].copy_from_slice(&fat_sectors.to_le_bytes()); // Sectors per FAT
    boot_sector[40..42].copy_from_slice(&0u16.to_le_bytes()); // Mirroring flags
    boot_sector[42..44].copy_from_slice(&0u16.to_le_bytes()); // FS Version
    boot_sector[44..48].copy_from_slice(&2u32.to_le_bytes()); // Root cluster = 2
    boot_sector[48..50].copy_from_slice(&1u16.to_le_bytes()); // FSInfo sector = 1
    boot_sector[50..52].copy_from_slice(&6u16.to_le_bytes()); // Backup boot sector = 6
    boot_sector[64] = 0x80; // Drive number
    boot_sector[66] = 0x29; // Extended boot signature
    boot_sector[67..71].copy_from_slice(&0x5E05_0001u32.to_le_bytes()); // Volume ID
    boot_sector[71..82].copy_from_slice(&label_11);
    boot_sector[82..90].copy_from_slice(b"FAT32   ");
    boot_sector[510] = 0x55;
    boot_sector[511] = 0xAA;

    // 2. Sector 1: FSInfo Sector
    let mut fsinfo = [0u8; 512];
    fsinfo[0..4].copy_from_slice(&0x4161_5252u32.to_le_bytes()); // LeadSig
    fsinfo[484..488].copy_from_slice(&0x6141_7272u32.to_le_bytes()); // StrucSig
    let free_clusters = total_clusters.saturating_sub(1); // Cluster 2 is root dir
    fsinfo[488..492].copy_from_slice(&free_clusters.to_le_bytes());
    fsinfo[492..496].copy_from_slice(&3u32.to_le_bytes()); // Next free cluster = 3
    fsinfo[508..512].copy_from_slice(&0xAA55_0000u32.to_le_bytes()); // TrailSig

    // Write Sector 0 (Main Boot) & Sector 1 (Main FSInfo)
    dev.write_sectors(0, &boot_sector)?;
    dev.write_sectors(1, &fsinfo)?;

    // Write Sector 6 (Backup Boot) & Sector 7 (Backup FSInfo)
    dev.write_sectors(6, &boot_sector)?;
    dev.write_sectors(7, &fsinfo)?;

    // 3. Clear and initialize FAT 1 and FAT 2 using bounded 32 KiB chunks
    let mut first_fat_sector = [0u8; 512];
    // Entry 0: 0x0FFF_FFF8 (Media descriptor in lower byte)
    first_fat_sector[0..4].copy_from_slice(&0x0FFF_FFF8u32.to_le_bytes());
    // Entry 1: 0x0FFF_FFFF (End of chain marker)
    first_fat_sector[4..8].copy_from_slice(&0x0FFF_FFFFu32.to_le_bytes());
    // Entry 2: 0x0FFF_FFFF (Root directory cluster end of chain)
    first_fat_sector[8..12].copy_from_slice(&0x0FFF_FFFFu32.to_le_bytes());

    let fat1_lba = reserved_sectors as u64;
    let fat2_lba = fat1_lba + fat_sectors as u64;
    dev.write_sectors(fat1_lba, &first_fat_sector)?;
    dev.write_sectors(fat2_lba, &first_fat_sector)?;

    if fat_sectors > 1 {
        let chunk_sectors = 64u64.min((fat_sectors - 1) as u64);
        let zero_buf = vec![0u8; (chunk_sectors as usize) * 512];
        let mut remaining = (fat_sectors - 1) as u64;
        let mut cur1 = fat1_lba + 1;
        let mut cur2 = fat2_lba + 1;
        while remaining > 0 {
            let to_write = remaining.min(chunk_sectors);
            let bytes_to_write = (to_write as usize) * 512;
            dev.write_sectors(cur1, &zero_buf[..bytes_to_write])?;
            dev.write_sectors(cur2, &zero_buf[..bytes_to_write])?;
            cur1 += to_write;
            cur2 += to_write;
            remaining -= to_write;
        }
    }

    // 4. Initialize Root Directory Cluster (Cluster 2)
    let root_lba = fat2_lba + fat_sectors as u64;
    let root_bytes = (spc as usize) * 512;
    let mut root_cluster = vec![0u8; root_bytes];

    // Volume label directory entry (ATTR_VOLUME = 0x08)
    if !opts.volume_label.is_empty() {
        root_cluster[0..11].copy_from_slice(&label_11);
        root_cluster[11] = 0x08; // ATTR_VOLUME
    }
    dev.write_sectors(root_lba, &root_cluster)?;

    dev.flush()?;

    // 5. Cross-crate integration verification: sex-volumes must mount it immediately!
    let mut adapter = SexVolumesAdapter(dev);
    let mounted = Fat::mount(&mut adapter)
        .map_err(|_| CorsetError::FilesystemMountFailed("sex-volumes FAT32 mount verification failed"))?;
    let _ = mounted;

    Ok(())
}

// -----------------------------------------------------------------------------
// exFAT Formatter
// -----------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ExFatFormatOptions<'a> {
    pub volume_label: &'a str,
}

impl<'a> Default for ExFatFormatOptions<'a> {
    fn default() -> Self {
        Self { volume_label: "SEXOS_EXFAT" }
    }
}

/// Computes the standard exFAT boot region checksum.
fn exfat_boot_checksum(sector: &[u8], is_sector_0: bool) -> u32 {
    let mut sum = 0u32;
    for (i, &b) in sector.iter().enumerate() {
        if is_sector_0 && (i == 106 || i == 107 || i == 112) {
            continue;
        }
        sum = sum.rotate_right(1).wrapping_add(b as u32);
    }
    sum
}

/// Formats `dev` as a valid exFAT filesystem with standard boot region, checksum,
/// allocation bitmap, up-case table, and root directory.
/// Verifies immediately with `sex-volumes` mount!
pub fn format_exfat<D: BlockIo + ?Sized>(
    dev: &mut D,
    opts: ExFatFormatOptions<'_>,
) -> Result<(), CorsetError> {
    let sector_size = dev.sector_size();
    if sector_size != SECTOR_512 {
        return Err(CorsetError::UnsupportedSectorSize(sector_size));
    }
    let total_sectors = dev.sector_count();
    if total_sectors < 2048 {
        return Err(CorsetError::VolumeTooSmallForExFat {
            sectors: total_sectors,
            required: 2048,
        });
    }

    let bps_shift: u8 = 9; // 2^9 = 512 bytes
    // The allocation bitmap, the up-case table and the root directory take one cluster
    // each, so a cluster must be large enough for the bitmap of every cluster: the
    // smallest power of two from 4 KiB (8 sectors) that is.
    let mut spc_shift: u8 = 3;
    while spc_shift < 16 && total_sectors / (1u64 << spc_shift) > (512u64 << spc_shift) * 8 {
        spc_shift += 1;
    }
    let spc: u32 = 1 << spc_shift;
    let cluster_bytes: u64 = 512 << spc_shift;

    // Boot region: 24 sectors (12 main + 12 backup)
    let fat_offset: u32 = 24;

    // Approximate cluster count to determine FAT length
    let approx_heap_sectors = total_sectors.saturating_sub(64);
    let approx_clusters = (approx_heap_sectors / spc as u64) as u32;
    let fat_length: u32 = ((approx_clusters as u64 + 2) * 4).div_ceil(512) as u32;

    // Align cluster heap to multiple of 8 sectors
    let raw_heap_offset = fat_offset + fat_length;
    let heap_offset = (raw_heap_offset + spc - 1) & !(spc - 1);

    let heap_sectors = total_sectors.saturating_sub(heap_offset as u64);
    let cluster_count = (heap_sectors / spc as u64) as u32;
    if cluster_count < 16 {
        return Err(CorsetError::VolumeTooSmallForExFat {
            sectors: total_sectors,
            required: 2048,
        });
    }

    let root_cluster: u32 = 4; // Cluster 2 = Bitmap, 3 = Upcase, 4 = Root Directory

    // 1. Prepare Main Boot Sector (Sector 0)
    let mut boot_sector = [0u8; 512];
    boot_sector[0..3].copy_from_slice(&[0xEB, 0x76, 0x90]); // JMP short 0x76, NOP
    boot_sector[3..11].copy_from_slice(b"EXFAT   ");
    // Bytes 11..64 must be zero
    boot_sector[64..72].copy_from_slice(&0u64.to_le_bytes()); // PartitionOffset
    boot_sector[72..80].copy_from_slice(&total_sectors.to_le_bytes()); // VolumeLength
    boot_sector[80..84].copy_from_slice(&fat_offset.to_le_bytes()); // FatOffset
    boot_sector[84..88].copy_from_slice(&fat_length.to_le_bytes()); // FatLength
    boot_sector[88..92].copy_from_slice(&heap_offset.to_le_bytes()); // ClusterHeapOffset
    boot_sector[92..96].copy_from_slice(&cluster_count.to_le_bytes()); // ClusterCount
    boot_sector[96..100].copy_from_slice(&root_cluster.to_le_bytes()); // FirstClusterOfRootDir
    boot_sector[100..104].copy_from_slice(&0x5E05_BEEFu32.to_le_bytes()); // VolumeSerialNumber
    boot_sector[104..106].copy_from_slice(&0x0100u16.to_le_bytes()); // Revision 1.00
    boot_sector[106..108].copy_from_slice(&0u16.to_le_bytes()); // VolumeFlags
    boot_sector[108] = bps_shift;
    boot_sector[109] = spc_shift;
    boot_sector[110] = 1; // NumberOfFats
    boot_sector[111] = 0x80; // DriveSelect
    boot_sector[112] = 0; // PercentInUse
    boot_sector[510] = 0x55;
    boot_sector[511] = 0xAA;

    // Compute boot checksum over sectors 0..10
    let mut sum = exfat_boot_checksum(&boot_sector, true);
    for _ in 1..11 {
        let empty_sec = [0u8; 512];
        let mut s = empty_sec;
        s[510] = 0x55;
        s[511] = 0xAA;
        for &b in &s {
            sum = sum.rotate_right(1).wrapping_add(b as u32);
        }
    }

    // Sector 11: Checksum Sector (repeated 128 times)
    let mut checksum_sector = [0u8; 512];
    for chunk in checksum_sector.chunks_exact_mut(4) {
        chunk.copy_from_slice(&sum.to_le_bytes());
    }

    // Write Main Boot Region (Sectors 0..11)
    dev.write_sectors(0, &boot_sector)?;
    for s in 1..11 {
        let mut empty = [0u8; 512];
        empty[510] = 0x55;
        empty[511] = 0xAA;
        dev.write_sectors(s, &empty)?;
    }
    dev.write_sectors(11, &checksum_sector)?;

    // Write Backup Boot Region (Sectors 12..23)
    dev.write_sectors(12, &boot_sector)?;
    for s in 13..23 {
        let mut empty = [0u8; 512];
        empty[510] = 0x55;
        empty[511] = 0xAA;
        dev.write_sectors(s, &empty)?;
    }
    dev.write_sectors(23, &checksum_sector)?;

    // 2. Write FAT (Sectors 24..24+fat_length) using bounded chunks
    let mut first_fat = [0u8; 512];
    // Entries 0, 1: Media descriptor and reserved
    first_fat[0..4].copy_from_slice(&0xFFFF_FFF8u32.to_le_bytes());
    first_fat[4..8].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    // Cluster 2 (Bitmap): End of chain (or 1 cluster run)
    first_fat[8..12].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    // Cluster 3 (Upcase): End of chain
    first_fat[12..16].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    // Cluster 4 (Root Dir): End of chain
    first_fat[16..20].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    dev.write_sectors(fat_offset as u64, &first_fat)?;

    if fat_length > 1 {
        let chunk_sectors = 64u64.min((fat_length - 1) as u64);
        let zero_buf = vec![0u8; (chunk_sectors as usize) * 512];
        let mut remaining = (fat_length - 1) as u64;
        let mut cur = (fat_offset + 1) as u64;
        while remaining > 0 {
            let to_write = remaining.min(chunk_sectors);
            let bytes_to_write = (to_write as usize) * 512;
            dev.write_sectors(cur, &zero_buf[..bytes_to_write])?;
            cur += to_write;
            remaining -= to_write;
        }
    }

    // 3. Cluster Heap
    let cluster_offset = |c: u32| -> u64 {
        heap_offset as u64 + ((c - 2) as u64) * spc as u64
    };

    // Cluster 2: Allocation Bitmap (clusters 2, 3, 4 are allocated = bits 0, 1, 2)
    let mut bitmap_cluster = vec![0u8; cluster_bytes as usize];
    bitmap_cluster[0] = 0x07; // 0b00000111: clusters 2, 3, 4 used
    dev.write_sectors(cluster_offset(2), &bitmap_cluster)?;

    // Cluster 3: Up-case table
    // 0xFFFF 0x0100 indicates compressed identity for 256 units (ASCII fallback)
    let mut upcase_cluster = vec![0u8; cluster_bytes as usize];
    upcase_cluster[0..4].copy_from_slice(&[0xFF, 0xFF, 0x00, 0x01]);
    dev.write_sectors(cluster_offset(3), &upcase_cluster)?;

    // Cluster 4: Root Directory
    let mut root_cluster_data = vec![0u8; cluster_bytes as usize];

    // Entry 1: Allocation Bitmap (0x81)
    let bm_entry = &mut root_cluster_data[0..32];
    bm_entry[0] = 0x81;
    bm_entry[1] = 0; // BitmapFlags: 1st bitmap
    bm_entry[20..24].copy_from_slice(&2u32.to_le_bytes()); // FirstCluster = 2
    let bitmap_len = (cluster_count as u64).div_ceil(8);
    bm_entry[24..32].copy_from_slice(&bitmap_len.to_le_bytes()); // DataLength

    // Entry 2: Up-case Table (0x82)
    let up_entry = &mut root_cluster_data[32..64];
    up_entry[0] = 0x82;
    up_entry[20..24].copy_from_slice(&3u32.to_le_bytes()); // FirstCluster = 3
    let upcase_len = 4u64;
    up_entry[24..32].copy_from_slice(&upcase_len.to_le_bytes()); // DataLength

    // Entry 3: Volume Label (0x83) if label is non-empty
    if !opts.volume_label.is_empty() {
        let vl_entry = &mut root_cluster_data[64..96];
        vl_entry[0] = 0x83;
        let u16_chars: Vec<u16> = opts.volume_label.encode_utf16().take(11).collect();
        vl_entry[1] = u16_chars.len() as u8;
        for (i, &u) in u16_chars.iter().enumerate() {
            vl_entry[2 + 2 * i..4 + 2 * i].copy_from_slice(&u.to_le_bytes());
        }
    }

    dev.write_sectors(cluster_offset(4), &root_cluster_data)?;

    dev.flush()?;

    // 4. Cross-crate integration verification: sex-volumes must mount it immediately!
    let mut adapter = SexVolumesAdapter(dev);
    let mounted = ExFat::mount(&mut adapter)
        .map_err(|_| CorsetError::FilesystemMountFailed("sex-volumes exFAT mount verification failed"))?;
    let _ = mounted;

    Ok(())
}

// -----------------------------------------------------------------------------
// SexFS Formatter
// -----------------------------------------------------------------------------

struct SexFsMemStore {
    cache: Vec<[u8; sexfs::BLOCK]>,
    slots: Vec<sexfs::Slot>,
    used: Vec<u64>,
    pending: Vec<u64>,
    scratch: Box<[u8; sexfs::BLOCK]>,
}

impl SexFsMemStore {
    fn new(blocks: usize) -> Self {
        let tracking_words = blocks.div_ceil(64).max(64);
        Self {
            cache: vec![[0; sexfs::BLOCK]; 64],
            slots: vec![sexfs::Slot::EMPTY; 64],
            used: vec![0; tracking_words],
            pending: vec![0; tracking_words],
            scratch: Box::new([0; sexfs::BLOCK]),
        }
    }

    fn memory(&mut self) -> sexfs::Memory<'_> {
        sexfs::Memory {
            cache: &mut self.cache,
            slots: &mut self.slots,
            used: &mut self.used,
            pending: &mut self.pending,
            scratch: &mut self.scratch,
        }
    }
}

/// Formats `dev` with a clean SexFS copy-on-write filesystem tree.
pub fn format_sexfs<D: BlockIo + ?Sized>(dev: &mut D) -> Result<(), CorsetError> {
    let total_bytes = dev.sector_count() * dev.sector_size() as u64;
    let total_blocks = (total_bytes / sexfs::BLOCK as u64) as usize;
    if total_blocks < 512 {
        return Err(CorsetError::NoSpace("device too small for SexFS (needs >= 512 4KiB blocks)"));
    }

    let mut store = SexFsMemStore::new(total_blocks);
    let adapter = SexFsAdapter(dev);
    let fs = sexfs::Fs::format(adapter, store.memory())
        .map_err(|_| CorsetError::FilesystemMountFailed("sexfs format failed"))?;
    let _ = fs;
    dev.flush()?;
    Ok(())
}

// -----------------------------------------------------------------------------
// Filesystem Inspector
// -----------------------------------------------------------------------------

/// Inspects the partition or device to detect its filesystem and extract metadata.
pub fn inspect_filesystem<D: BlockIo + ?Sized>(dev: &mut D) -> Result<FilesystemInspection, CorsetError> {
    let sector_size = dev.sector_size();
    let total_sectors = dev.sector_count();
    let mut boot = vec![0u8; sector_size as usize];

    dev.read_sectors(0, &mut boot)?;

    // 1. Check for SexOS BDSM Encrypted Volume (sexcrypt header)
    if boot.len() >= 8 && &boot[..8] == sex_crypt::MAGIC {
        return Ok(FilesystemInspection {
            fs_type: FilesystemType::EncryptedBdsm,
            label: "Encrypted Volume".to_string(),
            sector_size,
            total_sectors,
            cluster_bytes: None,
        });
    }

    // 2. Check for exFAT
    if boot.len() >= 11 && &boot[3..11] == b"EXFAT   " && boot.len() >= 512 && boot[510..512] == [0x55, 0xAA] {
        let spc = boot[109] as u64;
        let cluster_bytes = (sector_size as u64) << spc;
        let mut adapter = SexVolumesAdapter(dev);
        let label = if let Ok(vol) = ExFat::mount(&mut adapter) {
            use sex_volumes::Volume;
            vol.label()
        } else {
            String::new()
        };
        return Ok(FilesystemInspection {
            fs_type: FilesystemType::ExFat,
            label,
            sector_size,
            total_sectors,
            cluster_bytes: Some(cluster_bytes),
        });
    }

    // 3. Check for NTFS
    if boot.len() >= 11 && &boot[3..11] == b"NTFS    " {
        return Ok(FilesystemInspection {
            fs_type: FilesystemType::Ntfs,
            label: "NTFS Volume".to_string(),
            sector_size,
            total_sectors,
            cluster_bytes: None,
        });
    }

    // 4. Check for ext4 (Superblock at offset 1024 / sector 2)
    if total_sectors > 3 {
        let mut sb = [0u8; 512];
        if dev.read_sectors(2, &mut sb).is_ok() {
            if u16::from_le_bytes([sb[56], sb[57]]) == 0xEF53 {
                return Ok(FilesystemInspection {
                    fs_type: FilesystemType::Ext4,
                    label: "ext4 Volume".to_string(),
                    sector_size,
                    total_sectors,
                    cluster_bytes: Some(4096),
                });
            }
        }
    }

    // 5. Check for SexFS (Superblock 0 at block 0, or superblock 1 at block 1)
    if total_sectors >= 16 {
        let mut sb = [0u8; 4096];
        for slot in [0u64, 4096] {
            if dev.read_bytes_at(slot, &mut sb).is_ok() {
                let magic = u64::from_le_bytes(sb[..8].try_into().unwrap());
                // MAGIC_SB = 0x3142_5346_5845_53
                if magic == 0x3142_5346_5845_53 {
                    return Ok(FilesystemInspection {
                        fs_type: FilesystemType::SexFs,
                        label: "SexFS".to_string(),
                        sector_size,
                        total_sectors,
                        cluster_bytes: Some(4096),
                    });
                }
            }
        }
    }

    // 6. Check for FAT12 / FAT16 / FAT32
    if boot.len() >= 512 && boot[510] == 0x55 && boot[511] == 0xAA && sex_volumes::fat::looks_like_fat(&boot) {
        let bps = u16::from_le_bytes([boot[11], boot[12]]) as u64;
        let spc = boot[13] as u64;
        let cluster_bytes = spc * bps;

        let mut adapter = SexVolumesAdapter(dev);
        if let Ok(vol) = Fat::mount(&mut adapter) {
            use sex_volumes::Volume;
            let format = match vol.format() {
                sex_volumes::Format::Fat12 => FilesystemType::Fat12,
                sex_volumes::Format::Fat16 => FilesystemType::Fat16,
                _ => FilesystemType::Fat32,
            };
            return Ok(FilesystemInspection {
                fs_type: format,
                label: vol.label(),
                sector_size,
                total_sectors,
                cluster_bytes: Some(cluster_bytes),
            });
        }
    }

    Ok(FilesystemInspection {
        fs_type: FilesystemType::Unknown,
        label: String::new(),
        sector_size,
        total_sectors,
        cluster_bytes: None,
    })
}
