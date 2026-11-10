// SPDX-License-Identifier: GPL-2.0-or-later OR MIT
//! The file protocol. Paths and data travel through a BOOT region a client shares with
//! Files alone; holding that region and a route to Files is the whole right to use it.

pub const OP: u64 = 0x54;
pub const CLIENT_SLOT: u64 = 258;
pub const PROVIDER_SLOT_BASE: u64 = 300;
pub const REGION_ID_BASE: u32 = 0x5000;
pub const REGION_BYTES: usize = 256 * 1024;
pub const PATH_BYTES: usize = 1024;
/// The region's last 4 KiB hold Files' answers to earlier STATs (`stat_cache`).
pub const CACHE_BYTES: usize = 4096;
pub const CACHE_AT: usize = REGION_BYTES - CACHE_BYTES;
pub const DATA_MAX: usize = CACHE_AT - PATH_BYTES;
/// The most a READ or WRITE should carry: whole 4 KiB blocks, so a long transfer never
/// ends a request in the middle of a block (which would cost Files a read-modify-write).
pub const IO_MAX: usize = DATA_MAX & !4095;
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
/// Installs SexOS beside whatever the disk holds (GPT, shared EFI System partition,
/// slots A and B and a store in free space). The path names a boot image file, or is
/// empty for the running system's. `offset` is the store size in MiB (0: all the free
/// space), `length` carries INSTALL_GO; without it Files only plans. Either way the
/// reply's value is the length of a report, lines of text in the data area.
/// Spindle only: it changes how the machine starts.
pub const INSTALL: u8 = 14;
pub const INSTALL_GO: u64 = 1;
/// The boot slot: after the program store, before the filesystem. MBR partition 1, type EFI.
pub const BOOT_LBA: u64 = 65536;
pub const BOOT_SECTORS: u64 = 65536;
/// The boot image the running system came with, read-only in Files: `install` with no
/// file writes this one.
pub const BOOT_IMAGE_SLOT: u64 = 101;
pub const BOOT_IMAGE_REGION_ID: u32 = 0x4100;
pub const BOOT_IMAGE_MODULE: &str = "boot.img";
/// Mounts the volumes a disk image holds (FAT, exFAT, ext4, NTFS) under `/media/NAME`.
/// The data area gets one line per volume: name, format, size. Spindle only.
pub const MOUNT: u8 = 15;
/// Writes out and forgets the volume at `/media/NAME`. Spindle only.
pub const UNMOUNT: u8 = 16;
/// The open/save picker, a portal: a program asks the person for a file instead of
/// being given the whole filesystem. PICK starts the question (data: mode byte, then a
/// suggested name `length` bytes long) and answers with a ticket. PICKED (`offset` the
/// ticket) answers PENDING until the person chose; then the data area holds the granted
/// path, `@/absolute/path`, which the run may use from then on, or DENIED if they
/// cancelled. Programs only.
pub const PICK: u8 = 17;
pub const PICKED: u8 = 18;
/// The questions waiting, for the program showing the picker: one that holds the whole
/// filesystem (`@/`), which the person gave it. Entries from DATA: row, mode, ticket
/// (u32 LE), name length, name, suggestion length, suggestion.
pub const PICKS: u8 = 19;
/// Answers one question: `offset` the row, `length` the ticket, the path `@/...` the
/// person chose (empty to cancel). The chosen path must be one the answering run holds.
pub const ANSWER: u8 = 20;
/// Answers OK at once and does nothing: what one call and its reply cost.
pub const PING: u8 = 21;
pub const PICK_OPEN: u8 = 0;
pub const PICK_SAVE: u8 = 1;
pub const PICK_FOLDER: u8 = 2;
pub const PICK_HINT_MAX: usize = 64;
/// Where other systems' volumes appear.
pub const MEDIA: &[u8] = b"/media";

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
/// A rename between two volumes: copy, then remove.
pub const OTHER_DISK: u8 = 13;
pub const READ_ONLY: u8 = 14;
/// A kind of volume, or a feature of one (compression, encryption), SexOS can't read yet.
pub const UNSUPPORTED: u8 = 15;
/// The person hasn't answered yet.
pub const PENDING: u8 = 16;

pub fn message(status: u8) -> &'static [u8] {
    match status {
        OK => b"ok", UNAVAILABLE => b"no filesystem on this disk", NOT_FOUND => b"not found", EXISTS => b"already exists",
        NOT_DIR => b"not a directory", IS_DIR => b"is a directory", NOT_EMPTY => b"directory not empty", NAME => b"bad name",
        NO_SPACE => b"disk full", IO => b"disk error", CORRUPT => b"damaged data found", DENIED => b"not allowed",
        OTHER_DISK => b"on another disk", READ_ONLY => b"read-only disk", UNSUPPORTED => b"not supported on this disk", PENDING => b"waiting for the person", _ => b"invalid request",
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

/// What Files said about paths this client asked about, kept where the client reads it
/// without a call: the region's last `CACHE_BYTES`. Files fills an entry when it answers
/// a STAT, and clears it, in every client's region, before any change to that file can
/// be seen; a rename, removal or mount clears them all. Only Files writes here.
/// Each entry is guarded by a sequence word (odd while Files writes it).
pub mod stat_cache {
    use core::sync::atomic::{fence, AtomicU64, Ordering};
    pub const ENTRIES: usize = super::CACHE_BYTES / ENTRY;
    const ENTRY: usize = 64;
    /// Words: 0 sequence, 1 key (the path as the client spelled it), 2 file (the path
    /// Files resolved it to), 3 size, 4 mtime, 5 directory, 6 file id.
    const WORDS: usize = 7;
    /// FNV-1a; 0 means an empty entry, so it never returns 0.
    pub fn hash(path: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in path { h ^= *b as u64; h = h.wrapping_mul(0x0100_0000_01b3); }
        h | 1
    }
    fn words(cache: &[u8], key: u64) -> &[AtomicU64; WORDS] {
        let at = (key as usize % ENTRIES) * ENTRY;
        // The region is page aligned, so every entry is 8-byte aligned.
        unsafe { &*(cache[at..at + ENTRY].as_ptr() as *const [AtomicU64; WORDS]) }
    }
    /// (directory, size, mtime, id) if Files answered for this spelling and nothing
    /// changed since.
    pub fn read(cache: &[u8], path: &[u8]) -> Option<(bool, u64, u64, u64)> {
        let key = hash(path);
        let w = words(cache, key);
        let seq = w[0].load(Ordering::Acquire);
        if seq & 1 != 0 || w[1].load(Ordering::Relaxed) != key { return None; }
        let (size, mtime, dir, id) = (w[3].load(Ordering::Relaxed), w[4].load(Ordering::Relaxed), w[5].load(Ordering::Relaxed), w[6].load(Ordering::Relaxed));
        fence(Ordering::Acquire);
        (w[0].load(Ordering::Relaxed) == seq && w[1].load(Ordering::Relaxed) == key).then_some((dir != 0, size, mtime, id))
    }
    fn write(w: &[AtomicU64; WORDS], values: [u64; WORDS - 1]) {
        let seq = w[0].load(Ordering::Relaxed);
        w[0].store(seq | 1, Ordering::Relaxed);
        fence(Ordering::Release);
        for (i, v) in values.iter().enumerate() { w[i + 1].store(*v, Ordering::Relaxed); }
        w[0].store((seq | 1) + 1, Ordering::Release);
    }
    /// Files only: what a STAT just answered.
    pub fn fill(cache: &[u8], path: &[u8], file: u64, dir: bool, size: u64, mtime: u64, id: u64) {
        let key = hash(path);
        write(words(cache, key), [key, file, size, mtime, dir as u64, id]);
    }
    /// Files only: forgets every entry for `file`, or every entry at all.
    pub fn forget(cache: &[u8], file: Option<u64>) -> usize {
        let mut cleared = 0;
        for i in 0..ENTRIES {
            let at = i * ENTRY;
            let w = unsafe { &*(cache[at..at + ENTRY].as_ptr() as *const [AtomicU64; WORDS]) };
            if w[1].load(Ordering::Relaxed) == 0 { continue; }
            if file.is_some_and(|f| w[2].load(Ordering::Relaxed) != f) { continue; }
            write(w, [0; WORDS - 1]);
            cleared += 1;
        }
        cleared
    }
}
