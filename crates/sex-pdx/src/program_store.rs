// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! Programs kept on disk, and the protocol that loads one into the image region.
//! A name selects data; it grants nothing. The program runs with exactly what its
//! BOOT catalog row holds, whatever the file says.

pub const OP: u64 = 0x4E;
/// First sector of the store: a directory, then program bytes.
pub const DIRECTORY_LBA: u64 = 2048;
pub const MAGIC: [u8; 8] = *b"SEXPROG1";
pub const SECTOR: usize = 512;
pub const ENTRY_BYTES: usize = 32;
/// Directory: 8 byte magic, a u32 count of directory sectors (0 means 1), 4 reserved,
/// then entries of name[16], lba u32, len u32, hash u64. Every further directory sector
/// keeps the same 16-byte head and entry places, so each holds `MAX_ENTRIES`.
pub const MAX_ENTRIES: usize = (SECTOR - 16) / ENTRY_BYTES;
/// Up to 64 directory sectors: 960 programs in the store.
pub const MAX_DIRECTORY_SECTORS: usize = 64;
pub const MAX_PROGRAMS: usize = MAX_ENTRIES * MAX_DIRECTORY_SECTORS;
/// How many sectors the directory spans, from its first sector.
pub fn directory_sectors(first: &[u8]) -> Option<usize> {
    if first.len() < SECTOR || first[..8] != MAGIC { return None; }
    let count = u32::from_le_bytes(first[8..12].try_into().ok()?) as usize;
    match count { 0 => Some(1), n if n <= MAX_DIRECTORY_SECTORS => Some(n), _ => None }
}
pub const NAME_BYTES: usize = 16;
/// One program's largest file.
pub const IMAGE_BYTES: usize = 32 * 1024 * 1024;
/// The most address space a program row's arena spans: one run's image first, then the
/// memory it asks for while it runs (`program_memory`). Frames are lent per page as the
/// run asks, from memory every program shares, and page tables are made as pages are first
/// used, so an unused arena costs nothing.
/// The kernel sizes each arena at BOOT to what the machine can hold, up to this.
pub const WINDOW_BYTES: usize = 256 * 1024 * 1024 * 1024;
const _: () = assert!(IMAGE_BYTES <= WINDOW_BYTES && WINDOW_BYTES % 4096 == 0);
pub const IMAGE_SLOT_FILES: u64 = 100;
pub const IMAGE_REGION_ID: u32 = 0x4000;
/// Shell's route to Files, for this protocol alone.
pub const SHELL_ROUTE_SLOT: u64 = 256;
/// Spindle asks Shell to run a program by name.
pub const OP_RUN: u64 = 0x4F;
/// Shell tells Spindle why a program did not start.
pub const OP_REFUSED: u64 = 0x53;
/// Shell to Files: the file Files last read now runs as this row and task. The program
/// may then use files inside its own folder, and nowhere else.
pub const OP_BIND: u64 = 0x55;
/// In OP_RUN's third word: the person asked for this run to reach the network.
pub const RUN_NETWORK: u64 = 1;
/// In OP_LAUNCH's flags: the run has no window on any workspace and no place in the bar,
/// for helpers such as thumbnailers (`sex_pdx::launch`).
pub const RUN_HIDDEN: u64 = 2;
/// Where each program's folder lives.
pub const DATA: &[u8] = b"/data/";
/// Where a bare program name is looked for first.
pub const BIN: &[u8] = b"/bin/";
/// A name word no packed name can have: the second word is a ticket from fs::PREPARE.
pub const TICKET: u64 = 0;

pub const REPLY_TAG: u64 = 0x50;
pub const NOT_FOUND: u32 = 1;
pub const TOO_LARGE: u32 = 2;
pub const IO: u32 = 3;
pub const CORRUPT: u32 = 4;
/// Refusals only Shell decides.
pub const NOT_A_PROGRAM: u32 = 5;
pub const NO_FREE_ROW: u32 = 6;
pub const NO_MORE_RECORDS: u32 = 7;
pub const IMAGE_BUSY:u32=8;
/// A program asked to hand on a grant it does not hold.
pub const NOT_ALLOWED: u32 = 9;
pub fn refusal(code: u64) -> &'static [u8] {
    match code as u32 {
        NOT_FOUND => b"No program by that name.",
        TOO_LARGE => b"That program is over the 16 MiB limit.",
        IO => b"The disk failed while reading the program.",
        CORRUPT => b"The program file is damaged.",
        NOT_A_PROGRAM => b"That file is not a program SexOS can run.",
        NO_FREE_ROW => b"All program windows are in use. Close one first.",
        NO_MORE_RECORDS => b"This machine has room for no more background programs.",
        IMAGE_BUSY=>b"Another program is still preparing. Try again.",
        NOT_ALLOWED => b"A program may pass on only what it holds itself.",
        _ => b"The program did not start.",
    }
}
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
/// The named entry of a directory (all its sectors), if it is one.
pub fn find(directory: &[u8], name: [u64; 2]) -> Option<Entry> {
    let sectors = directory_sectors(directory)?;
    if directory.len() < sectors * SECTOR { return None; }
    let mut want = [0u8; 16];
    want[..8].copy_from_slice(&name[0].to_le_bytes());
    want[8..].copy_from_slice(&name[1].to_le_bytes());
    let entries = directory[..sectors * SECTOR].chunks_exact(SECTOR)
        .flat_map(|sector| sector[16..16 + MAX_ENTRIES * ENTRY_BYTES].chunks_exact(ENTRY_BYTES));
    for raw in entries {
        if raw[..16] != want { continue; }
        let word = |at: usize| u32::from_le_bytes([raw[at], raw[at + 1], raw[at + 2], raw[at + 3]]);
        let hash = u64::from_le_bytes(raw[24..32].try_into().ok()?);
        return Some(Entry { lba: word(16) as u64, len: word(20), hash });
    }
    None
}

/// Program records: small background programs, made at BOOT from one template.
/// They hold no routes, so a record starts with only its own memory and time.
pub const RECORD_SLOT_SHELL: u64 = 257;
pub const SYSCALL_RECORDS: u64 = 79;
/// Spindle to Shell: name, name, how many.
pub const OP_SPAWN: u64 = 0x50;
/// Spindle to Shell: 0 asks records to finish, 1 ends them whatever they do.
pub const OP_RECORDS_STOP: u64 = 0x51;
pub const OP_RECORDS_CENSUS: u64 = 0x52;
pub const RECORD_START: u64 = 0;
pub const RECORD_STOP: u64 = 1;
pub const RECORD_CENSUS: u64 = 2;
/// The one operation a record itself may make.
pub const RECORD_STOP_REQUESTED: u64 = 3;
#[cfg(target_arch = "x86_64")]
pub fn record_call(op: u64, a: u64, b: u64) -> (u64, u64, u64) {
    let (status, first, second): (u64, u64, u64);
    unsafe {
        core::arch::asm!("syscall", inlateout("rax") SYSCALL_RECORDS => status,
            inlateout("rdi") op => _, inlateout("rsi") a => first, inlateout("rdx") b => second,
            out("rcx") _, out("r11") _, lateout("r10") _, lateout("r8") _);
    }
    (status, first, second)
}
