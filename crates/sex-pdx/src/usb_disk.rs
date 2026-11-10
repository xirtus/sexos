// SPDX-License-Identifier: GPL-3.0-only
//! USB sticks and disks, from SexUSB to Files.
//!
//! BOOT gives Files a domain capability to SexUSB and SexUSB one to Files, both at
//! [`SLOT`], and one shared region at [`BUFFER_SLOT`] in each. Files asks; SexUSB moves
//! sectors through the region and answers with `pdx_reply`. When a disk comes or goes,
//! SexUSB sends Files [`OP_CHANGED`] (an enqueue, never a wait), and Files asks again.
//! Sectors are 512 bytes; disks whose own blocks are larger are translated by SexUSB.

/// Files: Domain(SexUSB). SexUSB: Domain(Files).
pub const SLOT: u64 = 1000;
/// The transfer region, in both.
pub const BUFFER_SLOT: u64 = 1001;
pub const REGION_ID: u32 = 0x4400;
pub const BUFFER_BYTES: u64 = 64 * 1024;
/// Sectors one READ or WRITE may move.
pub const MAX_SECTORS: u64 = BUFFER_BYTES / 512;
/// Disks at once; a disk's number is its bit in the [`OP_DISKS`] mask.
pub const MAX_DISKS: u64 = 8;

/// Reply: `(generation << 8) | mask` of the disks present now.
pub const OP_DISKS: u64 = 0x5D0;
/// arg0 disk. Reply: its size in sectors (0 if it is gone, `| READ_ONLY_FLAG` when the
/// medium is write-protected); the region holds its vendor and product, NUL-ended.
pub const OP_INFO: u64 = 0x5D1;
/// arg0 disk, arg1 first sector, arg2 sectors (1..=MAX_SECTORS). Reply: a status.
pub const OP_READ: u64 = 0x5D2;
pub const OP_WRITE: u64 = 0x5D3;
/// arg0 disk: write the disk's cache out. Reply: a status.
pub const OP_SYNC: u64 = 0x5D4;
/// SexUSB to Files: the set of disks changed. arg0 = the new generation.
pub const OP_CHANGED: u64 = 0x5D8;

pub const READ_ONLY_FLAG: u64 = 1 << 63;

pub const OK: u64 = 0;
pub const GONE: u64 = 1;
pub const IO: u64 = 2;
pub const INVALID: u64 = 3;
pub const READ_ONLY: u64 = 4;
