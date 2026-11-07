// SPDX-License-Identifier: GPL-3.0-only
//! Programs kept on disk, and the protocol that loads one into the image region.
//! A name selects data; it grants nothing. The program runs with exactly what its
//! BOOT catalog row holds, whatever the file says.

pub const OP: u64 = 0x4E;
/// First sector of the store: a directory, then program bytes.
pub const DIRECTORY_LBA: u64 = 2048;
pub const MAGIC: [u8; 8] = *b"SEXPROG1";
pub const SECTOR: usize = 512;
pub const ENTRY_BYTES: usize = 32;
/// Directory: 8 byte magic, 8 reserved, then entries of name[16], lba u32, len u32, hash u64.
pub const MAX_ENTRIES: usize = (SECTOR - 16) / ENTRY_BYTES;
pub const NAME_BYTES: usize = 16;
/// One program's largest file. Rows reserve the same span of frames.
pub const IMAGE_BYTES: usize = 1024 * 1024;
pub const IMAGE_SLOT_FILES: u64 = 100;
pub const IMAGE_REGION_ID: u32 = 0x4000;
/// Shell's route to Files, for this protocol alone.
pub const SHELL_ROUTE_SLOT: u64 = 256;
/// Spindle asks Shell to run a program by name.
pub const OP_RUN: u64 = 0x4F;

pub const REPLY_TAG: u64 = 0x50;
pub const NOT_FOUND: u32 = 1;
pub const TOO_LARGE: u32 = 2;
pub const IO: u32 = 3;
pub const CORRUPT: u32 = 4;
pub const fn loaded(len: u32) -> u64 { REPLY_TAG << 56 | 1 << 48 | len as u64 }
pub const fn failed(error: u32) -> u64 { REPLY_TAG << 56 | error as u64 }
pub fn decode_reply(value: u64) -> Option<Result<u32, u32>> {
    if value >> 56 != REPLY_TAG { return None; }
    if value >> 48 & 1 == 1 { Some(Ok(value as u32)) } else { Some(Err(value as u32)) }
}
pub fn pack_name(name: &[u8]) -> Option<[u64; 2]> {
    if name.is_empty() || name.len() > NAME_BYTES || !name.iter().all(|b| b.is_ascii_graphic()) { return None; }
    let mut bytes = [0u8; 16];
    bytes[..name.len()].copy_from_slice(name);
    Some([u64::from_le_bytes(bytes[..8].try_into().ok()?), u64::from_le_bytes(bytes[8..].try_into().ok()?)])
}
pub fn fnv(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes { hash = (hash ^ *byte as u64).wrapping_mul(0x100000001b3); }
    hash
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry { pub lba: u64, pub len: u32, pub hash: u64 }
/// The named entry of a directory sector, if the sector is one.
pub fn find(directory: &[u8], name: [u64; 2]) -> Option<Entry> {
    if directory.len() < SECTOR || directory[..8] != MAGIC { return None; }
    let mut want = [0u8; 16];
    want[..8].copy_from_slice(&name[0].to_le_bytes());
    want[8..].copy_from_slice(&name[1].to_le_bytes());
    for raw in directory[16..16 + MAX_ENTRIES * ENTRY_BYTES].chunks_exact(ENTRY_BYTES) {
        if raw[..16] != want { continue; }
        let word = |at: usize| u32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]);
        let hash = u64::from_le_bytes(raw[24..32].try_into().ok()?);
        return Some(Entry { lba: word(16) as u64, len: word(20), hash });
    }
    None
}
