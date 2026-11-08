// SPDX-License-Identifier: GPL-3.0-only
//! The file protocol. Paths and data travel through a BOOT region a client shares with
//! Files alone; holding that region and a route to Files is the whole right to use it.

pub const OP: u64 = 0x54;
pub const CLIENT_SLOT: u64 = 258;
pub const PROVIDER_SLOT_BASE: u64 = 300;
pub const REGION_ID_BASE: u32 = 0x5000;
pub const REGION_BYTES: usize = 64 * 1024;
pub const PATH_BYTES: usize = 1024;
pub const DATA_MAX: usize = REGION_BYTES - PATH_BYTES;
/// Program rows follow the Spindles in the provider slots and regions.
pub const PROGRAM_INDEX_BASE: usize = crate::app_catalog::SPINDLE_TARGETS.len();
/// First filesystem sector: past the fixed DiskFS area and the program store.
pub const BASE_LBA: u64 = 131072;
/// A disk smaller than this has no filesystem.
pub const MIN_SECTORS: u64 = BASE_LBA + 262144;

pub const STAT: u8 = 1;
pub const LIST: u8 = 2;
pub const MKDIR: u8 = 3;
pub const CREATE: u8 = 4;
pub const REMOVE: u8 = 5;
pub const RENAME: u8 = 6;
pub const READ: u8 = 7;
pub const WRITE: u8 = 8;
pub const TRUNCATE: u8 = 9;
pub const SYNC: u8 = 10;
pub const STATFS: u8 = 11;
/// Checks a file can be a program and holds its path for one load. The reply's value is
/// the ticket Shell passes back to Files.
pub const PREPARE: u8 = 12;
/// A program's own arguments, as typed after its name. Programs only.
pub const ARGS: u8 = 13;
pub const ARGS_MAX: usize = 256;
/// Writes a boot partition image, a file, to the disk's boot slot and marks it bootable.
/// Spindle only: it replaces how the machine starts.
pub const INSTALL: u8 = 14;
/// The boot slot: after the program store, before the filesystem. MBR partition 1, type EFI.
pub const BOOT_LBA: u64 = 65536;
pub const BOOT_SECTORS: u64 = 65536;

pub const OK: u8 = 0;
pub const UNAVAILABLE: u8 = 1;
pub const NOT_FOUND: u8 = 2;
pub const EXISTS: u8 = 3;
pub const NOT_DIR: u8 = 4;
pub const IS_DIR: u8 = 5;
pub const NOT_EMPTY: u8 = 6;
pub const NAME: u8 = 7;
pub const NO_SPACE: u8 = 8;
pub const IO: u8 = 9;
pub const CORRUPT: u8 = 10;
pub const DENIED: u8 = 11;
pub const INVALID: u8 = 12;

pub fn message(status: u8) -> &'static [u8] {
    match status {
        OK => b"ok", UNAVAILABLE => b"no filesystem on this disk", NOT_FOUND => b"not found", EXISTS => b"already exists",
        NOT_DIR => b"not a directory", IS_DIR => b"is a directory", NOT_EMPTY => b"directory not empty", NAME => b"bad name",
        NO_SPACE => b"disk full", IO => b"disk error", CORRUPT => b"damaged data found", DENIED => b"not allowed", _ => b"invalid request",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request { pub seq: u32, pub kind: u8, pub path_len: u16, pub offset: u64, pub length: u64 }
impl Request {
    pub fn args(self) -> [u64; 3] {
        [self.seq as u64 & 0xFF_FFFF | (self.kind as u64) << 32 | (self.path_len as u64) << 40, self.offset, self.length]
    }
    pub fn decode(a: u64, offset: u64, length: u64) -> Option<Self> {
        let path_len = (a >> 40) as u16;
        if a >> 56 != 0 || path_len as usize > PATH_BYTES || a >> 24 & 0xFF != 0 { return None; }
        Some(Self { seq: (a & 0xFF_FFFF) as u32, kind: (a >> 32) as u8, path_len, offset, length })
    }
}
const TAG: u64 = 0xF5;
pub const fn reply(seq: u32, status: u8, value: u32) -> u64 {
    TAG << 56 | ((seq as u64) & 0xFF_FFFF) << 32 | (status as u64) << 24 | (value as u64 & 0xFF_FFFF)
}
pub fn decode_reply(word: u64) -> Option<(u32, u8, u32)> {
    if word >> 56 != TAG { return None; }
    Some((((word >> 32) & 0xFF_FFFF) as u32, (word >> 24) as u8, (word & 0xFF_FFFF) as u32))
}
/// LIST packs entries from DATA: 8-byte next cursor (0 when done), then for each
/// entry a kind byte (1 for a directory), a name length byte, an 8-byte size and the name.
pub const LIST_HEAD: usize = 8;
pub const ENTRY_HEAD: usize = 10;
