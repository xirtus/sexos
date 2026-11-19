// SPDX-License-Identifier: GPL-3.0-only
//! Corset: Disks and partitions engine for SexOS (BDSM storage stack).
//!
//! Provides:
//! - Block device abstraction (`BlockIo`, `MemBlockDevice`, `PartitionIo`, and cross-crate adapters)
//! - Disk layout scanner and visual map (`layout`, IEEE CRC32 verification, `DiskMap`, `DiskRegion`)
//! - Non-destructive GPT partition editor (`gpt_edit`, 1 MiB alignment, overlap refusal, dual-table commit)
//! - Filesystem formatters and inspector (`format`, FAT32, exFAT, SexFS, mount verification)
//! - NVMe and ATA SMART health analyzer (`smart`, diagnostic findings, wear %, temperature)
//! - BDSM encrypted volume workflow (`crypt_flow`, Argon2id + XTS-AES-256 header, sector-translating I/O)

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod crypt_flow;
pub mod device;
pub mod error;
pub mod format;
pub mod gpt_edit;
pub mod layout;
pub mod smart;

pub use crypt_flow::{
    erase_encrypted_header, format_encrypted_volume, rewrap_passphrase, unlock_encrypted_volume,
    Cost, EncryptedBlockIo, CRYPT_HEADER_BYTES, MASTER_KEY_BYTES,
};
pub use device::{
    BlockIo, MemBlockDevice, PartitionIo, SexDiskAdapter, SexFsAdapter, SexVolumesAdapter,
    SECTOR_4096, SECTOR_512,
};
pub use error::{CorsetError, IoError, IoErrorKind};
pub use format::{
    format_exfat, format_fat32, format_sexfs, inspect_filesystem, ExFatFormatOptions,
    Fat32FormatOptions, FilesystemInspection, FilesystemType,
};
pub use gpt_edit::{CreatePartitionOptions, GptEditor};
pub use layout::{
    crc32, scan_disk_map, Crc32, DiskMap, DiskRegion, DiskRegionKind, FreeExtent, GptHeader,
    Guid, PartitionEntry, TYPE_BDSM_ENCRYPTED, TYPE_EFI_SYSTEM, TYPE_LINUX_FILESYSTEM,
    TYPE_LINUX_LUKS, TYPE_LINUX_LVM, TYPE_LINUX_SWAP, TYPE_MICROSOFT_BASIC_DATA,
    TYPE_SEXOS_BOOT, TYPE_SEXOS_DATA,
};
pub use smart::{
    AtaSmartAttribute, AtaSmartData, DeviceProtocol, FindingSeverity, HealthFinding, HealthReport,
    NvmeSmartLog, OverallHealth,
};
